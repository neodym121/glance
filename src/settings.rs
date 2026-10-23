//! The settings window: created on demand, destroyed on close. Changes are read back by
//! a poll and applied as they are made (there is no Save button).

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::window::WindowButtons;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{self, Action};
use crate::config::{Config, Models, Routes};
use crate::error::Result;
use crate::languages::{self, LANGUAGES};
use crate::models::{self, Catalog, ModelInfo, Purpose};
use crate::providers::{AiProvider, OcrProvider, Secret, TranslationProvider};
use crate::window_role::{self, Role};
use crate::{EditKeys, HotkeyCapture, ModelItem, ModelList, SettingsWindow, hotkey, ocr, secrets, strings, win32};

pub enum KeyChange {
    Store(Secret, String),
    Delete(Secret),
}

/// Everything the window edits, as last applied or as it is now.
#[derive(Clone, PartialEq)]
pub struct Snapshot {
    pub config: Config,
    /// Per key: the text in its field. Fields start with the stored keys.
    pub keys: Vec<(Secret, String)>,
    pub autostart: bool,
}

/// Key writes needed to go from `before` to `now`; a cleared field removes the key.
pub fn key_changes(before: &[(Secret, String)], now: &[(Secret, String)]) -> Vec<KeyChange> {
    before
        .iter()
        .zip(now)
        .filter_map(|((_, old_value), (secret, value))| {
            if value == old_value {
                None
            } else if value.is_empty() {
                Some(KeyChange::Delete(*secret))
            } else {
                Some(KeyChange::Store(*secret, value.clone()))
            }
        })
        .collect()
}

/// Where the model list scrolls after an update.
#[derive(Clone, Copy, PartialEq)]
enum Scroll {
    Keep,
    Top,
    Selected,
}

pub struct Settings {
    window: SettingsWindow,
    /// BCP-47 tags behind the OCR language list; "" is Automatic.
    ocr_tags: Vec<String>,
    applied: Snapshot,
    /// Model lists per provider: the cache file, refreshed by fetches.
    catalog: Catalog,
    loading: Vec<AiProvider>,
    model_errors: BTreeMap<AiProvider, String>,
    /// The model picker on screen, if any.
    picker: Option<(AiProvider, Purpose)>,
    /// Polls the window for changes while it is open (settings save themselves).
    _poll: slint::Timer,
}

fn set_key(window: &SettingsWindow, secret: Secret, value: SharedString) {
    match secret {
        Secret::OpenRouter => window.set_key_openrouter(value),
        Secret::Gemini => window.set_key_gemini(value),
        Secret::Mistral => window.set_key_mistral(value),
        Secret::Groq => window.set_key_groq(value),
        Secret::Polza => window.set_key_polza(value),
        Secret::Google => window.set_key_google(value),
        Secret::DeepL => window.set_key_deepl(value),
        Secret::Microsoft => window.set_key_microsoft(value),
    }
}

fn key(window: &SettingsWindow, secret: Secret) -> SharedString {
    match secret {
        Secret::OpenRouter => window.get_key_openrouter(),
        Secret::Gemini => window.get_key_gemini(),
        Secret::Mistral => window.get_key_mistral(),
        Secret::Groq => window.get_key_groq(),
        Secret::Polza => window.get_key_polza(),
        Secret::Google => window.get_key_google(),
        Secret::DeepL => window.get_key_deepl(),
        Secret::Microsoft => window.get_key_microsoft(),
    }
}

/// The model setting behind a picker ("" means the default) and that default.
fn model_setting(window: &SettingsWindow, provider: AiProvider, purpose: Purpose) -> (SharedString, &'static str) {
    match purpose {
        Purpose::Recognition => (
            match provider {
                AiProvider::OpenRouter => window.get_ocr_model_openrouter(),
                AiProvider::Gemini => window.get_ocr_model_gemini(),
                AiProvider::Mistral => window.get_ocr_model_mistral(),
                AiProvider::Groq => window.get_ocr_model_groq(),
                AiProvider::Polza => window.get_ocr_model_polza(),
            },
            provider.default_vision_model(),
        ),
        Purpose::Translation => (
            match provider {
                AiProvider::OpenRouter => window.get_tr_model_openrouter(),
                AiProvider::Gemini => window.get_tr_model_gemini(),
                AiProvider::Mistral => window.get_tr_model_mistral(),
                AiProvider::Groq => window.get_tr_model_groq(),
                AiProvider::Polza => window.get_tr_model_polza(),
            },
            provider.default_text_model(),
        ),
    }
}

