//! Model lists of the AI providers, for the model pickers in Settings. A list is fetched
//! on a worker thread when a picker first needs it or on Refresh, and cached in
//! %APPDATA%\Glance\models.json so it shows at once the next time.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::{self, Action};
use crate::error::{Error, Result};
use crate::providers::AiProvider;
use crate::{config, http, secrets, strings};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    /// Display name; the id when the provider has none.
    pub name: String,
    /// Whether the model reads images; `None` when the provider doesn't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Recognition,
    Translation,
}

pub type Catalog = BTreeMap<AiProvider, Vec<ModelInfo>>;

const CACHE_FILE: &str = "models.json";

/// The cached lists; empty when there is no cache yet or it can't be read.
pub fn load_cache() -> Catalog {
    config::dir()
        .and_then(|dir| std::fs::read(dir.join(CACHE_FILE)).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_cache(catalog: &Catalog) -> std::io::Result<()> {
    let dir = config::dir().ok_or(std::io::ErrorKind::NotFound)?;
    config::write_atomic(&dir.join(CACHE_FILE), &serde_json::to_vec(catalog).expect("catalog serializes"))
}

/// Fetches on a short-lived worker thread and posts `ModelsLoaded`.
pub fn spawn(provider: AiProvider) {
    let worker = std::thread::Builder::new().name("glance-models".into()).spawn(move || {
        let result = std::panic::catch_unwind(|| fetch(provider))
            .unwrap_or_else(|_| Err(Error::new(strings::models_failed(provider.name()))));
        app::post(Action::ModelsLoaded { provider, result });
    });
    if let Err(e) = worker {
        app::post(Action::ModelsLoaded { provider, result: Err(Error::new(e.to_string())) });
    }
}

fn fetch(provider: AiProvider) -> Result<Vec<ModelInfo>> {
    let key = secrets::load(provider.secret());
    if key.is_none() && provider.models_need_key() {
        return Err(Error::new(strings::models_need_key(provider.name())));
    }
    let key = key.unwrap_or_default();
    let auth = format!("Bearer {key}");
    let mut headers = Vec::new();
    if !key.is_empty() {
        // Gemini's native API takes the key in its own header (never in the URL).
        headers.push(if provider == AiProvider::Gemini { ("X-Goog-Api-Key", key.as_str()) } else { ("Authorization", auth.as_str()) });
    }
    let response = http::get_json(&http::Request {
        service: provider.name(),
        url: provider.models_url(),
        headers: &headers,
        secret: &key,
    })?;
    Ok(parse(provider, &response))
}

/// Chat models from a provider's list response, sorted by name.
fn parse(provider: AiProvider, response: &Value) -> Vec<ModelInfo> {
    let text = |entry: &Value, field: &str| entry.get(field).and_then(Value::as_str).unwrap_or("").trim().to_owned();
    let entries = match provider {
        AiProvider::Gemini => response.get("models"),
        _ => response.get("data"),
    };
    let mut seen = HashSet::new();
    let mut models: Vec<ModelInfo> = entries
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let model = match provider {
                AiProvider::Gemini => gemini_model(entry, &text)?,
                AiProvider::Mistral => {
                    let capabilities = entry.get("capabilities");
                    let flag = |name| capabilities.and_then(|c| c.get(name)).and_then(Value::as_bool);
                    if flag("completion_chat") == Some(false) {
                        return None;
                    }
                    let id = text(entry, "id");
                    ModelInfo { name: id.clone(), id, vision: flag("vision") }
                }
                AiProvider::Groq => {
                    let id = text(entry, "id");
                    // Groq says nothing about modalities; leave out speech and guard models.
                    let skip = ["whisper", "tts", "guard"].iter().any(|s| id.to_ascii_lowercase().contains(s));
                    if skip || entry.get("active").and_then(Value::as_bool) == Some(false) {
                        return None;
                    }
                    ModelInfo { name: id.clone(), id, vision: None }
                }
                AiProvider::Polza => routed_model(entry, &text)?,
            };
            (!model.id.is_empty() && seen.insert(model.id.clone())).then_some(model)
        })
        .collect();
    models.sort_by_cached_key(|m| (m.name.to_lowercase(), m.id.clone()));
    models
}

