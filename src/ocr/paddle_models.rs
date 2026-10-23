//! The model files of the local Paddle OCR engine: which scripts exist, where the
//! files live, and how Settings downloads them. One recognition model per script
//! plus a shared detector, ~12 MB together, stored in %APPDATA%\Glance\models and
//! fetched once from the oar-ocr GitHub release. Names, sizes and hashes mirror the
//! crate's own model registry (oar-ocr-core `download::registry`).

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

/// A recognition model plus its dictionary. The detector is shared by all scripts.
pub struct Script {
    /// Value stored in the config and shown by the Settings picker.
    pub code: &'static str,
    /// Full label for the Settings picker.
    pub label: &'static str,
    /// Short name for the popup footer.
    pub short: &'static str,
    /// Windows locales (primary subtag) this script reads best; the first match
    /// wins for the Automatic setting.
    pub locales: &'static [&'static str],
    pub rec: ModelFile,
    dict: ModelFile,
}

const DETECTOR: ModelFile = ModelFile {
    name: "pp-ocrv5_mobile_det.onnx",
    sha256: "1eb7b4f7ab657ebd1c66d5f79bca7497f29768a2e3c15e52daecbba1a8e4a039",
    size: 4_826_518,
};

/// The scripts worth offering. More exist upstream (Greek, Korean, Arabic, Thai,
/// Devanagari, Tamil, Telugu, Chinese, a pan-Cyrillic and a pan-Latin model), but
/// on real captures their quality is indistinguishable from these two, so the
/// picker stays short. Both models also read the other's text passably.
pub const SCRIPTS: &[Script] = &[
    Script {
        code: "eslav",
        label: "Slavic",
        short: "Slavic",
        // East Slavic Windows locales; the model reads Russian, Ukrainian and
        // Belarusian plus Latin text.
        locales: &["ru", "uk", "be"],
        rec: ModelFile {
            name: "eslav_pp-ocrv5_mobile_rec.onnx",
            sha256: "36a66a68097e88b103e0f60f489e88c7239d3ea79d96fbac2d80ac9d134944cd",
            size: 7_915_218,
        },
        dict: ModelFile {
            name: "ppocrv5_eslav_dict.txt",
            sha256: "3e95f1581557162870cacdba5af91a4c6be2890710d395b0c3c7578e7ee5e6eb",
            size: 1_663,
        },
    },
    Script {
        code: "en",
        label: "English",
        short: "English",
        locales: &["en"],
        rec: ModelFile {
            name: "en_pp-ocrv5_mobile_rec.onnx",
            sha256: "8307465d3c9ef2ba4055c3bd0be55aafe11f518630212b7598b70ccb376028ac",
            size: 7_876_014,
        },
        dict: ModelFile {
            name: "ppocrv5_en_dict.txt",
            sha256: "e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6",
            size: 1_416,
        },
    },
];

/// The script for a config value: the named one, or the Automatic pick for the
/// Windows locale — Slavic for East Slavic locales, English for everything else.
pub fn resolve(setting: &str) -> &'static Script {
    resolve_for(setting, &crate::win32::user_locale())
}

fn resolve_for(setting: &str, locale: &str) -> &'static Script {
    let code = setting.trim();
    if let Some(script) = SCRIPTS.iter().find(|s| s.code.eq_ignore_ascii_case(code)) {
        return script;
    }
    let primary = locale.split('-').next().unwrap_or("").to_ascii_lowercase();
    SCRIPTS
        .iter()
        .find(|s| s.locales.iter().any(|l| *l == primary))
        .unwrap_or(&SCRIPTS[1])
}

/// %APPDATA%\Glance\models\paddle-ocrv5.
fn dir() -> Option<PathBuf> {
    Some(config::dir()?.join("models").join("paddle-ocrv5"))
}

/// Paths of the files `script` needs when all of them are on disk with the
/// published sizes; `None` while something is missing or truncated.
pub fn paths(script: &Script) -> Option<Files> {
    let dir = dir()?;
    let files = Files {
        det: dir.join(DETECTOR.name),
        rec: dir.join(script.rec.name),
        dict: dir.join(script.dict.name),
    };
    let present = |path: &std::path::Path, size: u64| {
        std::fs::metadata(path).is_ok_and(|meta| meta.len() == size)
    };
    (present(&files.det, DETECTOR.size)
        && present(&files.rec, script.rec.size)
        && present(&files.dict, script.dict.size))
    .then_some(files)
}

/// The paths handed to the OCR engine.
pub struct Files {
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
}