fn string_model(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>())))
}

/// The OCR language worth suggesting: the user's own, else English; `None` when both
/// are already installed.
fn suggest_ocr_language(locale: &str, installed: &[&str]) -> Option<String> {
    let primary = |tag: &str| tag.split('-').next().unwrap_or("").to_ascii_lowercase();
    [locale, "en-US"]
        .into_iter()
        .find(|candidate| !installed.iter().any(|tag| primary(tag) == primary(candidate)))
        .map(str::to_owned)
}

/// The window has size limits and no maximize button, but Slint enables maximizing for
/// every resizable window once the native window exists; this takes it away again.
fn disable_maximize(window: slint::Weak<SettingsWindow>, attempts: u32) {
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(strong) = window.upgrade() else { return };
        let done = strong
            .window()
            .with_winit_window(|w| w.set_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE))
            .is_some();
        if !done && attempts > 0 {
            disable_maximize(window, attempts - 1);
        }
    });
}

impl Settings {
    pub fn open(config: &Config) -> Result<Self> {
        let window = window_role::create(Role::Settings, SettingsWindow::new)?;

        // General
        window.set_hotkey(config.hotkey.as_str().into());
        window.set_auto_copy(config.auto_copy);
        window.set_autostart(win32::autostart_enabled());

        // Recognition
        window.set_ocr_providers(string_model(OcrProvider::ALL.iter().map(|p| p.name().to_owned())));
        let provider_index = OcrProvider::ALL.iter().position(|p| *p == config.ocr.provider).unwrap_or(0);
        window.set_ocr_provider(provider_index as i32);
        let installed = ocr::available_languages();
        window.set_ocr_languages_missing(installed.is_empty());
        let installed_tags: Vec<&str> = installed.iter().map(|(tag, _)| tag.as_str()).collect();
        let suggestion = suggest_ocr_language(&strings::ocr_capability_locale(), &installed_tags);
        window.set_ocr_capability(suggestion.map(|tag| format!("Language.OCR~~~{tag}~0.0.1.0")).unwrap_or_default().into());
        let mut ocr_tags = vec![String::new()];
        let mut labels = vec![strings::AUTOMATIC_LANGUAGE.to_owned()];
        for (tag, name) in installed {
            ocr_tags.push(tag);
            labels.push(name);
        }
        let configured = config.ocr.language.trim();
        if !configured.is_empty() && !ocr_tags.iter().any(|t| t.eq_ignore_ascii_case(configured)) {
            // Keep a language that was uninstalled meanwhile visible instead of silently switching.
            ocr_tags.push(configured.to_owned());
            labels.push(format!("{configured} (not installed)"));
        }
        let ocr_index = ocr_tags.iter().position(|t| t.eq_ignore_ascii_case(configured)).unwrap_or(0);
        window.set_ocr_languages(string_model(labels));
        window.set_ocr_language_index(ocr_index as i32);
        let models = &config.ocr.models;
        window.set_ocr_model_openrouter(models.openrouter.as_str().into());
        window.set_ocr_default_openrouter(AiProvider::OpenRouter.default_vision_model().into());
        window.set_ocr_model_gemini(models.gemini.as_str().into());
        window.set_ocr_default_gemini(AiProvider::Gemini.default_vision_model().into());
        window.set_ocr_model_mistral(models.mistral.as_str().into());
        window.set_ocr_default_mistral(AiProvider::Mistral.default_vision_model().into());
        window.set_ocr_model_groq(models.groq.as_str().into());
        window.set_ocr_default_groq(AiProvider::Groq.default_vision_model().into());
        window.set_ocr_model_polza(models.polza.as_str().into());
        window.set_ocr_default_polza(AiProvider::Polza.default_vision_model().into());
        window.set_ocr_route_openrouter(config.ocr.routes.openrouter.as_str().into());
        window.set_ocr_route_polza(config.ocr.routes.polza.as_str().into());

        // Translation
        let translation = &config.translation;
        window.set_translation_providers(string_model(TranslationProvider::ALL.iter().map(|p| p.name().to_owned())));
        let provider_index = TranslationProvider::ALL.iter().position(|p| *p == translation.provider).unwrap_or(0);
        window.set_translation_provider(provider_index as i32);
        window.set_target_languages(string_model(LANGUAGES.iter().map(|l| l.name.to_owned())));
        let target = languages::index_of(&translation.target).or_else(|| languages::index_of("en")).unwrap_or(0);
        window.set_target_language_index(target as i32);
        window.set_auto_translate(translation.auto);
        window.set_tr_model_openrouter(translation.models.openrouter.as_str().into());
        window.set_tr_default_openrouter(AiProvider::OpenRouter.default_text_model().into());
        window.set_tr_model_gemini(translation.models.gemini.as_str().into());
        window.set_tr_default_gemini(AiProvider::Gemini.default_text_model().into());
        window.set_tr_model_mistral(translation.models.mistral.as_str().into());
        window.set_tr_default_mistral(AiProvider::Mistral.default_text_model().into());
        window.set_tr_model_groq(translation.models.groq.as_str().into());
        window.set_tr_default_groq(AiProvider::Groq.default_text_model().into());
        window.set_tr_model_polza(translation.models.polza.as_str().into());
        window.set_tr_default_polza(AiProvider::Polza.default_text_model().into());
        window.set_tr_route_openrouter(translation.routes.openrouter.as_str().into());
        window.set_tr_route_polza(translation.routes.polza.as_str().into());
        window.set_microsoft_region(translation.microsoft_region.as_str().into());

        // Keys: the stored ones fill their (masked) fields while the window is open.
        for &secret in &Secret::ALL {
            set_key(&window, secret, secrets::load(secret).unwrap_or_default().into());
        }
        window.set_version(env!("CARGO_PKG_VERSION").into());

        window.global::<EditKeys>().on_action(|text, ctrl, shift| hotkey::edit_action(&text, ctrl, shift).into());
        window.on_close_clicked(|| app::post(Action::SettingsClose));
        window.on_hotkey_recording(|recording| app::post(Action::HotkeyRecording(recording)));
        window.on_capture_hotkey(|text, ctrl, alt, shift, win, released| {
            let capture = hotkey::capture(&text, ctrl, alt, shift, win, released);
            HotkeyCapture { text: capture.text.into(), complete: capture.complete, is_key: capture.is_key }
        });
        // Right away, so a page switch shows its model list in the same frame.
        window.on_models_context(|| app::handle_now(Action::ModelsContext));
        let list = window.global::<ModelList>();
        list.on_refresh(|| app::post(Action::ModelsRefresh));
        list.on_filter(|| app::post(Action::ModelsFilter));
        list.on_picked(|| app::post(Action::ModelsPicked));
        window.window().on_close_requested(|| {
            app::post(Action::SettingsClose);
            slint::CloseRequestResponse::KeepWindowShown
        });

        window.show()?;
        disable_maximize(window.as_weak(), 50);
        let poll = slint::Timer::default();
        poll.start(slint::TimerMode::Repeated, Duration::from_millis(400), || app::post(Action::SettingsPoll));
        let mut settings = Self {
            window,
            ocr_tags,
            applied: Snapshot { config: config.clone(), keys: Vec::new(), autostart: false },
            catalog: models::load_cache(),
            loading: Vec::new(),
            model_errors: BTreeMap::new(),
            picker: None,
            _poll: poll,
        };
        settings.applied = settings.read(config);
        settings.update_picker();
        Ok(settings)
    }

