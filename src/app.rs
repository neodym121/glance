//! Application state on the UI thread. Every event (hotkey, tray, window callbacks,
//! worker results) arrives as an `Action` through the Slint event loop, so state is
//! only touched from one place and never re-entrantly.
//!
//! Lifecycle: idle (tray only) → selecting (overlay windows + frozen frames) →
//! result (popup + one worker thread) → idle. Each step destroys what the previous
//! one created.

use std::cell::RefCell;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::models::ModelInfo;
use crate::providers::AiProvider;
use crate::settings::{self, KeyChange, Settings};
use crate::tray::Tray;
use crate::{hotkey, languages, links, ocr, overlay, popup, secrets, strings, translate, win32, window_role};

pub enum Action {
    Capture,
    CaptureAfterMenu,
    OpenSettings,
    Quit,
    OverlayCancel,
    OverlaySelected { screen: usize, x: f32, y: f32, w: f32, h: f32 },
    OcrDone { job: u64, result: Result<String> },
    TranslationDone { job: u64, result: Result<String> },
    PopupClose,
    PopupFocusLost,
    PopupCopy,
    PopupTranslate,
    PopupOpenLink,
    SettingsPoll,
    SettingsClose,
    HotkeyRecording(bool),
    ModelsContext,
    ModelsRefresh,
    ModelsFilter,
    ModelsPicked,
    ModelsLoaded { provider: AiProvider, result: Result<Vec<ModelInfo>> },
    PaddleDownload,
    PaddleDownloadProgress { done: u64, total: u64 },
    PaddleDownloadDone { result: Result<()> },
}

/// Queues an action for the UI thread. Safe to call from any thread.
pub fn post(action: Action) {
    let _ = slint::invoke_from_event_loop(move || dispatch(action));
}

/// Handles an action at once, from a Slint callback on the UI thread, so its effect is
/// in the next frame (or shortly after, when another action is running).
pub fn handle_now(action: Action) {
    dispatch(action);
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn dispatch(action: Action) {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => {
            if let Some(app) = app.as_mut() {
                app.handle(action);
            }
        }
        // Reached from a nested message loop while an action is running: retry shortly.
        Err(_) => slint::Timer::single_shot(Duration::from_millis(20), move || dispatch(action)),
    });
}

/// Shows a message box outside of action handling, so its modal loop can't nest
/// inside a borrowed `App`.
fn notify(message: String) {
    slint::Timer::single_shot(Duration::ZERO, move || win32::message_box(&message));
}

enum State {
    Idle,
    Selecting(overlay::Session),
    Result(popup::Session),
}

struct App {
    config: Config,
    hotkeys: GlobalHotKeyManager,
    registered: Option<HotKey>,
    tray: Tray,
    state: State,
    settings: Option<Settings>,
    /// Identifies the current recognition; results of older ones are dropped.
    job: u64,
}

pub fn run() -> Result<()> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(window_role::attributes_hook)
        .select()?;

    let config = config::load();
    // No open/close animation for Glance's windows: the overlay must appear at once.
    win32::disable_window_animations();
    let hotkeys = GlobalHotKeyManager::new().map_err(|e| Error::new(format!("Global shortcuts are unavailable: {e}")))?;
    let registered = hotkey::parse(&config.hotkey).filter(|hk| hotkeys.register(*hk).is_ok());
    if registered.is_none() {
        notify(strings::hotkey_taken(&config.hotkey));
    }
    GlobalHotKeyEvent::set_event_handler(Some(|event: GlobalHotKeyEvent| {
        if event.state() == HotKeyState::Pressed {
            post(Action::Capture);
        }
    }));
    let tray = Tray::create(&config.hotkey)?;

    APP.with(|cell| {
        *cell.borrow_mut() =
            Some(App { config, hotkeys, registered, tray, state: State::Idle, settings: None, job: 0 });
    });
    // A tray app starts invisibly; on the first launch (or with `--settings`) show the
    // settings so the user sees it running and can pick a shortcut and engines.
    if !config::exists() || std::env::args().any(|arg| arg == "--settings") {
        post(Action::OpenSettings);
    }
    schedule_trim();

    let result = slint::run_event_loop_until_quit();
    // Tear down windows, the tray icon and the hotkey before the process exits.
    APP.with(|cell| drop(cell.borrow_mut().take()));
    result.map_err(Error::from)
}

