//! JSON over HTTPS. A client is built for each request and dropped with it, so no
//! connection pool, TLS session or buffer outlives the operation.

use std::time::Duration;

use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::error::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RESPONSE: u64 = 8 * 1024 * 1024;

pub struct Request<'a> {
    /// Service name for messages, e.g. "DeepL".
    pub service: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    /// The API key, used only to scrub it from any message shown to the user.
    pub secret: &'a str,
}

pub fn post_json(request: &Request, body: &Value) -> Result<Value> {
    let mut builder = agent().post(request.url);
    for (name, value) in request.headers {
        builder = builder.header(*name, *value);
    }
    let response = builder.send_json(body).map_err(|e| unreachable(request, &e))?;
    read_json(request, response)
}

/// Posts `application/x-www-form-urlencoded` fields.
pub fn post_form(request: &Request, fields: &[(&str, &str)]) -> Result<Value> {
    let mut builder = agent().post(request.url);
    for (name, value) in request.headers {
        builder = builder.header(*name, *value);
    }
    let response = builder.send_form(fields.iter().copied()).map_err(|e| unreachable(request, &e))?;
    read_json(request, response)
}

pub fn get_json(request: &Request) -> Result<Value> {
    let mut builder = agent().get(request.url);
    for (name, value) in request.headers {
        builder = builder.header(*name, *value);
    }
    let response = builder.call().map_err(|e| unreachable(request, &e))?;
    read_json(request, response)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("Glance/", env!("CARGO_PKG_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

fn unreachable(request: &Request, error: &ureq::Error) -> Error {
    Error::new(redact(&format!("Could not reach {}: {}", request.service, describe(error)), request.secret))
}

fn read_json(request: &Request, response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = response.status().as_u16();
    let text = response
        .into_body()
        .with_config()
        .limit(MAX_RESPONSE)
        .read_to_string()
        .map_err(|e| Error::new(redact(&format!("{} sent an unreadable response: {e}", request.service), request.secret)))?;

    if !(200..300).contains(&status) {
        return Err(status_error(request.service, status, &text, request.secret));
    }
    serde_json::from_str(&text)
        .map_err(|_| Error::new(format!("{} sent a response Glance could not read.", request.service)))
}

fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => "the request timed out".to_owned(),
        ureq::Error::HostNotFound => "the server address could not be resolved".to_owned(),
        ureq::Error::ConnectionFailed => "the connection failed".to_owned(),
        other => other.to_string(),
    }
}

fn status_error(service: &str, status: u16, body: &str, secret: &str) -> Error {
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| error_message(&v));
    let message = match status {
        401 | 403 => format!("{service} rejected the API key. Check it in Glance Settings."),
        402 => format!("Your {service} balance is too low for this request."),
        429 => format!("Too many requests to {service}. Wait a moment and try again."),
        _ => match detail {
            Some(detail) => format!("{service} returned an error ({status}): {detail}"),
            None => format!("{service} returned an error ({status})."),
        },
    };
    Error::new(redact(&message, secret))
}

/// Pulls a human-readable message out of the error shapes these services use.
pub fn error_message(value: &Value) -> Option<String> {
    let candidates = [
        value.pointer("/error/message"),
        value.pointer("/error"),
        value.pointer("/message"),
        value.pointer("/detail"),
        value.pointer("/0/error/message"),
    ];
    let message = candidates.into_iter().flatten().find_map(|v| v.as_str())?.trim();
    if message.is_empty() {
        return None;
    }
    let short: String = message.chars().take(300).collect();
    Some(if short.len() < message.len() { short + "…" } else { short })
}

/// Removes the key from text that may echo it back (some services quote it in errors).
pub fn redact(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.len() < 4 { text.to_owned() } else { text.replace(secret, "•••") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_the_key() {
        assert_eq!(redact("bad key sk-12345 given", "sk-12345"), "bad key ••• given");
        assert_eq!(redact("nothing here", ""), "nothing here");
    }

    #[test]
    fn reads_common_error_shapes() {
        assert_eq!(error_message(&json!({"error": {"message": "Bad model"}})).as_deref(), Some("Bad model"));
        assert_eq!(error_message(&json!({"error": "quota"})).as_deref(), Some("quota"));
        assert_eq!(error_message(&json!({"message": "Wrong target_lang"})).as_deref(), Some("Wrong target_lang"));
        assert_eq!(error_message(&json!([{"error": {"message": "x"}}])).as_deref(), Some("x"));
        assert_eq!(error_message(&json!({"ok": true})), None);
    }

    #[test]
    fn status_errors_never_include_the_key() {
        let body = r#"{"error":{"message":"Incorrect API key provided: gsk_secret_value"}}"#;
        let error = status_error("Groq", 400, body, "gsk_secret_value");
        assert!(!error.to_string().contains("gsk_secret_value"));
        let error = status_error("Groq", 401, body, "gsk_secret_value");
        assert!(error.to_string().contains("rejected the API key"));
    }
}