    pub fn applied(&self) -> &Snapshot {
        &self.applied
    }

    pub fn mark_applied(&mut self, now: Snapshot) {
        self.applied = now;
    }

    /// The settings as they are in the window, on top of `base`.
    pub fn read(&self, base: &Config) -> Snapshot {
        let w = &self.window;
        let text = |s: SharedString| s.trim().to_owned();
        let mut config = base.clone();

        config.hotkey = text(w.get_hotkey());
        config.auto_copy = w.get_auto_copy();

        config.ocr.provider = OcrProvider::ALL.get(w.get_ocr_provider() as usize).copied().unwrap_or_default();
        config.ocr.language = self.ocr_tags.get(w.get_ocr_language_index() as usize).cloned().unwrap_or_default();
        config.ocr.models = Models {
            openrouter: text(w.get_ocr_model_openrouter()),
            gemini: text(w.get_ocr_model_gemini()),
            mistral: text(w.get_ocr_model_mistral()),
            groq: text(w.get_ocr_model_groq()),
            polza: text(w.get_ocr_model_polza()),
        };
        config.ocr.routes =
            Routes { openrouter: text(w.get_ocr_route_openrouter()), polza: text(w.get_ocr_route_polza()) };

        let translation = &mut config.translation;
        translation.provider =
            TranslationProvider::ALL.get(w.get_translation_provider() as usize).copied().unwrap_or_default();
        if let Some(language) = LANGUAGES.get(w.get_target_language_index() as usize) {
            translation.target = language.code.to_owned();
        }
        translation.auto = w.get_auto_translate();
        translation.models = Models {
            openrouter: text(w.get_tr_model_openrouter()),
            gemini: text(w.get_tr_model_gemini()),
            mistral: text(w.get_tr_model_mistral()),
            groq: text(w.get_tr_model_groq()),
            polza: text(w.get_tr_model_polza()),
        };
        translation.routes =
            Routes { openrouter: text(w.get_tr_route_openrouter()), polza: text(w.get_tr_route_polza()) };
        translation.microsoft_region = text(w.get_microsoft_region());

        let keys = Secret::ALL.iter().map(|&secret| (secret, text(key(w, secret)))).collect();
        Snapshot { config, keys, autostart: w.get_autostart() }
    }

