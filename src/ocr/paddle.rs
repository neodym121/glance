//! Local recognition with PaddleOCR's PP-OCRv5 mobile models through ONNX Runtime
//! (the oar-ocr crate). Nothing leaves the computer and no key is needed. The model
//! files are downloaded from Settings on first use and live in %APPDATA%\Glance
//! (see `paddle_models`); the engine is built for one recognition and dropped with
//! it, so no model stays in memory while Glance sits in the tray.
//!
//! There is no language picker: a crop is read by the recognizer that reads it
//! best. The one the Windows locale suggests goes first, and the text itself then
//! says whether another model has to be asked (see `choose`).

use std::collections::VecDeque;

use oar_ocr::oarocr::OAROCRBuilder;

use super::{OcrLine, Recognizer, join_lines, paddle_models};
use crate::error::{Error, Result};
use crate::imaging::RgbImage;
use crate::strings;
use paddle_models::Rec;

/// The detector scales its input down so the longest side is at most this many
/// pixels, so enlarging a crop beyond it would be undone before detection. Small
/// screen text is still recognized far more reliably enlarged (the same ratios the
/// Windows engine measured), so crops grow 3x or 2x only while the result stays
/// within the cap; anything larger is left for the engine's own scaling.
const DET_INPUT_SIDE: u32 = 960;

/// Mean confidence a result must reach to be taken as it is. A model that never saw
/// the script still answers, so the score alone is not enough (see `recognize`).
const CONFIDENT: f32 = 0.90;

pub struct PaddleOcr;

impl Recognizer for PaddleOcr {
    /// Reads the crop with the first recognizer that both scores well and outputs
    /// the script the text is actually in, trying the rest when neither holds. A
    /// model never sees a script it has no glyphs for: it answers with whatever its
    /// own alphabet offers (the Latin model returns punctuation for a Russian line,
    /// the Cyrillic one reads English almost perfectly), so the text itself decides
    /// which model is the right one.
    fn recognize(&self, image: &RgbImage) -> Result<String> {
        let candidates = paddle_models::candidates();
        if candidates.is_empty() {
            return Err(Error::new(strings::paddle_models_missing()));
        }
        let mut failure = None;
        let chosen = choose(candidates, |rec| {
            let files = paddle_models::paths(rec)?;
            match recognize_with(files, image) {
                Ok(read) => Some(read),
                Err(e) => {
                    failure = Some(e);
                    None
                }
            }
        });
        match chosen {
            Some((_, text)) => Ok(text),
            None => Err(failure.unwrap_or_else(|| Error::new(strings::paddle_models_missing()))),
        }
    }

}

