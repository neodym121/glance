//! Local recognition with Windows.Media.Ocr (Windows 10 and later, unpackaged apps
//! included). Nothing leaves the computer.
//!
//! WinRT initialization: worker threads don't call `RoInitialize`; they join the
//! process-wide multithreaded apartment implicitly. `CoIncrementMTAUsage` keeps that
//! apartment alive for the whole process, so activation factories cached by
//! `windows-rs` stay valid from one recognition to the next and blocking on the
//! async operation is allowed (it never runs on the UI thread).

use std::sync::Once;

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::System::UserProfile::GlobalizationPreferences;
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::core::HSTRING;

use super::Recognizer;
use crate::error::{Error, Result};
use crate::imaging::{RgbImage, resample};
use crate::strings;

/// Upper bound for the enlarged image, in pixels (one byte each in Gray8). The engine
/// briefly holds two more copies, so this caps recognition at roughly 27 MB.
const MAX_ENLARGED_PIXELS: u64 = 9_000_000;

fn ensure_mta() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        // The cookie is intentionally never released: the apartment lives as long as the process.
        let _ = CoIncrementMTAUsage();
    });
}

pub struct WindowsOcr {
    /// BCP-47 tag, or empty to choose from the user profile languages.
    language: String,
}

impl WindowsOcr {
    pub fn new(language: &str) -> Self {
        Self { language: language.trim().to_owned() }
    }

    fn engine(&self) -> Result<OcrEngine> {
        let no_languages = || Error::new(strings::ocr_no_languages(&strings::ocr_capability_locale()));
        let tag = if self.language.is_empty() {
            let installed: Vec<String> = OcrEngine::AvailableRecognizerLanguages()
                .map(|list| list.into_iter().filter_map(|l| l.LanguageTag().ok()).map(|t| t.to_string()).collect())
                .unwrap_or_default();
            let profile: Vec<String> = GlobalizationPreferences::Languages()
                .map(|list| list.into_iter().map(|t| t.to_string()).collect())
                .unwrap_or_default();
            automatic_language(&profile, &installed).ok_or_else(no_languages)?
        } else {
            self.language.clone()
        };
        let missing = || Error::new(strings::ocr_language_missing(&tag, &tag));
        let language = Language::CreateLanguage(&HSTRING::from(&tag)).map_err(|_| missing())?;
        if !OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
            let name = language.DisplayName().map(|n| n.to_string()).unwrap_or_else(|_| tag.clone());
            return Err(Error::new(strings::ocr_language_missing(&name, &tag)));
        }
        OcrEngine::TryCreateFromLanguage(&language).map_err(|_| missing())
    }
}

impl Recognizer for WindowsOcr {
    fn recognize(&self, image: &RgbImage) -> Result<String> {
        ensure_mta();
        let engine = self.engine()?;
        let max = OcrEngine::MaxImageDimension().unwrap_or(2600);

        let (width, height, gray) = prepare(image, max);
        let buffer = CryptographicBuffer::CreateFromByteArray(&gray)?;
        drop(gray);
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, width as i32, height as i32)?;
        drop(buffer);

        // Gray8 keeps memory low; fall back to BGRA should an engine version reject it.
        let result = match engine.RecognizeAsync(&bitmap).and_then(|op| op.join()) {
            Ok(result) => result,
            Err(_) => {
                let bgra = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8)?;
                let result = engine.RecognizeAsync(&bgra)?.join()?;
                let _ = bgra.Close();
                result
            }
        };
        let _ = bitmap.Close();

        // Chinese and Japanese are written without spaces between words.
        let tag = engine.RecognizerLanguage()?.LanguageTag()?.to_string().to_ascii_lowercase();
        let separator = if tag.starts_with("zh") || tag.starts_with("ja") { "" } else { " " };
        let mut lines = Vec::new();
        for line in result.Lines()? {
            let mut words = Vec::new();
            let mut piece = OcrLine { text: String::new(), left: f32::MAX, right: f32::MIN, top: f32::MAX, bottom: f32::MIN };
            for word in line.Words()? {
                let rect = word.BoundingRect()?;
                piece.left = piece.left.min(rect.X);
                piece.right = piece.right.max(rect.X + rect.Width);
                piece.top = piece.top.min(rect.Y);
                piece.bottom = piece.bottom.max(rect.Y + rect.Height);
                words.push(word.Text()?.to_string());
            }
            if !words.is_empty() {
                piece.text = words.join(separator);
                lines.push(piece);
            }
        }
        Ok(crate::links::repair_schemes(&join_lines(&lines, separator)))
    }
}