    /// The model picker on screen: the Recognition or Translation page with an AI provider.
    fn picker_on_screen(&self) -> Option<(AiProvider, Purpose)> {
        let w = &self.window;
        match w.get_page() {
            1 => OcrProvider::ALL.get(w.get_ocr_provider() as usize)?.ai().map(|p| (p, Purpose::Recognition)),
            2 => TranslationProvider::ALL
                .get(w.get_translation_provider() as usize)?
                .ai()
                .map(|p| (p, Purpose::Translation)),
            _ => None,
        }
    }

    /// The page or a provider changed: shows the list of the picker now on screen,
    /// fetching it the first time it is needed.
    pub fn update_picker(&mut self) {
        let picker = self.picker_on_screen();
        if picker == self.picker {
            return;
        }
        self.picker = picker;
        self.window.global::<ModelList>().set_query(SharedString::new());
        if let Some((provider, _)) = picker
            && !self.catalog.contains_key(&provider)
        {
            self.fetch_models(provider);
        }
        self.show_models(Scroll::Selected);
    }

    pub fn refresh_models(&mut self) {
        if let Some((provider, _)) = self.picker {
            self.fetch_models(provider);
            self.show_models(Scroll::Keep);
        }
    }

    pub fn filter_models(&self) {
        let empty = self.window.global::<ModelList>().get_query().trim().is_empty();
        self.show_models(if empty { Scroll::Selected } else { Scroll::Top });
    }

    /// A model was clicked: the list stays as it is, only the "Selected:" line changes.
    pub fn model_picked(&self) {
        if let Some((provider, purpose)) = self.picker {
            self.window.global::<ModelList>().set_selected(self.selected_label(provider, purpose).into());
        }
    }

    pub fn models_loaded(&mut self, provider: AiProvider, result: Result<Vec<ModelInfo>>) {
        self.loading.retain(|p| *p != provider);
        match result {
            Ok(list) => {
                self.model_errors.remove(&provider);
                self.catalog.insert(provider, list);
                // Only a cache: if it can't be written, the list is fetched again next time.
                let _ = models::save_cache(&self.catalog);
            }
            Err(e) => {
                self.model_errors.insert(provider, e.to_string());
            }
        }
        if self.picker.is_some_and(|(p, _)| p == provider) {
            self.show_models(Scroll::Selected);
        }
    }

    fn fetch_models(&mut self, provider: AiProvider) {
        if self.loading.contains(&provider) {
            return;
        }
        self.model_errors.remove(&provider);
        self.loading.push(provider);
        models::spawn(provider);
    }

