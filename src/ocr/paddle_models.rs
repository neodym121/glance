//! The model files of the local Paddle OCR engine: where they live, what Settings
//! downloads, and which recognizer to try on a crop. The detector (where the text
//! is) is shared; each script needs its own recognizer (what the characters look
//! like) plus a dictionary of the characters it may output. Latin and Cyrillic come
//! with the app, every other script is one optional pack, all of it fetched from
//! the oar-ocr GitHub release into %APPDATA%\Glance\models. Names, sizes and hashes
//! mirror the crate's own model registry (oar-ocr-core `download::registry`).

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::app::{self, Action};
use crate::config;
use crate::error::{Error, Result};
use crate::strings;

/// One file of a pipeline, as published in the release.
pub struct ModelFile {
    pub name: &'static str,
    sha256: &'static str,
    pub size: u64,
}

/// A recognizer and the dictionary of the characters it can output.
pub struct Rec {
    /// The script this recognizer is meant for.
    pub script: Script,
    rec: ModelFile,
    dict: ModelFile,
}

impl Rec {
    fn files(&self) -> [&ModelFile; 2] {
        [&self.rec, &self.dict]
    }
}

/// The writing system a text is mostly made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Korean,
    Han,
}

impl Script {
    /// The recognizer that reads this script.
    pub fn rec(self) -> &'static Rec {
        match self {
            Self::Latin => &LATIN,
            Self::Cyrillic => &CYRILLIC,
            Self::Greek => &GREEK,
            Self::Arabic => &ARABIC,
            Self::Korean => &KOREAN,
            Self::Han => &UNIVERSAL,
        }
    }
}

/// The two downloads Settings offers: the files the engine needs to work at all,
/// and the one optional pack with the remaining scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pack {
    BuiltIn,
    Extra,
}

const DETECTOR: ModelFile = ModelFile {
    name: "pp-ocrv5_mobile_det.onnx",
    sha256: "1eb7b4f7ab657ebd1c66d5f79bca7497f29768a2e3c15e52daecbba1a8e4a039",
    size: 4_826_518,
};

const LATIN: Rec = Rec {
    script: Script::Latin,
    rec: ModelFile {
        name: "latin_pp-ocrv5_mobile_rec.onnx",
        sha256: "e3a6bfeea1c8a01d6fccfd480a0bd363fd907f8c65931e228bb2736f5c3e142f",
        size: 8_069_614,
    },
    dict: ModelFile {
        name: "ppocrv5_latin_dict.txt",
        sha256: "ccbcc45730b3fbbd9050c5bc74db6a99067141ef1035e3d14889a84a6b9b1aff",
        size: 2_616,
    },
};

const CYRILLIC: Rec = Rec {
    script: Script::Cyrillic,
    rec: ModelFile {
        name: "cyrillic_pp-ocrv5_mobile_rec.onnx",
        sha256: "a18d96d7c8d73d90f2ed056549caa1de3a8e6cb744cccba16cd593ea8cd2d569",
        size: 8_076_390,
    },
    dict: ModelFile {
        name: "ppocrv5_cyrillic_dict.txt",
        sha256: "db40aa52ceb112055be80c694afdf655d5d2c4f7873704524cc16a447ca913ba",
        size: 2_781,
    },
};

const GREEK: Rec = Rec {
    script: Script::Greek,
    rec: ModelFile {
        name: "el_pp-ocrv5_mobile_rec.onnx",
        sha256: "5a4a020e48e8783e035e1af135423c2161a363acab9ef16e48238c3d181f0f71",
        size: 7_836_326,
    },
    dict: ModelFile {
        name: "ppocrv5_el_dict.txt",
        sha256: "31defc62c0c3ad3674a82da6192226a2ba98ef4ff014a7045cb88d59f9c3de31",
        size: 1_103,
    },
};

