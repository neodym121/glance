//! Runs the local Paddle OCR engine on an image and prints what it read, with
//! timing — for checking recognition quality and speed on real screenshots.
//!
//!     cargo run --example paddle_smoke -- <image.png> [script-code]
//!
//! The script code defaults to Automatic (resolved from the Windows locale); the
//! model files must already be downloaded (%APPDATA%\Glance\models\paddle-ocrv5,
//! see Settings → Recognition). The pipeline mirrors `src/ocr/paddle.rs`; keep the
//! two in sync when the preprocessing there changes.

use std::time::Instant;

use oar_ocr::oarocr::OAROCRBuilder;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), script_setting) = (args.next(), args.next().unwrap_or_default()) else {
        eprintln!("usage: paddle_smoke <image.png> [script-code]");
        std::process::exit(2);
    };
    let image = read_png(&path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));

    // Same script resolution and path lookup as the recognizer.
    let script = script_for(&script_setting);
    let dir = std::env::var_os("APPDATA")
        .map(|base| std::path::PathBuf::from(base).join("Glance").join("models").join("paddle-ocrv5"))
        .expect("APPDATA");
    let (det, rec, dict) = (
        dir.join("pp-ocrv5_mobile_det.onnx"),
        dir.join(format!("{}_pp-ocrv5_mobile_rec.onnx", script.rec_prefix)),
        dir.join(script.dict),
    );
    for file in [&det, &rec, &dict] {
        if !file.exists() {
            panic!("model file missing: {} — download it in Settings → Recognition", file.display());
        }
    }

    let started = Instant::now();
    let engine = OAROCRBuilder::new(det, rec, dict).build().expect("engine builds");
    let built = started.elapsed();
    let factor = enlargement(image.0, image.1);
    let (width, height) = (image.0 * factor, image.1 * factor);
    let pixels = if factor > 1 { glance_resample(&image.2, image.0, image.1, width, height) } else { image.2 };
    let prepared = image::RgbImage::from_raw(width, height, pixels).expect("buffer matches");
    let recognized = Instant::now();
    let results = engine.predict(vec![prepared]).expect("recognition runs");
    println!("built in {built:.2?}, recognized in {:.2?}", recognized.elapsed());

    let regions = &results[0].text_regions;
    for region in regions {
        let (text, confidence) = region.text_with_confidence().unwrap_or(("", 0.0));
        let box_ = &region.bounding_box;
        println!("  [{confidence:.2}] {:.0},{:.0}: {text}", box_.x_min(), box_.y_min());
    }
    let text: Vec<&str> = regions.iter().filter_map(|r| r.text.as_deref()).collect::<Vec<_>>();
    println!("{}\n---\n{}", regions.len(), text.join(" "));
}

/// The script table of `src/ocr/paddle_models.rs`, reduced to what a smoke run
/// needs: the recognition model's file-name prefix and the dictionary.
struct SmokeScript {
    code: &'static str,
    rec_prefix: &'static str,
    dict: &'static str,
}

const SCRIPTS: &[SmokeScript] = &[
    SmokeScript { code: "eslav", rec_prefix: "eslav", dict: "ppocrv5_eslav_dict.txt" },
    SmokeScript { code: "en", rec_prefix: "en", dict: "ppocrv5_en_dict.txt" },
];

fn script_for(setting: &str) -> &'static SmokeScript {
    // A named code wins; anything else falls back to East Slavic (the Windows
    // locale the app would consult is not worth reaching for in a smoke run).
    SCRIPTS.iter().find(|s| s.code.eq_ignore_ascii_case(setting.trim())).unwrap_or(&SCRIPTS[0])
}

/// 3 or 2 while the enlarged long side stays within the detector's 960 px input
/// (as in `src/ocr/paddle.rs`).
fn enlargement(width: u32, height: u32) -> u32 {
    let longest = width.max(height);
    if longest.saturating_mul(3) <= 960 {
        3
    } else if longest.saturating_mul(2) <= 960 {
        2
    } else {
        1
    }
}

/// Box-average downscale / nearest-with-smoothing upscale stand-in: the app's
/// cubic resampler is in the bin crate, unreachable from examples. For a smoke
/// run the exact kernel hardly matters; 2x/3x nearest keeps glyph edges hard.
fn glance_resample(source: &[u8], width: u32, height: u32, new_width: u32, new_height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(new_width as usize * new_height as usize * 3);
    for y in 0..new_height {
        let sy = y * height / new_height;
        for x in 0..new_width {
            let sx = x * width / new_width;
            let at = (sy as usize * width as usize + sx as usize) * 3;
            out.extend_from_slice(&source[at..at + 3]);
        }
    }
    out
}

fn read_png(path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut pixels = vec![0u8; reader.output_buffer_size().ok_or("no frame size")?];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    let bytes = info.buffer_size();
    pixels.truncate(bytes);
    match info.color_type {
        png::ColorType::Rgb => Ok((info.width, info.height, pixels)),
        png::ColorType::Rgba => {
            let rgb = pixels.chunks_exact(4).flat_map(|px| [px[0], px[1], px[2]]).collect();
            Ok((info.width, info.height, rgb))
        }
        _ => Err("expected an RGB(A) PNG".into()),
    }
}