/// A line as the engine reports it, with the box around its words.
struct OcrLine {
    text: String,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

/// The engine sometimes splits one visual line into pieces, and may list all left
/// pieces before the right ones. Pieces in the same row and close to each other are
/// rejoined left to right; text far apart (real columns) keeps the engine's order.
fn join_lines(lines: &[OcrLine], separator: &str) -> String {
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

/// Picks the recognizer for "Automatic". One engine reads one language, and engines
/// for Cyrillic, Greek or East Asian scripts also read Latin text reasonably, while a
/// Latin-script engine can't read the others at all. So a profile language with such
/// a script wins; otherwise the first profile language with OCR, then any installed one.
fn automatic_language(profile: &[String], installed: &[String]) -> Option<String> {
    fn primary(tag: &str) -> String {
        tag.split('-').next().unwrap_or("").to_ascii_lowercase()
    }
    fn non_latin(tag: &str) -> bool {
        const NON_LATIN: &[&str] = &["ar", "be", "bg", "el", "fa", "he", "hi", "ja", "kk", "ko", "mk", "ru", "sr", "th", "uk", "zh"];
        NON_LATIN.contains(&primary(tag).as_str()) && !tag.to_ascii_lowercase().contains("latn")
    }
    let available: Vec<&String> = profile
        .iter()
        .filter_map(|p| installed.iter().find(|i| primary(i) == primary(p)))
        .collect();
    available
        .iter()
        .find(|tag| non_latin(tag))
        .or(available.first())
        .map(|tag| tag.to_string())
        .or_else(|| installed.first().cloned())
}

/// Grayscale copy sized for the engine. Small screen text is recognized far more
/// reliably when enlarged: measured on real captures, 3x beat 2x clearly and 4x
/// lost small text entirely, so crops are enlarged 3x when the result stays within
/// the engine limit and MAX_ENLARGED_PIXELS, else 2x; anything beyond the engine
/// limit is scaled down. Gray8 keeps the buffer a third of the RGB size.
fn prepare(image: &RgbImage, max_dimension: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (image.width, image.height);
    let longest = w.max(h);
    let fits = |f: u32| longest * f <= max_dimension && w as u64 * h as u64 * (f * f) as u64 <= MAX_ENLARGED_PIXELS;
    let factor = if fits(3) {
        3.0
    } else if fits(2) {
        2.0
    } else if longest > max_dimension {
        max_dimension as f64 / longest as f64
    } else {
        1.0
    };
    let nw = ((w as f64 * factor).round() as u32).clamp(1, max_dimension);
    let nh = ((h as f64 * factor).round() as u32).clamp(1, max_dimension);

    let gray: Vec<u8> = image
        .pixels
        .as_chunks::<3>()
        .0
        .iter()
        .map(|[r, g, b]| ((*r as u32 * 299 + *g as u32 * 587 + *b as u32 * 114) / 1000) as u8)
        .collect();
    if (nw, nh) == (w, h) {
        return (w, h, gray);
    }
    let scaled = resample(&gray, w, h, 1, nw, nh);
    (nw, nh, scaled)
}

/// Installed OCR languages as (BCP-47 tag, display name). Queried on a throwaway
/// thread so the UI thread's apartment is never involved.
pub fn available_languages() -> Vec<(String, String)> {
    std::thread::spawn(|| {
        ensure_mta();
        let Ok(languages) = OcrEngine::AvailableRecognizerLanguages() else { return Vec::new() };
        languages
            .into_iter()
            .filter_map(|l| Some((l.LanguageTag().ok()?.to_string(), l.DisplayName().ok()?.to_string())))
            .collect()
    })
    .join()
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(width: u32, height: u32) -> RgbImage {
        RgbImage { width, height, pixels: vec![255; (width * height * 3) as usize] }
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn enlarges_small_crops_and_limits_large_ones() {
        assert_eq!(prepare(&blank(300, 40), 10000).0, 900);
        assert_eq!(prepare(&blank(800, 400), 10000).0, 2400, "multi-line selections get 3x too");
        assert_eq!(prepare(&blank(1400, 1000), 10000).0, 2800, "2x once 3x exceeds the pixel budget");
        assert_eq!(prepare(&blank(800, 400), 2000).0, 1600, "2x when 3x exceeds the engine limit");
        let (w, h, gray) = prepare(&blank(5000, 1000), 2600);
        assert_eq!((w, h), (2600, 520));
        assert_eq!(gray.len(), (w * h) as usize);
        assert_eq!(prepare(&blank(2500, 2000), 10000).0, 2500);
    }

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

    #[test]
    fn automatic_prefers_a_non_latin_profile_language() {
        assert_eq!(automatic_language(&tags(&["en-US", "ru"]), &tags(&["en-US", "ru"])).as_deref(), Some("ru"));
        assert_eq!(automatic_language(&tags(&["en-US", "ru"]), &tags(&["ru"])).as_deref(), Some("ru"));
        assert_eq!(automatic_language(&tags(&["en-US", "de-DE"]), &tags(&["de-DE", "en-US"])).as_deref(), Some("en-US"));
        assert_eq!(automatic_language(&tags(&["fr-FR"]), &tags(&["de-DE"])).as_deref(), Some("de-DE"));
        assert_eq!(automatic_language(&tags(&["sr-Latn-RS"]), &tags(&["sr-Latn", "en-US"])).as_deref(), Some("sr-Latn"));
        assert_eq!(automatic_language(&tags(&["en-US"]), &[]), None);
    }
}