/// The files `script` still needs, in download order. Without an APPDATA folder
/// everything counts as missing; the download then fails with its own message.
pub fn missing(script: &'static Script) -> Vec<&'static ModelFile> {
    let Some(dir) = dir() else {
        return vec![&DETECTOR, &script.rec, &script.dict];
    };
    [&DETECTOR, &script.rec, &script.dict]
        .into_iter()
        .filter(|file| !std::fs::metadata(dir.join(file.name)).is_ok_and(|meta| meta.len() == file.size))
        .collect()
}

/// The release every file is downloaded from.
pub fn url(file: &ModelFile) -> String {
    format!("https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/{}", file.name)
}

/// Total bytes the download will move, for the Settings status line.
pub fn download_size(script: &'static Script) -> u64 {
    missing(script).iter().map(|file| file.size).sum()
}

/// Downloads on a short-lived worker thread, posting progress and the outcome.
/// One download at a time is enough: the Settings button starts disabled while
/// one runs.
pub fn spawn(script: &'static Script) {
    let worker = std::thread::Builder::new().name("glance-paddle-models".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            download(script, &|done, total| app::post(Action::PaddleDownloadProgress { done, total }))
        }))
        .unwrap_or_else(|_| Err(Error::new(strings::paddle_download_failed("the download stopped unexpectedly"))));
        app::post(Action::PaddleDownloadDone { result });
    });
    if let Err(e) = worker {
        app::post(Action::PaddleDownloadDone { result: Err(Error::new(e.to_string())) });
    }
}

/// Fetches the files `script` is missing, reporting progress as bytes done out of
/// the total. Every file is checked against its published SHA-256 before it lands
/// by atomic rename, so a file is either absent or complete and valid.
fn download(script: &'static Script, report: &dyn Fn(u64, u64)) -> Result<()> {
    let dir = dir().ok_or_else(|| Error::new("The APPDATA folder is not available."))?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::new(strings::paddle_download_failed(&e.to_string())))?;
    let total = download_size(script);
    let mut done = 0u64;
    for file in missing(script) {
        let bytes = fetch(&url(file), file, done, total, report)?;
        done += file.size;
        config::write_atomic(&dir.join(file.name), &bytes)
            .map_err(|e| Error::new(strings::paddle_download_failed(&e.to_string())))?;
    }
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
    fn scripts_have_unique_codes_and_known_shapes() {
        let mut codes: Vec<_> = SCRIPTS.iter().map(|s| s.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), SCRIPTS.len());
        for script in SCRIPTS {
            assert!(!script.label.is_empty() && !script.short.is_empty(), "{} needs names", script.code);
            assert!(script.rec.name.ends_with(".onnx") && script.dict.name.ends_with(".txt"));
            assert_eq!(script.rec.sha256.len(), 64, "{} rec hash", script.code);
            assert!(script.rec.size > 1_000_000 && script.dict.size < 100_000);
        }
        // The Automatic fallback for locales no script claims is English.
        assert_eq!(SCRIPTS[1].code, "en");
    }

    #[test]
    fn locales_map_to_one_script_each() {
        let mut all: Vec<_> = SCRIPTS.iter().flat_map(|s| s.locales.iter()).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), SCRIPTS.iter().map(|s| s.locales.len()).sum::<usize>(), "a locale names two scripts");
    }

    #[test]
    fn resolves_named_scripts_and_automatic_by_locale() {
        assert_eq!(resolve_for("eslav", "ru-RU").code, "eslav");
        assert_eq!(resolve_for(" en ", "ru-RU").code, "en", "a named script beats the locale");
        assert_eq!(resolve_for("", "ru-RU").code, "eslav");
        assert_eq!(resolve_for("", "be-BY").code, "eslav");
        assert_eq!(resolve_for("", "en-US").code, "en");
        assert_eq!(resolve_for("", "de-DE").code, "en", "no script claims German");
        assert_eq!(resolve_for("", "zh-CN").code, "en");
        assert_eq!(resolve_for("nonsense", "ru-RU").code, "eslav", "unknown values fall back like Automatic");
    }

    #[test]
    fn builds_release_urls() {
        assert_eq!(
            url(&DETECTOR),
            "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-ocrv5_mobile_det.onnx"
        );
    }

    #[test]
    #[ignore = "downloads from GitHub; run explicitly with --ignored"]
    fn fetches_a_file_and_verifies_its_hash() {
        let script = &SCRIPTS[0];
        let bytes = fetch(&url(&script.dict), &script.dict, 0, script.dict.size, &|_, _| {}).unwrap();
        assert_eq!(bytes.len() as u64, script.dict.size);
    }
}