/// Hands memory freed by the last operation back to Windows once things settle.
fn schedule_trim() {
    slint::Timer::single_shot(Duration::from_millis(600), win32::trim_memory);
}

impl App {
    fn handle(&mut self, action: Action) {
        match action {
            Action::Capture => self.start_capture(),
            Action::CaptureAfterMenu => {
                slint::Timer::single_shot(Duration::from_millis(200), || post(Action::Capture));
            }
            Action::OpenSettings => self.open_settings(),
            Action::Quit => {
                self.set_idle();
                if let Some(settings) = self.settings.take() {
                    settings.close();
                }
                let _ = slint::quit_event_loop();
            }
            Action::OverlayCancel => {
                if matches!(self.state, State::Selecting(_)) {
                    self.set_idle();
                }
            }
            Action::OverlaySelected { screen, x, y, w, h } => self.finish_selection(screen, x, y, w, h),
            Action::OcrDone { job, result } => {
                if job == self.job {
                    self.on_recognized(result);
                }
            }
            Action::TranslationDone { job, result } => {
                if job == self.job
                    && let State::Result(popup) = &mut self.state
                {
                    popup.show_translation(result);
                }
            }
            Action::PopupClose | Action::PopupFocusLost => {
                if matches!(self.state, State::Result(_)) {
                    self.set_idle();
                }
            }
            Action::PopupCopy => {
                if let State::Result(popup) = &self.state
                    && win32::set_clipboard_text(popup.displayed_text()).is_ok()
                {
                    popup.show_copied();
                }
            }
            Action::PopupTranslate => self.toggle_translation(),
            Action::PopupOpenLink => {
                if let State::Result(popup) = &self.state
                    && let Some(link) = popup.link()
                    && let Err(e) = links::open(link)
                {
                    notify(e.to_string());
                }
            }
            Action::SettingsPoll => self.poll_settings(),
            Action::SettingsClose => self.close_settings(),
            Action::HotkeyRecording(recording) => self.pause_hotkey(recording),
            Action::ModelsContext => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.update_picker();
                }
            }
            Action::ModelsRefresh => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.refresh_models();
                }
            }
            Action::ModelsFilter => {
                if let Some(settings) = &self.settings {
                    settings.filter_models();
                }
            }
            Action::ModelsPicked => {
                if let Some(settings) = &self.settings {
                    settings.model_picked();
                }
            }
            Action::ModelsLoaded { provider, result } => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.models_loaded(provider, result);
                }
            }
            Action::PaddleDownload => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.start_paddle_download();
                }
            }
            Action::PaddleDownloadProgress { done, total } => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.paddle_progress(done, total);
                }
            }
            Action::PaddleDownloadDone { result } => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.paddle_download_done(result);
                }
            }
        }
    }

    /// Closes the overlay or popup, invalidates running jobs and frees their memory.
    fn set_idle(&mut self) {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => return,
            State::Selecting(session) => session.close(),
            State::Result(popup) => popup.close(),
        }
        self.job += 1;
        schedule_trim();
    }

    fn start_capture(&mut self) {
        match self.state {
            State::Selecting(_) => return,
            State::Result(_) => {
                self.set_idle();
                // Make sure the closed popup is gone from the screen before freezing it.
                win32::wait_for_compositor();
            }
            State::Idle => {}
        }
        match overlay::Session::open() {
            Ok(session) => self.state = State::Selecting(session),
            Err(e) => {
                schedule_trim();
                notify(strings::capture_failed(&e.to_string()));
            }
        }
    }

    fn finish_selection(&mut self, screen: usize, x: f32, y: f32, w: f32, h: f32) {
        let State::Selecting(session) = std::mem::replace(&mut self.state, State::Idle) else { return };
        let Some(selection) = session.selection(screen, x, y, w, h) else {
            session.close();
            schedule_trim();
            return;
        };
        // The popup opens while the overlay still owns the foreground, so it can take
        // keyboard focus; then the overlay and its frames are released.
        let translation_label = self.translation_label();
        let popup = popup::Session::open(&selection, ocr::engine_label(&self.config.ocr), translation_label);
        session.close();
        schedule_trim();
        match popup {
            Ok(popup) => {
                self.job += 1;
                ocr::spawn(self.job, selection.image, self.config.ocr.clone());
                self.state = State::Result(popup);
            }
            Err(e) => notify(e.to_string()),
        }
    }

    fn on_recognized(&mut self, result: Result<String>) {
        let State::Result(popup) = &mut self.state else { return };
        match result {
            Ok(text) => {
                let text = text.trim_end().to_owned();
                let copied = self.config.auto_copy
                    && !text.trim().is_empty()
                    && win32::set_clipboard_text(&text).is_ok();
                popup.show_text(text, copied);
                if self.config.translation.auto && popup.has_text() {
                    self.toggle_translation();
                }
            }
            Err(e) => popup.show_error(&e),
        }
    }

    fn toggle_translation(&mut self) {
        let State::Result(popup) = &mut self.state else { return };
        if popup.toggle_translation() {
            translate::spawn(self.job, popup.original().to_owned(), self.config.translation.clone());
        }
    }

    fn translation_label(&self) -> String {
        let target = languages::find(&self.config.translation.target).map_or("English", |l| l.name);
        format!("{} · {target}", self.config.translation.provider.name())
    }

    fn open_settings(&mut self) {
        if let Some(settings) = &self.settings {
            settings.bring_to_front();
            return;
        }
        match Settings::open(&self.config) {
            Ok(settings) => self.settings = Some(settings),
            Err(e) => notify(e.to_string()),
        }
    }

    fn close_settings(&mut self) {
        // Whatever was typed last still gets saved.
        self.poll_settings();
        if let Some(settings) = self.settings.take() {
            settings.close();
            schedule_trim();
        }
        // In case the window closed while the shortcut field was recording.
        self.pause_hotkey(false);
    }

    /// The shortcut field needs the current combination to reach it, so the global
    /// hotkey is released while it records.
    fn pause_hotkey(&mut self, recording: bool) {
        if recording {
            if let Some(hk) = self.registered.take() {
                let _ = self.hotkeys.unregister(hk);
            }
        } else if self.registered.is_none()
            && let Some(hk) = hotkey::parse(&self.config.hotkey)
            && self.hotkeys.register(hk).is_ok()
        {
            self.registered = Some(hk);
        }
    }

    /// Applies whatever changed in the open settings window since the last poll.
    /// Settings save themselves: there is no Save button.
    fn poll_settings(&mut self) {
        if let Some(settings) = self.settings.as_ref() {
            // A new Paddle language changes what counts as downloaded.
            settings.update_paddle();
        }
        let Some((now, keys)) = self.settings.as_ref().map(|s| {
            let now = s.read(&self.config);
            let keys = settings::key_changes(&s.applied().keys, &now.keys);
            (now, keys)
        }) else {
            return;
        };
        if self.settings.as_ref().is_some_and(|s| *s.applied() == now) {
            return;
        }
        let (config, errors) = self.apply(now.config.clone(), &keys, now.autostart);
        self.config = config;
        self.tray.set_hotkey(&self.config.hotkey);
        if let Some(settings) = self.settings.as_mut() {
            settings.set_error(&errors.join(" "));
            settings.mark_applied(now);
        }
    }

    /// Applies new settings; returns the config actually in effect and any problems.
    fn apply(&mut self, mut new: Config, keys: &[KeyChange], autostart: bool) -> (Config, Vec<String>) {
        let mut errors = Vec::new();
        match hotkey::parse(&new.hotkey) {
            None => {
                errors.push(strings::hotkey_invalid(&new.hotkey));
                new.hotkey = self.config.hotkey.clone();
            }
            Some(hk) if self.registered != Some(hk) => {
                if let Some(old) = self.registered.take() {
                    let _ = self.hotkeys.unregister(old);
                }
                if self.hotkeys.register(hk).is_ok() {
                    self.registered = Some(hk);
                } else {
                    errors.push(strings::hotkey_taken(&new.hotkey));
                    new.hotkey = self.config.hotkey.clone();
                    // Keep the previous shortcut working.
                    if let Some(old) = hotkey::parse(&new.hotkey)
                        && self.hotkeys.register(old).is_ok()
                    {
                        self.registered = Some(old);
                    }
                }
            }
            Some(_) => {}
        }
        for change in keys {
            let result = match change {
                KeyChange::Store(secret, value) => secrets::store(*secret, value),
                KeyChange::Delete(secret) => secrets::delete(*secret),
            };
            if let Err(e) = result {
                errors.push(e.to_string());
            }
        }
        if autostart != win32::autostart_enabled()
            && let Err(e) = win32::set_autostart(autostart)
        {
            errors.push(e.to_string());
        }
        if let Err(e) = config::save(&new) {
            errors.push(e.to_string());
        }
        (new, errors)
    }
}
