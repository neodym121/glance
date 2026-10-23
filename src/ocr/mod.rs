//! Text recognition behind one interface. The UI only asks for `spawn`; which engine
//! runs is decided from a snapshot of the settings.

mod vision;
mod windows_ocr;

pub use windows_ocr::available_languages;

use crate::app::{self, Action};
use crate::config::OcrConfig;
use crate::error::{Error, Result};
use crate::imaging::RgbImage;
use crate::providers::OcrProvider;
use crate::{secrets, strings};

pub trait Recognizer {
    fn recognize(&self, image: &RgbImage) -> Result<String>;
}

/// Builds the engine for the current settings. Keys are read from Credential Manager
/// here, on the worker thread, and dropped with the engine.
fn recognizer_for(config: &OcrConfig) -> Result<Box<dyn Recognizer>> {
    match config.provider.ai() {
        None => Ok(Box::new(windows_ocr::WindowsOcr::new(&config.language))),
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
    }
}

/// Footer caption describing the engine, e.g. "Windows OCR" or "Groq · model-id".
pub fn engine_label(config: &OcrConfig) -> String {
    match config.provider.ai() {
        None => OcrProvider::Windows.name().to_owned(),
        Some(provider) => {
            let model = match config.models.get(provider).trim() {
                "" => provider.default_vision_model(),
                custom => custom,
            };
            format!("{} · {model}", provider.name())
        }
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
