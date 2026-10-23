//! OpenAI-compatible chat completions (OpenRouter, Gemini, Mistral, Groq, Polza AI).

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::http;
use crate::providers::AiProvider;

/// `route` names the upstream providers OpenRouter or Polza AI may use for the model
/// (`Routes` in the config); empty lets the service choose. Other providers ignore it.
pub fn chat(provider: AiProvider, model: &str, route: &str, key: &str, messages: Value) -> Result<String> {
    let auth = format!("Bearer {key}");
    let mut headers = vec![("Authorization", auth.as_str())];
    if provider == AiProvider::OpenRouter {
        // Optional attribution headers recommended by OpenRouter.
        headers.push(("X-Title", "Glance"));
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "temperature": 0,
        "stream": false,
    });
    let only = route_list(route);
    if provider.routes() && !only.is_empty() {
        // Both services take an allow-list of upstream providers in `provider.only`.
        body["provider"] = json!({ "only": only });
    }
    let response = http::post_json(
        &http::Request { service: provider.name(), url: provider.chat_url(), headers: &headers, secret: key },
        &body,
    )?;
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| match http::error_message(&response) {
            Some(detail) => Error::new(http::redact(&format!("{}: {detail}", provider.name()), key)),
            None => Error::new(format!("{} returned no text.", provider.name())),
        })?;
    Ok(clean_reply(content))
}

/// "google-vertex, anthropic" → ["google-vertex", "anthropic"].
fn route_list(route: &str) -> Vec<&str> {
    route.split(',').map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Strips what models add despite instructions: reasoning blocks and a Markdown fence
/// wrapped around the whole answer.
pub fn clean_reply(reply: &str) -> String {
    let mut text = reply.trim();
    if let Some(rest) = text.strip_prefix("<think>") {
        text = rest.split_once("</think>").map_or("", |(_, after)| after).trim();
    }
    if let Some(inner) = text.strip_prefix("```").and_then(|t| t.strip_suffix("```")) {
        // Drop an optional language tag on the opening fence line.
        text = match inner.split_once('\n') {
            Some((tag, body)) if !tag.contains(' ') => body,
            _ => inner,
        }
        .trim();
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_model_replies() {
        assert_eq!(clean_reply("  Hello\nworld \n"), "Hello\nworld");
        assert_eq!(clean_reply("```\nline 1\nline 2\n```"), "line 1\nline 2");
        assert_eq!(clean_reply("```text\nfn main() {}\n```"), "fn main() {}");
        assert_eq!(clean_reply("<think>hmm</think>\nПривет"), "Привет");
        assert_eq!(clean_reply("a ``` b"), "a ``` b");
    }

    #[test]
    fn splits_provider_routes() {
        assert_eq!(route_list(" google-vertex/global "), ["google-vertex/global"]);
        assert_eq!(route_list("Amazon Bedrock, anthropic,"), ["Amazon Bedrock", "anthropic"]);
        assert!(route_list("  ").is_empty());
    }
}
