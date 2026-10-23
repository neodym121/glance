//! Recognition with a vision model behind an OpenAI-compatible API. The selected
//! area is sent as a base64 PNG; this module is only used when the user picks a
//! cloud engine.

use serde_json::json;

use super::Recognizer;
use crate::error::Result;
use crate::imaging::{RgbImage, base64, fit_within};
use crate::llm;
use crate::providers::AiProvider;

const INSTRUCTION: &str = "You are an OCR engine. Transcribe all text in the image exactly as written, \
character for character, in its original language. Keep line breaks, punctuation and capitalization. \
Do not translate, correct, explain or comment. Output only the text, without quotes or Markdown. \
If there is no text, output nothing.";

/// Vision models downscale large images anyway; sending more only costs time.
const MAX_SIDE: u32 = 2048;

pub struct VisionOcr {
    pub provider: AiProvider,
    pub model: String,
    pub route: String,
    pub key: String,
}

impl Recognizer for VisionOcr {
    fn recognize(&self, image: &RgbImage) -> Result<String> {
        let (width, height) = fit_within(image.width, image.height, MAX_SIDE);
        let png = if (width, height) == (image.width, image.height) {
            image.encode_png()?
        } else {
            image.resized(width, height).encode_png()?
        };
        let data_url = format!("data:image/png;base64,{}", base64(&png));
        drop(png);
        // OpenAI-style `{ "url": … }`; Mistral documents the bare string form.
        let image_url = if self.provider == AiProvider::Mistral { json!(data_url) } else { json!({ "url": data_url }) };
        let messages = json!([
            { "role": "system", "content": INSTRUCTION },
            { "role": "user", "content": [
                { "type": "text", "text": "Transcribe the text in this image." },
                { "type": "image_url", "image_url": image_url }
            ]}
        ]);
        llm::chat(self.provider, &self.model, &self.route, &self.key, messages)
    }
}
