//! The cloud services Glance can talk to, and where their API keys live.
//!
//! The order of the `ALL` lists is the order of the pickers in Settings; the AI
//! providers come first in both pickers, in the same order (see ui/settings.slint).

use serde::{Deserialize, Serialize};

/// OpenAI-compatible chat services, used both for vision OCR and for translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    OpenRouter,
    Gemini,
    Mistral,
    Groq,
    Polza,
}

impl AiProvider {
    #[cfg(test)]
    pub const ALL: [Self; 5] = [Self::OpenRouter, Self::Gemini, Self::Mistral, Self::Groq, Self::Polza];

    pub fn name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::Gemini => "Google Gemini",
            Self::Mistral => "Mistral AI",
            Self::Groq => "Groq",
            Self::Polza => "Polza AI",
        }
    }

    pub fn chat_url(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
            // Gemini's OpenAI-compatible endpoint.
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            Self::Mistral => "https://api.mistral.ai/v1/chat/completions",
            Self::Groq => "https://api.groq.com/openai/v1/chat/completions",
            Self::Polza => "https://polza.ai/api/v1/chat/completions",
        }
    }

    /// The provider's model list. Gemini's native endpoint is used for its display names.
    pub fn models_url(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1/models",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000",
            Self::Mistral => "https://api.mistral.ai/v1/models",
            Self::Groq => "https://api.groq.com/openai/v1/models",
            Self::Polza => "https://polza.ai/api/v1/models",
        }
    }

    /// OpenRouter and Polza AI publish their model lists; the others need a key.
    pub fn models_need_key(self) -> bool {
        !matches!(self, Self::OpenRouter | Self::Polza)
    }

    /// Services that route a model to one of several upstream providers.
    pub fn routes(self) -> bool {
        matches!(self, Self::OpenRouter | Self::Polza)
    }

    pub fn default_vision_model(self) -> &'static str {
        match self {
            Self::OpenRouter | Self::Polza => "google/gemini-3.1-flash-lite",
            Self::Gemini => "gemini-3.1-flash-lite",
            Self::Mistral => "mistral-small-latest",
            Self::Groq => "meta-llama/llama-4-scout-17b-16e-instruct",
        }
    }

    pub fn default_text_model(self) -> &'static str {
        match self {
            Self::OpenRouter | Self::Polza => "google/gemini-3.1-flash-lite",
            Self::Gemini => "gemini-3.1-flash-lite",
            Self::Mistral => "mistral-small-latest",
            Self::Groq => "llama-3.3-70b-versatile",
        }
    }

    pub fn secret(self) -> Secret {
        match self {
            Self::OpenRouter => Secret::OpenRouter,
            Self::Gemini => Secret::Gemini,
            Self::Mistral => Secret::Mistral,
            Self::Groq => Secret::Groq,
            Self::Polza => Secret::Polza,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrProvider {
    #[default]
    OpenRouter,
    Gemini,
    Mistral,
    Groq,
    Polza,
    Windows,
}

impl OcrProvider {
    pub const ALL: [Self; 6] = [Self::OpenRouter, Self::Gemini, Self::Mistral, Self::Groq, Self::Polza, Self::Windows];

    pub fn ai(self) -> Option<AiProvider> {
        match self {
            Self::OpenRouter => Some(AiProvider::OpenRouter),
            Self::Gemini => Some(AiProvider::Gemini),
            Self::Mistral => Some(AiProvider::Mistral),
            Self::Groq => Some(AiProvider::Groq),
            Self::Polza => Some(AiProvider::Polza),
            Self::Windows => None,
        }
    }

    pub fn name(self) -> &'static str {
        self.ai().map_or("Windows OCR (on this PC)", AiProvider::name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslationProvider {
    #[default]
    OpenRouter,
    Gemini,
    Mistral,
    Groq,
    Polza,
    Google,
    DeepL,
    Microsoft,
}

impl TranslationProvider {
    pub const ALL: [Self; 8] = [
        Self::OpenRouter,
        Self::Gemini,
        Self::Mistral,
        Self::Groq,
        Self::Polza,
        Self::Google,
        Self::DeepL,
        Self::Microsoft,
    ];

    pub fn name(self) -> &'static str {
        match self.ai() {
            Some(ai) => ai.name(),
            None => match self {
                Self::Google => "Google Translate",
                Self::DeepL => "DeepL",
                _ => "Microsoft Translator",
            },
        }
    }

    pub fn ai(self) -> Option<AiProvider> {
        match self {
            Self::OpenRouter => Some(AiProvider::OpenRouter),
            Self::Gemini => Some(AiProvider::Gemini),
            Self::Mistral => Some(AiProvider::Mistral),
            Self::Groq => Some(AiProvider::Groq),
            Self::Polza => Some(AiProvider::Polza),
            Self::Google | Self::DeepL | Self::Microsoft => None,
        }
    }

    pub fn secret(self) -> Secret {
        match self {
            Self::Google => Secret::Google,
            Self::DeepL => Secret::DeepL,
            Self::Microsoft => Secret::Microsoft,
            ai => ai.ai().expect("AI translation provider").secret(),
        }
    }
}

/// An API key slot in Windows Credential Manager. AI keys are shared between
/// recognition and translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    OpenRouter,
    Gemini,
    Mistral,
    Groq,
    Polza,
    Google,
    DeepL,
    Microsoft,
}

impl Secret {
    pub const ALL: [Self; 8] = [
        Self::OpenRouter,
        Self::Gemini,
        Self::Mistral,
        Self::Groq,
        Self::Polza,
        Self::Google,
        Self::DeepL,
        Self::Microsoft,
    ];

    /// Credential Manager target name.
    pub fn target(self) -> &'static str {
        match self {
            Self::OpenRouter => "Glance:openrouter",
            Self::Gemini => "Glance:gemini",
            Self::Mistral => "Glance:mistral",
            Self::Groq => "Glance:groq",
            Self::Polza => "Glance:polza",
            Self::Google => "Glance:google",
            Self::DeepL => "Glance:deepl",
            Self::Microsoft => "Glance:microsoft",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::Gemini => "Google Gemini",
            Self::Mistral => "Mistral AI",
            Self::Groq => "Groq",
            Self::Polza => "Polza AI",
            Self::Google => "Google Cloud Translation",
            Self::DeepL => "DeepL",
            Self::Microsoft => "Microsoft Translator",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_round_trip_through_json() {
        for provider in TranslationProvider::ALL {
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(serde_json::from_str::<TranslationProvider>(&json).unwrap(), provider);
        }
        assert_eq!(serde_json::to_string(&OcrProvider::OpenRouter).unwrap(), "\"openrouter\"");
        assert_eq!(serde_json::to_string(&TranslationProvider::DeepL).unwrap(), "\"deepl\"");
    }

    #[test]
    fn credential_targets_are_unique() {
        let mut targets: Vec<_> = Secret::ALL.iter().map(|s| s.target()).collect();
        targets.sort();
        targets.dedup();
        assert_eq!(targets.len(), Secret::ALL.len());
    }

    #[test]
    fn pickers_list_ai_providers_first_in_the_same_order() {
        for (i, ai) in AiProvider::ALL.iter().enumerate() {
            assert_eq!(OcrProvider::ALL[i].ai(), Some(*ai));
            assert_eq!(TranslationProvider::ALL[i].ai(), Some(*ai));
        }
        assert_eq!(OcrProvider::default(), OcrProvider::OpenRouter);
        assert_eq!(TranslationProvider::default(), TranslationProvider::OpenRouter);
    }
}
