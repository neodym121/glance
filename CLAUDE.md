# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Glance is a Windows-only tray app (Rust 2024, [Slint](https://slint.dev) UI): a global hotkey freezes the screen, the user drags over text, and the selection is recognized (local Paddle OCR or a cloud vision model) and optionally translated. No WebView or browser engine; idle memory (~2 MB) is a deliberate design goal.

## Commands

```bash
cargo check
cargo test --locked                       # all unit tests (they live inline in src/)
cargo test --locked enlarges_small_crops  # one test, or every test whose path contains the filter
cargo run -- --settings                   # `--settings` opens the settings window at start
cargo run --example ui_snapshots -- <out-dir> [scale] [frame.png]   # renders every window to PNG, offscreen
cargo run --example paddle_smoke -- <image.png> [script-code]      # runs local Paddle OCR on an image, prints text + timing
python tools/make_icons.py                # regenerates assets/*.ico and glance.png (stdlib only)
```

- The binary uses `windows_subsystem = "windows"` outside tests, so `println!`/`eprintln!` go nowhere when running it; surface problems through `Error` / `win32::message_box`.
- Release artifacts (`.msi` via WiX, portable `.exe`) are built only by `.github/workflows/build.yml` on a `v*` tag. The version comes from `Cargo.toml`; the MSI strips the pre-release suffix (`0.0.1-beta` → `0.0.1`).
- `ui/*.slint` is compiled by `build.rs` (entry point `ui/app.slint`) and pulled in with `slint::include_modules!()` in both `src/main.rs` and `examples/ui_snapshots.rs`. The snapshot example is the way to look at UI changes without running the tray app.

## Architecture

**One event loop, one state owner.** Everything (hotkey, tray, Slint callbacks, worker results) becomes an `app::Action` and is handled in `App::handle` on the UI thread. Use `app::post` from any thread and `app::handle_now` from a Slint callback. `App` lives in a thread-local `RefCell`; if an action arrives while another is running (nested message loop), `dispatch` retries after 20 ms. Message boxes go through `notify()` (deferred timer) so they never nest inside a borrowed `App`.

**Lifecycle** is `State::Idle → Selecting(overlay::Session) → Result(popup::Session) → Idle`. Each step destroys the previous step's windows and buffers, then `schedule_trim()` hands memory back to Windows. `App.job` is bumped on every new recognition and on `set_idle`; `OcrDone`/`TranslationDone` carry the job id and stale results are dropped.

**Capture → text pipeline**
1. `capture.rs` takes one GDI `BitBlt` per monitor into a `Frame` (original + half-brightness copy). `overlay.rs` opens one frameless window per monitor showing them; the selection is cut from the *original* frame in Rust (never from what is drawn), so the scrim/hint can't reach recognition. The crop is padded with its border color (`RECOGNITION_PAD`) so edge glyphs aren't clipped.
2. `ocr::spawn` runs a worker thread: `recognizer_for` builds a `Recognizer` (`paddle::PaddleOcr`, the local engine, or `vision::VisionOcr`), reading the API key from Credential Manager on that thread. Both engines enlarge small crops before recognizing (3x, else 2x; ratios were measured, see comments). `paddle.rs` caps the enlargement at the detector's 960 px input, builds its engine from files in `%APPDATA%\Glance\models\paddle-ocrv5` and drops it after one recognition; `paddle_models.rs` holds the script table (names, sizes, SHA-256 from the oar-ocr model registry) and the Settings download with progress. Line joining (`join_lines`) lives in `ocr/mod.rs`.
3. `popup.rs` shows the result card; Translate calls `translate::spawn` (a `Translator` trait: AI chat, Google free/paid, DeepL, Microsoft). Only text is sent for translation, never the image.

**Providers** (`providers.rs`): `AiProvider` (OpenAI-compatible chat: Gemini, Mistral, Groq, Polza) is shared by vision OCR (`ocr/vision.rs`) and translation via `llm::chat`. `OcrProvider`/`TranslationProvider` extend it with the local Paddle engine and the non-chat translators. Retired provider ids ("openrouter", "windows") deserialize to the default via `serde(other)`, so old configs keep loading. The order of the `ALL` lists is the order of the Settings pickers (AI providers first, same order; a test enforces it), so index-based UI code depends on it. Adding an AI provider touches `providers.rs`, `config.rs` (`Models`/`Routes`), `models.rs` (list parsing), `settings.rs` + `ui/settings.slint` (per-provider properties), and `Secret`.

**Config and secrets.** `config.rs` → `%APPDATA%\Glance\config.json` (plus `models.json`, a model-list cache), atomic writes, `#[serde(default)]` everywhere so new fields stay backward compatible. API keys never go in the config (a test checks field names); they live in Windows Credential Manager (`secrets.rs`, targets `Glance:<provider>`). All HTTP goes through `http.rs`, which builds a fresh client per request and scrubs the key from every error message (`redact`); `error::Error` is a user-facing message string, so build it only from redacted text.

**Settings** (`settings.rs` + `ui/settings.slint`) has no Save button: a timer posts `SettingsPoll`, `Settings::read` snapshots the window, and `App::apply` diffs against the last applied `Snapshot` (hotkey re-registration, key writes, autostart, config save). The hotkey is released while the shortcut field records.

**Windows specifics.** `win32.rs` holds the Win32 helpers (clipboard, autostart in the registry, single-instance mutex, DWM tweaks); new `windows` API usage needs its feature added in `Cargo.toml`. `window_role.rs` sets native window attributes through a Slint hook, so every window must be created as `window_role::create(Role::X, Component::new)`.

## UI notes

- Slint uses the software renderer only (no GPU context) with the winit backend; `unstable-winit-030` is tied to the Slint minor version, so upgrade Slint deliberately.
- The software renderer repaints only changed regions. The overlay relies on this: the dimmed image is the backdrop and the original shows only inside the selection, so geometry that follows the pointer must not span the whole screen (see the header of `ui/overlay.slint`).
- Design tokens are in `ui/theme.slint`; their source, `media/DESIGN.md`, is git-ignored and local only. Shared widgets are in `ui/components.slint`.
- User-facing text produced in Rust belongs in `src/strings.rs`; Slint text uses `@tr()`.
