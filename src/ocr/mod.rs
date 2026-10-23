//! Text recognition behind one interface. The UI only asks for `spawn`; which engine
//! runs is decided from a snapshot of the settings.

mod paddle;
pub(crate) mod paddle_models;
mod vision;

use crate::app::{self, Action};
use crate::config::OcrConfig;
use crate::error::{Error, Result};
use crate::imaging::RgbImage;
use crate::providers::OcrProvider;
use crate::{secrets, strings};

pub trait Recognizer {
    fn recognize(&self, image: &RgbImage) -> Result<String>;
}

/// A line as an engine reports it, with the box around it in image pixels.
pub(super) struct OcrLine {
    pub text: String,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// An engine sometimes splits one visual line into pieces, and may list all left
/// pieces before the right ones. Pieces in the same row and close to each other are
/// rejoined left to right; text far apart (real columns) keeps the engine's order.
pub(super) fn join_lines(lines: &[OcrLine], separator: &str) -> String {
    fn same_row(a: &OcrLine, b: &OcrLine) -> bool {
        let overlap = a.bottom.min(b.bottom) - a.top.max(b.top);
        let smaller = (a.bottom - a.top).min(b.bottom - b.top).max(1.0);
        let taller = (a.bottom - a.top).max(b.bottom - b.top).max(1.0);
        let gap = (b.left - a.right).max(a.left - b.right);
        overlap >= smaller * 0.5 && gap <= taller * 4.0
    }
    let mut rows: Vec<Vec<&OcrLine>> = Vec::new();
    for line in lines {
        match rows.iter_mut().find(|row| row.iter().any(|piece| same_row(piece, line))) {
            Some(row) => row.push(line),
            None => rows.push(vec![line]),
        }
    }
    rows.into_iter()
        .map(|mut row| {
            row.sort_by(|a, b| a.left.total_cmp(&b.left));
            row.iter().map(|piece| piece.text.as_str()).collect::<Vec<_>>().join(separator)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Builds the engine for the current settings. Keys are read from Credential Manager
/// here, on the worker thread, and dropped with the engine.
fn recognizer_for(config: &OcrConfig) -> Result<Box<dyn Recognizer>> {
    match config.provider.ai() {
        Some(provider) => {
            let key = secrets::load(provider.secret())
                .ok_or_else(|| Error::new(strings::missing_key(provider.name())))?;
            let model = match config.models.get(provider).trim() {
                "" => provider.default_vision_model().to_owned(),
                custom => custom.to_owned(),
            };
            let route = config.routes.get(provider).trim().to_owned();
            Ok(Box::new(vision::VisionOcr { provider, model, route, key }))
        }
        None => Ok(Box::new(paddle::PaddleOcr::new(&config.paddle_script))),
    }
}

/// Footer caption describing the engine, e.g. "Paddle OCR · East Slavic" or "Groq · model-id".
pub fn engine_label(config: &OcrConfig) -> String {
    match config.provider.ai() {
        Some(provider) => {
            let model = match config.models.get(provider).trim() {
                "" => provider.default_vision_model(),
                custom => custom,
            };
            format!("{} · {model}", provider.name())
        }
        None => format!(
            "{} · {}",
            OcrProvider::Paddle.name(),
            paddle_models::resolve(&config.paddle_script).short
        ),
    }
}

/// Recognizes on a short-lived worker thread and posts `OcrDone` with the result.
/// The image is moved in and freed when the thread ends.
pub fn spawn(job: u64, image: RgbImage, config: OcrConfig) {
    let worker = std::thread::Builder::new().name("glance-ocr".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recognizer_for(&config)?.recognize(&image)
        }))
        .unwrap_or_else(|_| Err(Error::new("Text recognition failed unexpectedly.")));
        drop(image);
        app::post(Action::OcrDone { job, result });
    });
    if let Err(e) = worker {
        app::post(Action::OcrDone { job, result: Err(Error::new(format!("Could not start recognition: {e}"))) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(text: &str, left: f32, right: f32, top: f32) -> OcrLine {
        OcrLine { text: text.into(), left, right, top, bottom: top + 30.0 }
    }

    #[test]
    fn rejoins_pieces_of_one_visual_line() {
        let lines = [
            piece("SyncUp (https://syncup.tv/)", 0.0, 400.0, 10.0),
            piece("— веб-сервис для организации", 430.0, 900.0, 12.0),
            piece("просмотров видео", 0.0, 300.0, 50.0),
        ];
        assert_eq!(
            join_lines(&lines, " "),
            "SyncUp (https://syncup.tv/) — веб-сервис для организации\nпросмотров видео"
        );
        assert_eq!(join_lines(&[], " "), "");
    }

    #[test]
    fn reorders_left_and_right_pieces_by_row() {
        let lines = [
            piece("one", 0.0, 300.0, 0.0),
            piece("three", 0.0, 300.0, 40.0),
            piece("two", 320.0, 600.0, 2.0),
            piece("four", 320.0, 600.0, 41.0),
        ];
        assert_eq!(join_lines(&lines, " "), "one two\nthree four");
    }

    #[test]
    fn keeps_distant_columns_apart() {
        let lines = [
            piece("left 1", 0.0, 200.0, 0.0),
            piece("left 2", 0.0, 200.0, 40.0),
            piece("right 1", 900.0, 1100.0, 0.0),
            piece("right 2", 900.0, 1100.0, 40.0),
        ];
        assert_eq!(join_lines(&lines, " "), "left 1\nleft 2\nright 1\nright 2");
    }
}
