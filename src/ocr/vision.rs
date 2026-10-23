//! Recognition with a vision model behind an OpenAI-compatible API. The selected
//! area is sent as a base64 PNG; this module is only used when the user picks a
//! cloud engine.

use std::borrow::Cow;

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

/// Crops with a short side under this are enlarged before encoding: small screen
/// text is exactly what vision models read worst.
const MIN_SHORT_SIDE: u32 = 768;

pub struct VisionOcr {
    pub provider: AiProvider,
    pub model: String,
    pub route: String,
    pub key: String,
}

impl Recognizer for VisionOcr {
    fn recognize(&self, image: &RgbImage) -> Result<String> {
        let png = prepared(image).encode_png()?;
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

/// The image sized for the model, still RGB and PNG-encoded without loss. Small
/// crops go up 3x, or 2x once 3x would pass MAX_SIDE — the same ratios measured
/// best for the Windows engine; anything larger only shrinks to MAX_SIDE.
/// Upscaling runs through the cubic resampler, so glyphs stay sharp.
fn prepared(image: &RgbImage) -> Cow<'_, RgbImage> {
    match enlargement(image.width, image.height) {
        0 => {
            let (width, height) = fit_within(image.width, image.height, MAX_SIDE);
            if (width, height) == (image.width, image.height) {
                Cow::Borrowed(image)
            } else {
                Cow::Owned(image.resized(width, height))
            }
        }
        factor => Cow::Owned(image.resized(image.width * factor, image.height * factor)),
    }
}

/// 3 or 2 for a crop worth enlarging, 0 when it should only be capped. The factor
/// never pushes the long side past MAX_SIDE.
fn enlargement(width: u32, height: u32) -> u32 {
    if width.min(height) >= MIN_SHORT_SIDE {
        return 0;
    }
    let longest = width.max(height);
    if longest * 3 <= MAX_SIDE { 3 } else if longest * 2 <= MAX_SIDE { 2 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enlarges_small_crops_in_steps() {
        assert_eq!(enlargement(300, 40), 3);
        assert_eq!(enlargement(700, 700), 2, "3x would pass the 2048 cap");
        assert_eq!(enlargement(1000, 100), 2, "3x of the long side does not fit");
        assert_eq!(enlargement(1200, 800), 0, "large enough as is");
        assert_eq!(enlargement(2048, 100), 0, "even 2x would not fit");
    }

    #[test]
    fn sends_small_crops_enlarged_and_large_ones_untouched() {
        let small = RgbImage { width: 300, height: 40, pixels: vec![0; 300 * 40 * 3] };
        let up = prepared(&small);
        assert_eq!((up.width, up.height), (900, 120));

        let enough = RgbImage { width: 768, height: 800, pixels: vec![0; 768 * 800 * 3] };
        assert!(matches!(prepared(&enough), Cow::Borrowed(_)));
    }
}
