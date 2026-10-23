//! User-facing text produced on the Rust side (tray menu, popup status, errors).
//! Kept in one place so a Russian translation can be added next to the Slint
//! `@tr()` strings later.

pub const TRAY_CAPTURE: &str = "Capture text";
pub const TRAY_SETTINGS: &str = "Settings…";
pub const TRAY_QUIT: &str = "Quit Glance";

pub const ALREADY_RUNNING: &str = "Glance is already running. You can find it in the system tray.";

pub const COPIED: &str = "Copied to clipboard";
pub const HOTKEY_NEEDS_MODIFIER: &str = "Add Ctrl, Alt, Shift or Win";
pub const HOTKEY_IN_USE: &str = "Used by another app";

pub fn tray_tooltip(hotkey: &str) -> String {
    format!("Glance — {hotkey} to capture text")
}

pub fn startup_failed(error: &str) -> String {
    format!("Glance could not start.\n\n{error}")
}

pub fn hotkey_invalid(hotkey: &str) -> String {
    format!("“{hotkey}” is not a shortcut Glance can use.")
}

pub fn hotkey_taken(hotkey: &str) -> String {
    format!("{hotkey} is already used by Windows or another app. Choose a different shortcut.")
}

pub fn capture_failed(error: &str) -> String {
    format!("Could not capture the screen: {error}")
}

pub fn missing_key(provider: &str) -> String {
    format!("No API key for {provider}. Add one in Settings → API Keys.")
}

pub fn ocr_language_missing(language: &str, tag: &str) -> String {
    format!(
        "The Windows OCR language “{language}” is not installed. Install it from an administrator PowerShell:\n\
         Add-WindowsCapability -Online -Name Language.OCR~~~{tag}~0.0.1.0\n\
         Or pick another language in Glance Settings → Recognition."
    )
}

pub fn ocr_no_languages(locale: &str) -> String {
    format!(
        "Windows OCR has no languages installed. Install one from an administrator PowerShell:\n\
         Add-WindowsCapability -Online -Name Language.OCR~~~{locale}~0.0.1.0\n\
         Or choose a cloud engine in Glance Settings → Recognition."
    )
}

/// Locale tag for the OCR capability name, e.g. "ru-RU".
pub fn ocr_capability_locale() -> String {
    let locale = crate::win32::user_locale();
    if locale.is_empty() { "en-US".to_owned() } else { locale }
}

pub fn deepl_unsupported(language: &str) -> String {
    format!("DeepL can't translate into {language}. Choose another language or provider in Settings → Translation.")
}

pub const AUTOMATIC_LANGUAGE: &str = "Automatic";

pub const MODELS_LOADING: &str = "Loading models…";
pub const MODELS_NO_MATCH: &str = "No models match.";
pub const MODELS_EMPTY: &str = "The provider returned no models.";

pub fn models_need_key(provider: &str) -> String {
    format!("Add a {provider} key on the API Keys page to load its models.")
}

pub fn models_failed(provider: &str) -> String {
    format!("Could not load the {provider} models.")
}

/// The "Selected:" line of a model picker; an empty model setting means the default.
pub fn selected_model(name: &str, is_default: bool) -> String {
    if is_default { format!("{name} (default)") } else { name.to_owned() }
}
