//! Translation of recognized text. Only the text is sent, never the image; the source
//! language is detected by the service.

use serde_json::{Value, json};

use crate::app::{self, Action};
use crate::config::TranslationConfig;
use crate::error::{Error, Result};
use crate::http;
use crate::languages::{self, Language};
use crate::llm;
use crate::providers::{AiProvider, TranslationProvider};
use crate::{secrets, strings};

pub trait Translator {
    fn translate(&self, text: &str, target: &Language) -> Result<String>;
}

struct ChatTranslator {
    provider: AiProvider,
    model: String,
    route: String,
    key: String,
}

impl Translator for ChatTranslator {
    fn translate(&self, text: &str, target: &Language) -> Result<String> {
        let instruction = format!(
            "Translate the user's text into {}. Detect the source language automatically. \
             Reply with the translation only: no explanations, notes, quotes or Markdown. \
             Keep line breaks, numbers, names and URLs as they are.",
            target.name
        );
        let messages = json!([
            { "role": "system", "content": instruction },
            { "role": "user", "content": text }
        ]);
        llm::chat(self.provider, &self.model, &self.route, &self.key, messages)
    }
}

/// Google Cloud Translation, Basic edition (v2).
struct GoogleTranslator {
    key: String,
}

impl Translator for GoogleTranslator {
    fn translate(&self, text: &str, target: &Language) -> Result<String> {
        // The key goes in a header, never in the URL.
        let headers = [("X-Goog-Api-Key", self.key.as_str())];
        let response = http::post_json(
            &http::Request {
                service: "Google Translate",
                url: "https://translation.googleapis.com/language/translate/v2",
                headers: &headers,
                secret: &self.key,
            },
            &json!({ "q": text, "target": target.google, "format": "text" }),
        )?;
        first_text(&response, "/data/translations/0/translatedText", "Google Translate")
    }
}

/// Google Translate without a key: the free endpoint behind Google's web translation
/// widgets (`client=gtx`). Unofficial, so it may be rate-limited or change.
struct GoogleFreeTranslator;

impl Translator for GoogleFreeTranslator {
    fn translate(&self, text: &str, target: &Language) -> Result<String> {
        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={}&dt=t&dj=1",
            target.google
        );
        // The text goes in the form body, so long selections don't hit URL limits.
        let response = http::post_form(
            &http::Request { service: "Google Translate", url: &url, headers: &[], secret: "" },
            &[("q", text)],
        )?;
        joined_sentences(&response)
    }
}

/// The free endpoint answers sentence by sentence; line breaks stay inside them.
fn joined_sentences(response: &Value) -> Result<String> {
    let text: String = response
        .get("sentences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sentence| sentence.get("trans").and_then(Value::as_str))
        .collect();
    match text.trim() {
        "" => Err(Error::new("Google Translate returned no translation.")),
        text => Ok(text.to_owned()),
    }
}

struct DeepLTranslator {
    key: String,
}

impl Translator for DeepLTranslator {
    fn translate(&self, text: &str, target: &Language) -> Result<String> {
        let target_lang = target.deepl.ok_or_else(|| Error::new(strings::deepl_unsupported(target.name)))?;
        // Keys of the free plan end in ":fx" and use a separate host.
        let url = if self.key.trim_end().ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate"
        } else {
            "https://api.deepl.com/v2/translate"
        };
        let auth = format!("DeepL-Auth-Key {}", self.key);
        let headers = [("Authorization", auth.as_str())];
        let response = http::post_json(
            &http::Request { service: "DeepL", url, headers: &headers, secret: &self.key },
            &json!({ "text": [text], "target_lang": target_lang }),
        )?;
        first_text(&response, "/translations/0/text", "DeepL")
    }
}

struct MicrosoftTranslator {
    key: String,
    region: String,
}

impl Translator for MicrosoftTranslator {
    fn translate(&self, text: &str, target: &Language) -> Result<String> {
        let url = format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to={}",
            target.microsoft
        );
        let mut headers = vec![("Ocp-Apim-Subscription-Key", self.key.as_str())];
        if !self.region.is_empty() {
            headers.push(("Ocp-Apim-Subscription-Region", self.region.as_str()));
        }
        let response = http::post_json(
            &http::Request { service: "Microsoft Translator", url: &url, headers: &headers, secret: &self.key },
            &json!([{ "Text": text }]),
        )?;
        first_text(&response, "/0/translations/0/text", "Microsoft Translator")
    }
}

fn first_text(response: &Value, pointer: &str, service: &str) -> Result<String> {
    response
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(|t| t.trim().to_owned())
        .ok_or_else(|| Error::new(format!("{service} returned no translation.")))
}

fn translator_for(config: &TranslationConfig) -> Result<Box<dyn Translator>> {
    let provider = config.provider;
    let key = secrets::load(provider.secret());
    if provider == TranslationProvider::Google && key.is_none() {
        return Ok(Box::new(GoogleFreeTranslator));
    }
    let key = key.ok_or_else(|| Error::new(strings::missing_key(provider.secret().name())))?;
    if let Some(ai) = provider.ai() {
        let model = match config.models.get(ai).trim() {
            "" => ai.default_text_model().to_owned(),
            custom => custom.to_owned(),
        };
        let route = config.routes.get(ai).trim().to_owned();
        return Ok(Box::new(ChatTranslator { provider: ai, model, route, key }));
    }
    Ok(match provider {
        TranslationProvider::Google => Box::new(GoogleTranslator { key }),
        TranslationProvider::DeepL => Box::new(DeepLTranslator { key }),
        _ => Box::new(MicrosoftTranslator { key, region: config.microsoft_region.trim().to_owned() }),
    })
}

/// Translates on a short-lived worker thread and posts `TranslationDone`.
pub fn spawn(job: u64, text: String, config: TranslationConfig) {
    let worker = std::thread::Builder::new().name("glance-translate".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let target = languages::find(&config.target).unwrap_or(&languages::LANGUAGES[0]);
            translator_for(&config)?.translate(&text, target)
        }))
        .unwrap_or_else(|_| Err(Error::new("Translation failed unexpectedly.")));
        app::post(Action::TranslationDone { job, result });
    });
    if let Err(e) = worker {
        app::post(Action::TranslationDone { job, result: Err(Error::new(format!("Could not start translation: {e}"))) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_translation_responses() {
        let google = json!({ "data": { "translations": [{ "translatedText": "Привет", "detectedSourceLanguage": "en" }] } });
        assert_eq!(first_text(&google, "/data/translations/0/translatedText", "G").unwrap(), "Привет");
        let deepl = json!({ "translations": [{ "detected_source_language": "EN", "text": "Hallo" }] });
        assert_eq!(first_text(&deepl, "/translations/0/text", "D").unwrap(), "Hallo");
        let microsoft = json!([{ "translations": [{ "text": "Bonjour", "to": "fr" }] }]);
        assert_eq!(first_text(&microsoft, "/0/translations/0/text", "M").unwrap(), "Bonjour");
        assert!(first_text(&json!({}), "/translations/0/text", "D").is_err());
    }

    #[test]
    fn joins_free_google_sentences() {
        let response = json!({ "sentences": [
            { "trans": "Привет, мир. ", "orig": "Hello world. " },
            { "trans": "Вторая строка.
", "orig": "Second line.
" },
            { "trans": "Третья.", "orig": "Third." },
            { "translit": "Privet" },
        ], "src": "en" });
        assert_eq!(joined_sentences(&response).unwrap(), "Привет, мир. Вторая строка.
Третья.");
        assert!(joined_sentences(&json!({ "sentences": [] })).is_err());
    }
}