const ARABIC: Rec = Rec {
    script: Script::Arabic,
    rec: ModelFile {
        name: "arabic_pp-ocrv5_mobile_rec.onnx",
        sha256: "2768206d9a0ce48eba45b59619184e18161dde8f44115f029920ca17a9dc0384",
        size: 8_026_538,
    },
    dict: ModelFile {
        name: "ppocrv5_arabic_dict.txt",
        sha256: "7f92f7dbb9b75a4787a83bfb4f6d14a8ab515525130c9d40a9036f61cf6999e9",
        size: 2_369,
    },
};

const KOREAN: Rec = Rec {
    script: Script::Korean,
    rec: ModelFile {
        name: "korean_pp-ocrv5_mobile_rec.onnx",
        sha256: "2d7ed96308065a86103325d22af07a88c4d06afc009f21602a4882342c0cc054",
        size: 13_446_374,
    },
    dict: ModelFile {
        name: "ppocrv5_korean_dict.txt",
        sha256: "a88071c68c01707489baa79ebe0405b7beb5cca229f4fc94cc3ef992328802d7",
        size: 47_451,
    },
};

/// The one model the release calls "universal": Chinese (simplified and
/// traditional), Japanese and English.
const UNIVERSAL: Rec = Rec {
    script: Script::Han,
    rec: ModelFile {
        name: "pp-ocrv5_mobile_rec.onnx",
        sha256: "243a0f06d826761323e9045e9b113ab2c191c3aa50565585e628300b8eda0224",
        size: 16_562_373,
    },
    dict: ModelFile {
        name: "ppocrv5_dict.txt",
        sha256: "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b",
        size: 74_012,
    },
};

/// Always installed: most screens are one of these two, and both also read the
/// other's text passably.
pub const BUILT_IN: [&Rec; 2] = [&LATIN, &CYRILLIC];
/// The optional pack: every other script the release has a model for.
pub const EXTRA: [&Rec; 4] = [&GREEK, &ARABIC, &KOREAN, &UNIVERSAL];

/// The recognizers of a pack.
fn recs(pack: Pack) -> &'static [&'static Rec] {
    match pack {
        Pack::BuiltIn => &BUILT_IN,
        Pack::Extra => &EXTRA,
    }
}

/// Every file of a pack. The detector is shared by all scripts and belongs to the
/// built-in set, so deleting the optional pack never takes it away.
fn files(pack: Pack) -> Vec<&'static ModelFile> {
    let mut files: Vec<&'static ModelFile> = recs(pack).iter().flat_map(|rec| rec.files()).collect();
    if pack == Pack::BuiltIn {
        files.push(&DETECTOR);
    }
    files
}

/// %APPDATA%\Glance\models\paddle-ocrv5.
fn dir() -> Option<PathBuf> {
    Some(config::dir()?.join("models").join("paddle-ocrv5"))
}

fn complete(path: &std::path::Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.len() == size)
}

/// Paths of the files `rec` needs when all of them are on disk with the published
/// sizes; `None` while something is missing or truncated.
pub fn paths(rec: &Rec) -> Option<Files> {
    let dir = dir()?;
    let files = Files {
        det: dir.join(DETECTOR.name),
        rec: dir.join(rec.rec.name),
        dict: dir.join(rec.dict.name),
    };
    (complete(&files.det, DETECTOR.size)
        && complete(&files.rec, rec.rec.size)
        && complete(&files.dict, rec.dict.size))
    .then_some(files)
}

/// The paths handed to the OCR engine.
pub struct Files {
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
}

/// The files of `pack` still missing, in download order. Without an APPDATA folder
/// everything counts as missing; the download then fails with its own message.
pub fn missing(pack: Pack) -> Vec<&'static ModelFile> {
    let Some(dir) = dir() else { return files(pack) };
    files(pack).into_iter().filter(|file| !complete(&dir.join(file.name), file.size)).collect()
}

/// True when every file of `pack` is on disk with the published size.
pub fn ready(pack: Pack) -> bool {
    missing(pack).is_empty()
}