/// Gemini lists every model; only those that generate content can chat.
fn gemini_model(entry: &Value, text: &impl Fn(&Value, &str) -> String) -> Option<ModelInfo> {
    let generates = entry
        .get("supportedGenerationMethods")
        .and_then(Value::as_array)
        .is_some_and(|methods| methods.iter().any(|m| m.as_str() == Some("generateContent")));
    if !generates {
        return None;
    }
    let full = text(entry, "name");
    let id = full.strip_prefix("models/").unwrap_or(&full).to_owned();
    let name = Some(text(entry, "displayName")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
    Some(ModelInfo { id, name, vision: None })
}

/// Polza AI describes modalities; it also lists image, speech and embedding
/// models, which are left out.
fn routed_model(entry: &Value, text: &impl Fn(&Value, &str) -> String) -> Option<ModelInfo> {
    if entry.get("type").and_then(Value::as_str).is_some_and(|kind| kind != "chat") {
        return None;
    }
    let architecture = entry.get("architecture");
    let modalities = |field| {
        architecture.and_then(|a| a.get(field)).and_then(Value::as_array).map(|list| {
            list.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()
        })
    };
    if modalities("output_modalities").is_some_and(|out| !out.iter().any(|m| m == "text")) {
        return None;
    }
    let id = text(entry, "id");
    let name = Some(text(entry, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
    let vision = modalities("input_modalities").map(|input| input.iter().any(|m| m == "image"));
    Some(ModelInfo { id, name, vision })
}

/// Models matching every word of `query` in their name or id. Recognition leaves out
/// models known not to read images.
pub fn search<'a>(models: &'a [ModelInfo], query: &str, purpose: Purpose) -> Vec<&'a ModelInfo> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    models
        .iter()
        .filter(|m| purpose == Purpose::Translation || m.vision != Some(false))
        .filter(|m| {
            let (name, id) = (m.name.to_lowercase(), m.id.to_lowercase());
            words.iter().all(|w| name.contains(w.as_str()) || id.contains(w.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_routed_lists() {
        let response = json!({ "data": [
            { "id": "z/vision", "name": "Zeta: Vision", "type": "chat",
              "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] } },
            { "id": "a/text", "name": "Alpha: Text",
              "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] } },
            { "id": "x/painter", "name": "Painter", "type": "image",
              "architecture": { "input_modalities": ["text"], "output_modalities": ["image"] } },
            { "id": "a/text", "name": "Duplicate" },
        ]});
        let models = parse(AiProvider::Polza, &response);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/text", "z/vision"]);
        assert_eq!(models[0].vision, Some(false));
        assert_eq!(models[1].vision, Some(true));
    }

    #[test]
    fn reads_gemini_mistral_and_groq_lists() {
        let gemini = json!({ "models": [
            { "name": "models/gemini-3.1-flash-lite", "displayName": "Gemini 3.1 Flash Lite",
              "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
        ]});
        let models = parse(AiProvider::Gemini, &gemini);
        assert_eq!(models.len(), 1);
        assert_eq!((models[0].id.as_str(), models[0].name.as_str()), ("gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite"));

        let mistral = json!({ "data": [
            { "id": "mistral-small-latest", "capabilities": { "completion_chat": true, "vision": true } },
            { "id": "mistral-embed", "capabilities": { "completion_chat": false } },
        ]});
        let models = parse(AiProvider::Mistral, &mistral);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].vision, Some(true));

        let groq = json!({ "data": [
            { "id": "llama-3.3-70b-versatile", "active": true },
            { "id": "whisper-large-v3", "active": true },
            { "id": "old-model", "active": false },
        ]});
        let models = parse(AiProvider::Groq, &groq);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["llama-3.3-70b-versatile"]);
    }

    #[test]
    fn searches_names_and_ids() {
        let models = vec![
            ModelInfo { id: "anthropic/claude-haiku-5.5".into(), name: "Anthropic: Claude Haiku 5.5".into(), vision: Some(true) },
            ModelInfo { id: "openai/gpt-oss-20b".into(), name: "GPT OSS 20B".into(), vision: Some(false) },
        ];
        let ids = |found: Vec<&ModelInfo>| found.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(search(&models, "claude 5.5", Purpose::Translation)), ["anthropic/claude-haiku-5.5"]);
        assert_eq!(ids(search(&models, "OSS-20", Purpose::Translation)), ["openai/gpt-oss-20b"]);
        assert_eq!(search(&models, "", Purpose::Translation).len(), 2);
        // Text-only models are not offered for recognition.
        assert_eq!(ids(search(&models, "", Purpose::Recognition)), ["anthropic/claude-haiku-5.5"]);
    }
}
