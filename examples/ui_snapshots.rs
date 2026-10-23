//! Renders Glance's windows off-screen with the Slint software renderer and saves
//! them as PNGs, for reviewing the UI without running the tray app.
//!
//!     cargo run --example ui_snapshots -- <out-dir> [scale] [frame.png]
//!
//! `frame.png` is an optional screenshot to show behind the capture overlay.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Image, LogicalPosition, ModelRc, PhysicalSize, Rgb8Pixel, SharedPixelBuffer, SharedString, VecModel};

slint::include_modules!();

thread_local! {
    static LAST_WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct Offscreen;

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        LAST_WINDOW.with(|last| *last.borrow_mut() = Some(window.clone()));
        Ok(window)
    }
}

fn last_window() -> Rc<MinimalSoftwareWindow> {
    LAST_WINDOW.with(|last| last.borrow_mut().take().expect("a window was created"))
}

fn render(window: &MinimalSoftwareWindow, scale: f32, logical_w: f32, logical_h: f32, path: &Path) {
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    let (w, h) = ((logical_w * scale).round() as u32, (logical_h * scale).round() as u32);
    window.set_size(PhysicalSize::new(w, h));
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        let mut buffer = vec![Rgb8Pixel::default(); (w * h) as usize];
        renderer.render(&mut buffer, w as usize);
        let bytes: Vec<u8> = buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        let file = std::fs::File::create(path).expect("create png");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
    });
    println!("wrote {}", path.display());
}

fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(items.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>())))
}

fn load_frame(path: &Path) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    let channels = info.color_type.samples();
    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(info.width, info.height);
    for (dst, src) in buffer.make_mut_bytes().as_chunks_mut::<3>().0.iter_mut().zip(data.chunks_exact(channels)) {
        dst.copy_from_slice(&src[..3]);
    }
    Some(buffer)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("snapshots"));
    let scale: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let frame = args.get(3).map(PathBuf::from);
    std::fs::create_dir_all(&out).unwrap();
    slint::platform::set_platform(Box::new(Offscreen)).unwrap();

    // Result popup in each state.
    let text = "Glance reads the text inside the selected area and shows it here.\n\
                Docs: https://docs.example.com/glance (see the Getting started section).";
    for (name, state, body, error, translated) in [
        ("popup-recognizing", PopupState::Recognizing, "", "", false),
        ("popup-text", PopupState::Text, text, "", false),
        ("popup-translation", PopupState::Text, "Glance распознаёт текст в выделенной области и показывает его здесь.", "", true),
        ("popup-error", PopupState::Error, "", "Windows OCR has no languages installed. Install one from an administrator PowerShell:
Add-WindowsCapability -Online -Name Language.OCR~~~ru-RU~0.0.1.0
Or choose a cloud engine in Glance Settings → Recognition.", false),
    ] {
        let popup = ResultPopup::new().unwrap();
        let window = last_window();
        popup.set_popup_width(420.0);
        popup.set_state(state);
        popup.set_text(body.into());
        popup.set_error(error.into());
        popup.set_has_link(body.contains("https://"));
        popup.set_actions_enabled(state != PopupState::Recognizing);
        popup.set_showing_translation(translated);
        if name == "popup-text" {
            popup.set_status("Copied to clipboard".into());
            popup.set_status_ok(true);
        } else {
            popup.set_status(if translated { "DeepL · Russian" } else { "Windows OCR" }.into());
        }
        popup.show().unwrap();
        render(&window, scale, popup.get_popup_width(), popup.get_desired_height(), &out.join(format!("{name}.png")));
    }

    // Settings, every page; Recognition with a Polza AI model list, Translation with a
    // search that matches nothing in the list.
    let models = [
        ("ALLaM-2-7b", "allam-2-7b"),
        ("Anthropic: Claude Haiku 5.5", "anthropic/claude-haiku-5.5"),
        ("GPT OSS 120B", "openai/gpt-oss-120b"),
        ("Google: Gemini 3.1 Flash Lite", "google/gemini-3.1-flash-lite"),
        ("Qwen/Qwen3.8-27B", "qwen/qwen3.8-27b"),
    ];
    for page in 0..5 {
        let settings = SettingsWindow::new().unwrap();
        let window = last_window();
        settings.set_page(page);
        settings.set_hotkey("Ctrl+Alt+S".into());
        settings.set_auto_copy(true);
        settings.set_ocr_providers(strings(&["OpenRouter", "Google Gemini", "Mistral AI", "Groq", "Polza AI", "Windows OCR (on this PC)"]));
        settings.set_ocr_provider(if page == 1 { 4 } else { 0 });
        settings.set_ocr_languages(strings(&["Automatic", "English (United States)", "Russian"]));
        settings.set_ocr_languages_missing(true);
        settings.set_ocr_capability("Language.OCR~~~ru-RU~0.0.1.0".into());
        settings.set_ocr_default_polza("google/gemini-3.1-flash-lite".into());
        settings.set_ocr_route_polza("google-vertex/global".into());
        settings.set_key_openrouter("sk-or-v1-0000000000".into());
        settings.set_translation_providers(strings(&[
            "OpenRouter", "Google Gemini", "Mistral AI", "Groq", "Polza AI", "Google Translate", "DeepL", "Microsoft Translator",
        ]));
        settings.set_translation_provider(if page == 2 { 1 } else { 6 });
        settings.set_target_languages(strings(&["English", "German", "Russian"]));
        settings.set_target_language_index(2);
        settings.set_version("0.0.1-beta".into());
        let list = settings.global::<ModelList>();
        let (items, query): (Vec<ModelItem>, &str) = if page == 2 {
            (vec![ModelItem { id: "my-org/custom-model".into(), name: "my-org/custom-model".into(), custom: true }], "my-org/custom-model")
        } else {
            (models.iter().map(|(name, id)| ModelItem { id: (*id).into(), name: (*name).into(), custom: false }).collect(), "")
        };
        list.set_items(ModelRc::from(Rc::new(VecModel::from(items))));
        list.set_query(query.into());
        list.set_selected("Google: Gemini 3.1 Flash Lite (default)".into());
        settings.show().unwrap();
        render(&window, scale, 920.0, 900.0, &out.join(format!("settings-{page}.png")));
    }

    // Capture overlay with a selection being dragged.
    if let Some(frame) = frame.as_deref().and_then(load_frame) {
        let overlay = OverlayWindow::new().unwrap();
        let window = last_window();
        let (w, h) = (frame.width() as f32 / scale, frame.height() as f32 / scale);
        overlay.set_frame(Image::from_rgb8(frame));
        overlay.show().unwrap();
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        window.set_size(PhysicalSize::new((w * scale) as u32, (h * scale) as u32));
        render(&window, scale, w, h, &out.join("overlay-idle.png"));
        let start = LogicalPosition::new(w * 0.25, h * 0.42);
        let end = LogicalPosition::new(w * 0.62, h * 0.6);
        window.dispatch_event(WindowEvent::PointerMoved { position: start });
        window.dispatch_event(WindowEvent::PointerPressed { position: start, button: PointerEventButton::Left });
        window.dispatch_event(WindowEvent::PointerMoved { position: end });
        render(&window, scale, w, h, &out.join("overlay-selecting.png"));
    }
}