/// The whole weight of `pack`, for the size shown before a download starts.
pub fn size(pack: Pack) -> u64 {
    files(pack).iter().map(|file| file.size).sum()
}

/// Total bytes a download of `pack` will move.
pub fn download_size(pack: Pack) -> u64 {
    missing(pack).iter().map(|file| file.size).sum()
}

/// The recognizers that can run now: the built-in two plus whatever the optional
/// pack put on disk.
pub fn available() -> Vec<&'static Rec> {
    BUILT_IN.iter().chain(EXTRA.iter()).copied().filter(|rec| paths(rec).is_some()).collect()
}

/// The recognizers to try for one crop, the script the Windows locale suggests
/// first. It is only a guess: the engine settles it by the scores (see `paddle`).
pub fn candidates() -> Vec<&'static Rec> {
    let mut list = available();
    let preferred = preferred_for(&crate::win32::user_locale());
    if let Some(position) = list.iter().position(|rec| rec.script == preferred.script) {
        list.rotate_left(position);
    }
    list
}

/// The recognizer a locale suggests. Locales no model covers get Latin.
fn preferred_for(locale: &str) -> &'static Rec {
    match locale.split('-').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "el" => &GREEK,
        "ar" | "fa" | "ur" | "ps" | "sd" | "ug" => &ARABIC,
        "ko" => &KOREAN,
        "zh" | "ja" => &UNIVERSAL,
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "kk" | "ky" | "mn" | "tg" | "tt" | "ba" | "cv" | "ce"
        | "os" => &CYRILLIC,
        _ => &LATIN,
    }
}

/// The script a text is mostly written in, counted over its letters; `None` when
/// there are none. Characters no model covers count as Latin, which is what the
/// Latin recognizer does with them anyway.
pub fn script_of(text: &str) -> Option<Script> {
    const ORDER: [Script; 6] =
        [Script::Latin, Script::Cyrillic, Script::Greek, Script::Arabic, Script::Korean, Script::Han];
    let mut counts = [0usize; ORDER.len()];
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        let index = match c as u32 {
            0x0400..=0x052F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => 1,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => 2,
            0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => 3,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF => 4,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => 5,
            _ => 0,
        };
        counts[index] += 1;
    }
    let (index, count) = counts.iter().enumerate().max_by_key(|(_, count)| **count)?;
    (*count > 0).then(|| ORDER[index])
}

/// The release every file is downloaded from.
pub fn url(file: &ModelFile) -> String {
    format!("https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/{}", file.name)
}

/// Downloads on a short-lived worker thread, posting progress and the outcome.
/// One download at a time is enough: the Settings buttons are disabled while one
/// runs.
pub fn spawn(pack: Pack) {
    let worker = std::thread::Builder::new().name("glance-paddle-models".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            download(pack, &|done, total| app::post(Action::PaddleDownloadProgress { done, total }))
        }))
        .unwrap_or_else(|_| Err(Error::new(strings::paddle_download_failed("the download stopped unexpectedly"))));
        app::post(Action::PaddleDownloadDone { result });
    });
    if let Err(e) = worker {
        app::post(Action::PaddleDownloadDone { result: Err(Error::new(e.to_string())) });
    }
}

/// Removes the files of `pack` and leaves the rest. Settings only offers this for
/// the optional pack; the built-in files are what the engine runs on.
pub fn delete(pack: Pack) -> Result<()> {
    let Some(dir) = dir() else { return Ok(()) };
    for file in files(pack) {
        let path = dir.join(file.name);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| Error::new(strings::paddle_delete_failed(file.name, &e.to_string())))?;
        }
    }
    Ok(())
}

/// Files earlier versions downloaded and no pack uses any more: the East Slavic and
/// English recognizers, replaced by the Cyrillic and Latin ones.
const RETIRED: [&str; 4] = [
    "eslav_pp-ocrv5_mobile_rec.onnx",
    "ppocrv5_eslav_dict.txt",
    "en_pp-ocrv5_mobile_rec.onnx",
    "ppocrv5_en_dict.txt",
];

