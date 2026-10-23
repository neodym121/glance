//! Settings stored in %APPDATA%\Glance\config.json. API keys are not part of it;
//! they live in Windows Credential Manager (see `secrets`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::providers::{AiProvider, OcrProvider, TranslationProvider};
use crate::{languages, win32};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+S";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub auto_copy: bool,
    pub ocr: OcrConfig,
    pub translation: TranslationConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_owned(),
            auto_copy: true,
            ocr: OcrConfig::default(),
            translation: TranslationConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub provider: OcrProvider,
    /// BCP-47 tag of the Windows OCR language; empty means the user profile languages.
    pub language: String,
    pub models: Models,
    pub routes: Routes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationConfig {
    pub provider: TranslationProvider,
    /// A code from `languages::LANGUAGES`.
    pub target: String,
    /// Translate right after recognition instead of waiting for the button.
    pub auto: bool,
    pub models: Models,
    pub routes: Routes,
    /// Azure region of a regional Microsoft Translator resource; empty for global.
    pub microsoft_region: String,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            provider: TranslationProvider::default(),
            target: languages::target_for_locale(&win32::user_locale()).to_owned(),
            auto: false,
            models: Models::default(),
            routes: Routes::default(),
            microsoft_region: String::new(),
        }
    }
}

/// Model ids per AI provider; empty means the provider default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Models {
    pub openrouter: String,
    pub gemini: String,
    pub mistral: String,
    pub groq: String,
    pub polza: String,
}

impl Models {
    pub fn get(&self, provider: AiProvider) -> &str {
        match provider {
            AiProvider::OpenRouter => &self.openrouter,
            AiProvider::Gemini => &self.gemini,
            AiProvider::Mistral => &self.mistral,
            AiProvider::Groq => &self.groq,
            AiProvider::Polza => &self.polza,
        }
    }
}

/// Upstream providers to send requests to, for the services that route between many
/// (OpenRouter, Polza AI): a provider ID or a comma-separated list; empty lets the
/// service choose.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routes {
    pub openrouter: String,
    pub polza: String,
}

impl Routes {
    pub fn get(&self, provider: AiProvider) -> &str {
        match provider {
            AiProvider::OpenRouter => &self.openrouter,
            AiProvider::Polza => &self.polza,
            _ => "",
        }
    }
}

/// %APPDATA%\Glance, where the config and the model list cache live.
pub fn dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Glance"))
}

fn path() -> Option<PathBuf> {
    Some(dir()?.join("config.json"))
}

/// Writes next to the target and renames, so a crash never leaves half a file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("file path has a parent"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// False on the first launch, before settings were ever saved.
pub fn exists() -> bool {
    path().is_some_and(|p| p.exists())
}

/// Reads the config; a missing or unreadable file yields the defaults.
pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> Result<()> {
    let path = path().ok_or_else(|| Error::new("The APPDATA folder is not available."))?;
    let json = serde_json::to_vec_pretty(config).expect("config serializes");
    write_atomic(&path, &json).map_err(|e| Error::new(format!("Could not save settings to {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let config: Config = serde_json::from_str(r#"{ "auto_copy": false }"#).unwrap();
        assert!(!config.auto_copy);
        assert_eq!(config.hotkey, DEFAULT_HOTKEY);
        assert_eq!(config.ocr.provider, OcrProvider::OpenRouter);
        assert!(!config.translation.auto);
    }

    #[test]
    fn round_trips_through_json() {
        let mut config = Config::default();
        config.ocr.provider = OcrProvider::Groq;
        config.ocr.models.groq = "some/model".into();
        config.ocr.routes.polza = "Amazon Bedrock".into();
        config.translation.provider = TranslationProvider::DeepL;
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }

    #[test]
    fn has_no_field_for_api_keys() {
        fn field_names(value: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(object) = value.as_object() {
                for (name, child) in object {
                    out.push(name.clone());
                    field_names(child, out);
                }
            }
        }
        let mut names = Vec::new();
        field_names(&serde_json::to_value(Config::default()).unwrap(), &mut names);
        assert!(names.iter().all(|n| n == "hotkey" || !(n.contains("key") || n.contains("secret") || n.contains("token"))));
    }
}