/// The recognizer to believe, out of the candidates in their order. `read` returns
/// what one recognizer made of the crop, or `None` when it cannot run.
///
/// A model never sees a script it has no glyphs for: it answers with whatever its
/// own alphabet offers (the Latin model returns punctuation for a Russian line, the
/// Cyrillic one reads English almost perfectly). So a confident result in the
/// model's own script ends the search, a result in another script sends the search
/// to the model that reads that script, and the rest are ranked: own script beats
/// another, letters beat punctuation, then the score.
fn choose(
    candidates: Vec<&'static Rec>,
    mut read: impl FnMut(&'static Rec) -> Option<(String, f32)>,
) -> Option<(&'static Rec, String)> {
    let mut best: Option<(u8, f32, &'static Rec, String)> = None;
    let mut queue: VecDeque<&'static Rec> = candidates.into();
    let mut tried: Vec<&'static Rec> = Vec::new();
    while let Some(rec) = queue.pop_front() {
        if tried.iter().any(|seen| seen.script == rec.script) {
            continue;
        }
        tried.push(rec);
        let Some((text, score)) = read(rec) else { continue };
        let letters = text.chars().any(char::is_alphabetic);
        let script = paddle_models::script_of(&text);
        let matched = script.is_none_or(|script| script == rec.script);
        if letters && matched && score >= CONFIDENT {
            return Some((rec, text));
        }
        let rank = match (letters, matched) {
            (true, true) => 2,
            (true, false) => 1,
            (false, _) => 0,
        };
        let better = best.as_ref().is_none_or(|(best_rank, best_score, ..)| {
            rank > *best_rank || (rank == *best_rank && score > *best_score)
        });
        if better {
            best = Some((rank, score, rec, text));
        }
        // The text is in a script this model was not made for: ask the one that was.
        if letters && let Some(script) = script && script != rec.script {
            queue.push_front(script.rec());
        }
    }
    best.map(|(_, _, rec, text)| (rec, text))
}

/// One pass with one recognizer: the text and how sure the model is of it.
fn recognize_with(files: paddle_models::Files, image: &RgbImage) -> Result<(String, f32)> {
    let engine = OAROCRBuilder::new(files.det, files.rec, files.dict)
        .build()
        .map_err(|e| Error::new(strings::paddle_failed(&e.to_string())))?;
    let result = engine
        .predict(vec![prepared(image)])
        .map_err(|e| Error::new(strings::paddle_failed(&e.to_string())))?;
    let Some(result) = result.first() else { return Ok((String::new(), 0.0)) };
    let lines = lines_of(result);
    Ok((crate::links::repair_schemes(&join_lines(&lines, " ")), confidence_of(result)))
}

/// The model's mean confidence, counted per character so a long line outweighs a
/// stray one.
fn confidence_of(result: &oar_ocr::oarocr::OAROCRResult) -> f32 {
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for region in &result.text_regions {
        let Some(text) = region.text.as_ref() else { continue };
        let chars = text.chars().count().max(1) as f32;
        sum += region.confidence.unwrap_or(0.0) * chars;
        weight += chars;
    }
    if weight > 0.0 { sum / weight } else { 0.0 }
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
    use paddle_models::Script;
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

    /// A fake recognizer table: what each model makes of the crop.
    fn fake(read: &'static [(Script, &'static str, f32)]) -> impl FnMut(&'static Rec) -> Option<(String, f32)> {
        move |rec| {
            read.iter().find(|(script, ..)| *script == rec.script).map(|(_, text, score)| ((*text).to_owned(), *score))
        }
    }

    fn latin() -> &'static Rec {
        Script::Latin.rec()
    }

    fn cyrillic() -> &'static Rec {
        Script::Cyrillic.rec()
    }

    #[test]
    fn takes_a_confident_result_in_the_models_own_script_at_once() {
        let mut calls = 0;
        let chosen = choose(vec![latin(), cyrillic()], |rec| {
            calls += 1;
            (rec.script == Script::Latin).then(|| ("Hello, world".to_owned(), 0.97))
        })
        .unwrap();
        assert_eq!(chosen.0.script, Script::Latin);
        assert_eq!(calls, 1, "a Latin screen costs one recognition");
    }

    #[test]
    fn asks_the_model_of_the_script_the_text_is_in() {
        // The Cyrillic model reads an English line nearly perfectly, so the score
        // alone would keep it; the text says Latin and Latin is asked next.
        let read: &'static [(Script, &'static str, f32)] =
            &[(Script::Cyrillic, "Glance reads any text", 0.98), (Script::Latin, "Glance reads any text", 0.99)];
        let chosen = choose(vec![cyrillic(), latin()], fake(read)).unwrap();
        assert_eq!(chosen.0.script, Script::Latin);

        // The other way round: the Latin model answers a Russian line with the few
        // punctuation marks it recognizes.
        let read: &'static [(Script, &'static str, f32)] =
            &[(Script::Latin, " , . .", 0.95), (Script::Cyrillic, "Привет, мир", 0.99)];
        let chosen = choose(vec![latin(), cyrillic()], fake(read)).unwrap();
        assert_eq!(chosen.0.script, Script::Cyrillic);
        assert_eq!(chosen.1, "Привет, мир");
    }

    #[test]
    fn falls_back_to_the_best_of_what_the_models_could_read() {
        // Nothing is confident and nothing is in its own script: the score decides.
        let read: &'static [(Script, &'static str, f32)] = &[
            (Script::Latin, "Привет", 0.62),
            (Script::Cyrillic, "Привет", 0.88),
            (Script::Greek, "Приβет", 0.71),
        ];
        let chosen = choose(vec![latin(), cyrillic(), Script::Greek.rec()], fake(read)).unwrap();
        assert_eq!(chosen.0.script, Script::Cyrillic);
    }

    #[test]
    fn keeps_a_model_that_cannot_run_out_of_the_search() {
        let chosen =
            choose(vec![latin(), cyrillic()], |rec| (rec.script == Script::Cyrillic).then(|| ("Привет".to_owned(), 0.99)))
                .unwrap();
        assert_eq!(chosen.0.script, Script::Cyrillic);
    }
}