/// Removes the files of earlier versions once the current ones are in place.
fn remove_retired(dir: &std::path::Path) {
    for name in RETIRED {
        let path = dir.join(name);
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Drops the files earlier versions downloaded and nothing uses now, so they do not
/// sit in %APPDATA% for nothing. Called once at startup.
pub fn clean_up() {
    if let Some(dir) = dir() {
        remove_retired(&dir);
    }
}

/// Fetches the files `pack` is missing, reporting progress as bytes done out of
/// the total. Every file is checked against its published SHA-256 before it lands
/// by atomic rename, so a file is either absent or complete and valid.
fn download(pack: Pack, report: &dyn Fn(u64, u64)) -> Result<()> {
    let dir = dir().ok_or_else(|| Error::new("The APPDATA folder is not available."))?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::new(strings::paddle_download_failed(&e.to_string())))?;
    let total = download_size(pack);
    let mut done = 0u64;
    for file in missing(pack) {
        let bytes = fetch(&url(file), file, done, total, report)?;
        done += file.size;
        config::write_atomic(&dir.join(file.name), &bytes)
            .map_err(|e| Error::new(strings::paddle_download_failed(&e.to_string())))?;
    }
    remove_retired(&dir);
    Ok(())
}

/// One file into memory, with progress reports. Large models arrive in ~8 MB
/// pieces, so buffering one file costs little and lets the hash be checked before
/// anything touches the disk.
fn fetch(url: &str, file: &ModelFile, done_before: u64, total: u64, report: &dyn Fn(u64, u64)) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(concat!("Glance/", env!("CARGO_PKG_VERSION")))
        // SChannel TLS like every other request in Glance (see http.rs); ureq's
        // default provider would need the rustls feature.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let response = agent.get(url).call().map_err(|e| Error::new(strings::paddle_download_failed(&describe(&e))))?;
    let mut bytes = Vec::with_capacity(file.size as usize);
    let mut reader = response.into_body().into_reader();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut reported = 0u64;
    loop {
        let read = reader.read(&mut chunk).map_err(|e| Error::new(strings::paddle_download_failed(&e.to_string())))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        let position = done_before + bytes.len() as u64;
        if position - reported >= 256 * 1024 {
            reported = position;
            report(position, total);
        }
    }
    if bytes.len() as u64 != file.size {
        return Err(Error::new(strings::paddle_corrupt(file.name)));
    }
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if hash != file.sha256 {
        return Err(Error::new(strings::paddle_corrupt(file.name)));
    }
    report(done_before + file.size, total);
    Ok(bytes)
}

