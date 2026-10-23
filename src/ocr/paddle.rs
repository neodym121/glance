//! Local recognition with PaddleOCR's PP-OCRv5 mobile models through ONNX Runtime
//! (the oar-ocr crate). Nothing leaves the computer and no key is needed. The model
//! files are downloaded from Settings on first use and live in %APPDATA%\Glance
//! (see `paddle_models`); the engine is built for one recognition and dropped with
//! it, so no model stays in memory while Glance sits in the tray.

use oar_ocr::oarocr::OAROCRBuilder;

use super::{OcrLine, Recognizer, join_lines, paddle_models};
use crate::error::{Error, Result};
use crate::imaging::RgbImage;
use crate::strings;

/// The detector scales its input down so the longest side is at most this many
/// pixels, so enlarging a crop beyond it would be undone before detection. Small
/// screen text is still recognized far more reliably enlarged (the same ratios the
/// Windows engine measured), so crops grow 3x or 2x only while the result stays
/// within the cap; anything larger is left for the engine's own scaling.
const DET_INPUT_SIDE: u32 = 960;

pub struct PaddleOcr {
    /// Script code from `paddle_models::SCRIPTS`; empty chooses from the locale.
    script: String,
}

impl PaddleOcr {
    pub fn new(script: &str) -> Self {
        Self { script: script.trim().to_owned() }
    }
}

impl Recognizer for PaddleOcr {
    fn recognize(&self, image: &RgbImage) -> Result<String> {
        let script = paddle_models::resolve(&self.script);
        let files = paddle_models::paths(script).ok_or_else(|| Error::new(strings::paddle_models_missing()))?;
        let engine = OAROCRBuilder::new(files.det, files.rec, files.dict)
            .build()
            .map_err(|e| Error::new(strings::paddle_failed(&e.to_string())))?;
        let result = engine
            .predict(vec![prepared(image)])
            .map_err(|e| Error::new(strings::paddle_failed(&e.to_string())))?;
        let lines = result.first().map(|r| lines_of(r)).unwrap_or_default();
        Ok(crate::links::repair_schemes(&join_lines(&lines, " ")))
    }
}

/// The engine's regions as lines with their boxes, in the order the engine sorted
/// them (reading order); `join_lines` then rejoins pieces of one visual line.
fn lines_of(result: &oar_ocr::oarocr::OAROCRResult) -> Vec<OcrLine> {
    result
        .text_regions
        .iter()
        .filter_map(|region| {
            let text = region.text.as_ref()?.to_string();
            let box_ = &region.bounding_box;
            Some(OcrLine {
                text,
                left: box_.x_min(),
                right: box_.x_max(),
                top: box_.y_min(),
                bottom: box_.y_max(),
            })
        })
        .collect()
}

/// The crop enlarged for the detector, converted to the image buffer oar-ocr
/// takes. Both layouts are tight RGB rows, so the bytes move as they are.
fn prepared(image: &RgbImage) -> image::RgbImage {
    let factor = enlargement(image.width, image.height);
    let enlarged = (factor > 1)
        .then(|| image.resized(image.width * factor, image.height * factor));
    let source = enlarged.as_ref().unwrap_or(image);
    image::RgbImage::from_raw(source.width, source.height, source.pixels.clone())
        .expect("the resampled buffer matches its size")
}

/// 3 or 2 for a crop worth enlarging, 1 when the detector would shrink it back.
fn enlargement(width: u32, height: u32) -> u32 {
    let longest = width.max(height);
    if longest.saturating_mul(3) <= DET_INPUT_SIDE {
        3
    } else if longest.saturating_mul(2) <= DET_INPUT_SIDE {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn region(text: Option<&str>, left: f32, top: f32, right: f32, bottom: f32) -> oar_ocr::oarocr::TextRegion {
        let bounding_box = oar_ocr::processors::BoundingBox::from_coords(left, top, right, bottom);
        oar_ocr::oarocr::TextRegion::with_recognition(
            bounding_box,
            text.map(|t| Arc::from(t)),
            Some(0.9),
        )
    }

    fn result(regions: Vec<oar_ocr::oarocr::TextRegion>) -> oar_ocr::oarocr::OAROCRResult {
        oar_ocr::oarocr::OAROCRResult {
            input_path: Arc::from(""),
            index: 0,
            input_img: Arc::new(image::RgbImage::new(1, 1)),
            text_regions: regions,
            orientation_angle: None,
            rectified_img: None,
        }
    }

    #[test]
    fn enlarges_small_crops_in_steps() {
        assert_eq!(enlargement(300, 40), 3);
        assert_eq!(enlargement(320, 120), 3, "3x of the long side still fits the detector input");
        assert_eq!(enlargement(400, 60), 2);
        assert_eq!(enlargement(480, 200), 2, "2x reaches the cap exactly");
        assert_eq!(enlargement(481, 60), 1, "the detector would shrink 2x back down");
        assert_eq!(enlargement(1920, 1080), 1);
    }

    #[test]
    fn prepared_keeps_pixels_and_dimensions() {
        let image = RgbImage { width: 4, height: 2, pixels: (0..24).collect() };
        let small = prepared(&image);
        assert_eq!((small.width(), small.height()), (12, 6));
        assert_eq!(small.as_raw().len(), 12 * 6 * 3);

        let large = RgbImage { width: 1000, height: 500, pixels: vec![7; 1000 * 500 * 3] };
        let same = prepared(&large);
        assert_eq!((same.width(), same.height()), (1000, 500), "not enlarged past the cap");
        assert!(same.as_raw().iter().all(|&v| v == 7));
    }

    #[test]
    fn maps_regions_to_lines_with_boxes() {
        let regions = result(vec![
            region(Some("Привет"), 0.0, 0.0, 90.0, 20.0),
            region(Some("мир"), 120.0, 2.0, 150.0, 18.0),
            region(None, 0.0, 40.0, 90.0, 60.0),
        ]);
        let lines = lines_of(&regions);
        assert_eq!(lines.len(), 2, "regions without text are dropped");
        assert_eq!(lines[0].text, "Привет");
        assert_eq!((lines[1].left, lines[1].right), (120.0, 150.0));
        // One visual row joins through join_lines, here with a space.
        assert_eq!(join_lines(&lines, " "), "Привет мир");
    }
}