    /// The model in use for a picker (its setting, or the default when that is empty).
    fn current_model(&self, provider: AiProvider, purpose: Purpose) -> (String, bool) {
        let (setting, default) = model_setting(&self.window, provider, purpose);
        match setting.trim() {
            "" => (default.to_owned(), true),
            id => (id.to_owned(), false),
        }
    }

    fn selected_label(&self, provider: AiProvider, purpose: Purpose) -> String {
        let (id, is_default) = self.current_model(provider, purpose);
        let known = self.catalog.get(&provider).and_then(|all| all.iter().find(|m| m.id == id));
        strings::selected_model(known.map_or(id.as_str(), |m| m.name.as_str()), is_default)
    }

    /// Fills the picker's list: the provider's models matching the search, plus the
    /// search text itself as a model ID when it is not in the list.
    fn show_models(&self, scroll: Scroll) {
        let list = self.window.global::<ModelList>();
        let Some((provider, purpose)) = self.picker else {
            list.set_items(ModelRc::default());
            return;
        };
        let query = list.get_query();
        let known = self.catalog.get(&provider);
        let found = known.map(|all| models::search(all, &query, purpose)).unwrap_or_default();
        let mut items: Vec<ModelItem> = found
            .iter()
            .map(|m| ModelItem { id: m.id.as_str().into(), name: m.name.as_str().into(), custom: false })
            .collect();
        let typed = query.trim();
        if !typed.is_empty() && !known.is_some_and(|all| all.iter().any(|m| m.id == typed)) {
            items.push(ModelItem { id: typed.into(), name: typed.into(), custom: true });
        }

        let loading = self.loading.contains(&provider);
        let error = self.model_errors.get(&provider);
        let status = match (error, known) {
            (Some(error), _) => error.as_str(),
            (None, None) if loading => strings::MODELS_LOADING,
            (None, Some(all)) if all.is_empty() => strings::MODELS_EMPTY,
            (None, Some(_)) if found.is_empty() => strings::MODELS_NO_MATCH,
            _ => "",
        };

        let (current, _) = self.current_model(provider, purpose);
        let selected_row = items.iter().position(|item| !item.custom && item.id == current.as_str());
        list.set_items(ModelRc::new(VecModel::from(items)));
        list.set_status(status.into());
        list.set_status_is_error(error.is_some());
        list.set_loading(loading);
        list.set_selected(self.selected_label(provider, purpose).into());
        list.set_reveal_index(selected_row.map_or(-1, |row| row as i32));
        match scroll {
            Scroll::Keep => {}
            Scroll::Top => {
                list.set_reveal_index(-1);
                list.set_reveal_request(list.get_reveal_request() + 1);
            }
            Scroll::Selected => list.set_reveal_request(list.get_reveal_request() + 1),
        }
    }

    pub fn set_error(&self, message: &str) {
        self.window.set_error_text(message.into());
    }

    pub fn bring_to_front(&self) {
        let _ = self.window.show();
        win32::bring_to_front(self.window.window());
    }

    pub fn close(self) {
        let _ = self.window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_changed_keys() {
        let before = vec![(Secret::Groq, String::new()), (Secret::DeepL, "old:fx".to_owned()), (Secret::Polza, "pz".to_owned())];
        let now = vec![(Secret::Groq, "gsk_1".to_owned()), (Secret::DeepL, String::new()), (Secret::Polza, "pz".to_owned())];
        let changes = key_changes(&before, &now);
        assert_eq!(changes.len(), 2);
        assert!(matches!(changes[0], KeyChange::Store(Secret::Groq, ref v) if v == "gsk_1"));
        assert!(matches!(changes[1], KeyChange::Delete(Secret::DeepL)));
        assert!(key_changes(&now, &now).is_empty());
    }

    #[test]
    fn suggests_a_missing_ocr_language() {
        assert_eq!(suggest_ocr_language("ru-RU", &[]).as_deref(), Some("ru-RU"));
        assert_eq!(suggest_ocr_language("ru-RU", &["ru"]).as_deref(), Some("en-US"));
        assert_eq!(suggest_ocr_language("ru-RU", &["ru", "en-US"]), None);
        assert_eq!(suggest_ocr_language("en-GB", &["en-US"]), None);
    }
}