/// Network failures in one line of user-facing text.
fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => "the download timed out".to_owned(),
        ureq::Error::HostNotFound => "the server address could not be resolved".to_owned(),
        ureq::Error::ConnectionFailed => "the connection failed".to_owned(),
        ureq::Error::StatusCode(code) => format!("GitHub returned {code}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_known_and_distinct() {
        let all: Vec<&ModelFile> = BUILT_IN
            .iter()
            .chain(EXTRA.iter())
            .flat_map(|rec| rec.files())
            .chain([&DETECTOR])
            .collect();
        let mut names: Vec<_> = all.iter().map(|file| file.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "two packs share a file");
        for file in all {
            assert_eq!(file.sha256.len(), 64, "{} hash", file.name);
            assert!(file.size > 1_000);
        }
        for rec in BUILT_IN.iter().chain(EXTRA.iter()) {
            assert!(rec.rec.name.ends_with(".onnx") && rec.dict.name.ends_with(".txt"));
            assert!(rec.rec.size > 1_000_000 && rec.dict.size < 100_000);
        }
    }

    #[test]
    fn packs_weigh_what_their_files_do() {
        // The detector belongs to the built-in set only, so the optional pack can be
        // deleted without taking it away.
        assert_eq!(size(Pack::BuiltIn), 4_826_518 + 8_069_614 + 2_616 + 8_076_390 + 2_781);
        assert_eq!(size(Pack::Extra), 7_836_326 + 1_103 + 8_026_538 + 2_369 + 13_446_374 + 47_451 + 16_562_373 + 74_012);
        assert_eq!(files(Pack::Extra).len(), 8);
        assert_eq!(files(Pack::BuiltIn).len(), 5);
    }

    #[test]
    fn locales_suggest_their_own_script() {
        let preferred = |locale: &str| preferred_for(locale).script;
        assert_eq!(preferred("ru-RU"), Script::Cyrillic);
        assert_eq!(preferred("uk-UA"), Script::Cyrillic);
        assert_eq!(preferred("en-US"), Script::Latin);
        assert_eq!(preferred("de-DE"), Script::Latin);
        assert_eq!(preferred("el-GR"), Script::Greek);
        assert_eq!(preferred("ar-EG"), Script::Arabic);
        assert_eq!(preferred("ko-KR"), Script::Korean);
        assert_eq!(preferred("zh-CN"), Script::Han);
        assert_eq!(preferred("ja-JP"), Script::Han);
        assert_eq!(preferred(""), Script::Latin);
    }

    #[test]
    fn reads_the_script_of_a_text() {
        assert_eq!(script_of("Glance reads any text"), Some(Script::Latin));
        assert_eq!(script_of("Привет, мир!"), Some(Script::Cyrillic));
        assert_eq!(script_of("Українська мова"), Some(Script::Cyrillic));
        assert_eq!(script_of("Καλημέρα"), Some(Script::Greek));
        assert_eq!(script_of("مرحبا"), Some(Script::Arabic));
        assert_eq!(script_of("안녕하세요"), Some(Script::Korean));
        assert_eq!(script_of("你好世界"), Some(Script::Han));
        assert_eq!(script_of("こんにちは"), Some(Script::Han));
        // Numbers, spaces and punctuation say nothing about the script.
        assert_eq!(script_of("12 345 — 6.7!"), None);
        assert_eq!(script_of(""), None);
        // The dominant script wins on a mixed line.
        assert_eq!(script_of("Привет, Glance"), Some(Script::Cyrillic));
    }

    #[test]
    fn scripts_have_their_own_recognizer() {
        let recs: Vec<&Rec> = [Script::Latin, Script::Cyrillic, Script::Greek, Script::Arabic, Script::Korean, Script::Han]
            .iter()
            .map(|script| script.rec())
            .collect();
        for (script, rec) in [Script::Latin, Script::Cyrillic, Script::Greek, Script::Arabic, Script::Korean, Script::Han]
            .iter()
            .zip(&recs)
        {
            assert_eq!(rec.script, *script);
        }
        assert_eq!(recs.len(), 6, "every script maps to a recognizer");
        // The packs hold the same recognizers the scripts point at.
        for (rec, script) in BUILT_IN.iter().chain(EXTRA.iter()).zip(
            [Script::Latin, Script::Cyrillic, Script::Greek, Script::Arabic, Script::Korean, Script::Han].iter(),
        ) {
            assert_eq!(rec.script, *script);
        }
    }

    #[test]
    fn builds_release_urls() {
        assert_eq!(
            url(&DETECTOR),
            "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-ocrv5_mobile_det.onnx"
        );
        assert_eq!(url(&LATIN.rec), "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/latin_pp-ocrv5_mobile_rec.onnx");
    }

    #[test]
    #[ignore = "downloads from GitHub; run explicitly with --ignored"]
    fn fetches_a_file_and_verifies_its_hash() {
        let file = &LATIN.dict;
        let bytes = fetch(&url(file), file, 0, file.size, &|_, _| {}).unwrap();
        assert_eq!(bytes.len() as u64, file.size);
    }
}
