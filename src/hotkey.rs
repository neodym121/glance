//! The global capture shortcut: parsing "Ctrl+Alt+S" and recording a new one.

use global_hotkey::hotkey::HotKey;

use crate::win32;

/// Keys a shortcut can end with: (virtual-key code, name). Names are what the
/// settings show and what `global_hotkey` parses. Virtual-key codes are layout
/// independent, so the recorder works the same with any keyboard layout active.
const KEYS: &[(u16, &str)] = &[
    (0x41, "A"), (0x42, "B"), (0x43, "C"), (0x44, "D"), (0x45, "E"), (0x46, "F"), (0x47, "G"),
    (0x48, "H"), (0x49, "I"), (0x4A, "J"), (0x4B, "K"), (0x4C, "L"), (0x4D, "M"), (0x4E, "N"),
    (0x4F, "O"), (0x50, "P"), (0x51, "Q"), (0x52, "R"), (0x53, "S"), (0x54, "T"), (0x55, "U"),
    (0x56, "V"), (0x57, "W"), (0x58, "X"), (0x59, "Y"), (0x5A, "Z"),
    (0x30, "0"), (0x31, "1"), (0x32, "2"), (0x33, "3"), (0x34, "4"),
    (0x35, "5"), (0x36, "6"), (0x37, "7"), (0x38, "8"), (0x39, "9"),
    (0x70, "F1"), (0x71, "F2"), (0x72, "F3"), (0x73, "F4"), (0x74, "F5"), (0x75, "F6"),
    (0x76, "F7"), (0x77, "F8"), (0x78, "F9"), (0x79, "F10"), (0x7A, "F11"), (0x7B, "F12"),
    (0x7C, "F13"), (0x7D, "F14"), (0x7E, "F15"), (0x7F, "F16"), (0x80, "F17"), (0x81, "F18"),
    (0x82, "F19"), (0x83, "F20"), (0x84, "F21"), (0x85, "F22"), (0x86, "F23"), (0x87, "F24"),
    (0x20, "Space"), (0x2D, "Insert"), (0x2E, "Delete"), (0x24, "Home"), (0x23, "End"),
    (0x21, "PageUp"), (0x22, "PageDown"), (0x25, "Left"), (0x26, "Up"), (0x27, "Right"),
    (0x28, "Down"), (0x2C, "PrintScreen"), (0x13, "Pause"),
    (0x60, "Num0"), (0x61, "Num1"), (0x62, "Num2"), (0x63, "Num3"), (0x64, "Num4"),
    (0x65, "Num5"), (0x66, "Num6"), (0x67, "Num7"), (0x68, "Num8"), (0x69, "Num9"),
    (0xBA, ";"), (0xBB, "="), (0xBC, ","), (0xBD, "-"), (0xBE, "."), (0xBF, "/"), (0xC0, "`"),
    (0xDB, "["), (0xDC, "\\"), (0xDD, "]"), (0xDE, "'"),
];

/// Keys that may be used on their own, without a modifier.
fn standalone(key: &str) -> bool {
    key == "PrintScreen" || key == "Pause" || (key.starts_with('F') && key.len() > 1 && key[1..].parse::<u8>().is_ok())
}

/// Parses a shortcut such as "Ctrl+Alt+S" or "Win+Shift+F2".
pub fn parse(text: &str) -> Option<HotKey> {
    let tokens: Vec<&str> = text.split('+').map(str::trim).collect();
    let key = *tokens.last()?;
    let has_modifier = tokens.len() > 1;
    if !has_modifier && !standalone(key) {
        return None;
    }
    let normalized: Vec<&str> = tokens
        .iter()
        .map(|t| if t.eq_ignore_ascii_case("win") { "Super" } else { t })
        .collect();
    normalized.join("+").parse().ok()
}

#[derive(Debug, PartialEq, Eq)]
pub struct Capture {
    pub text: String,
    pub complete: bool,
    /// The event was for a non-modifier key.
    pub is_key: bool,
}

/// Builds the recorder state from the pressed modifiers and key.
pub fn compose(ctrl: bool, alt: bool, shift: bool, win: bool, key: Option<&str>) -> Capture {
    let mut parts: Vec<&str> = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if win {
        parts.push("Win");
    }
    let (text, complete) = match key {
        Some(key) if !parts.is_empty() || standalone(key) => {
            parts.push(key);
            (parts.join("+"), true)
        }
        Some(_) => (crate::strings::HOTKEY_NEEDS_MODIFIER.to_owned(), false),
        None if parts.is_empty() => (String::new(), false),
        None => (format!("{}+…", parts.join("+")), false),
    };
    Capture { text, complete, is_key: key.is_some() }
}

/// Name of the key in a Slint key event, or `None` for modifiers and unsupported keys.
/// Characters are mapped to their physical key through the active keyboard layout,
/// so Ctrl+Alt+S records as "S" even while a Cyrillic layout types "ы".
pub fn key_name(text: &str) -> Option<&'static str> {
    let mut chars = text.chars();
    let (c, None) = (chars.next()?, chars.next()) else { return None };
    let name = |vk: u16| KEYS.iter().find(|(code, _)| *code == vk).map(|(_, name)| *name);
    match c {
        // Slint's codes for special keys (i-slint-common key_codes).
        '\u{F704}'..='\u{F71B}' => name(0x70 + (c as u16 - 0xF704)),
        '\u{F700}' => Some("Up"),
        '\u{F701}' => Some("Down"),
        '\u{F702}' => Some("Left"),
        '\u{F703}' => Some("Right"),
        '\u{F727}' => Some("Insert"),
        '\u{F729}' => Some("Home"),
        '\u{F72B}' => Some("End"),
        '\u{F72C}' => Some("PageUp"),
        '\u{F72D}' => Some("PageDown"),
        '\u{F730}' => Some("Pause"),
        '\u{F731}' => Some("PrintScreen"),
        '\u{007F}' => Some("Delete"),
        ' ' => Some("Space"),
        c if c.is_ascii_alphanumeric() => name(c.to_ascii_uppercase() as u16),
        // Modifiers, Escape, Tab, Enter and other control codes.
        c if c.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&c) => None,
        c => win32::virtual_key_for_char(c).and_then(name),
    }
}

/// Text-field shortcut for a key event, independent of the keyboard layout.
pub fn edit_action(text: &str, ctrl: bool, shift: bool) -> &'static str {
    match (ctrl, shift, key_name(text)) {
        (true, false, Some("V")) | (false, true, Some("Insert")) => "paste",
        (true, false, Some("C")) | (true, false, Some("Insert")) => "copy",
        (true, false, Some("X")) | (false, true, Some("Delete")) => "cut",
        (true, false, Some("A")) => "select-all",
        _ => "",
    }
}

/// Recorder state for a key event in the shortcut field. For a released key the text
/// is the "in use" notice: the field shows it when the press never arrived, which is
/// what happens when another app owns that global shortcut (Windows hands the press
/// to that app only).
pub fn capture(text: &str, ctrl: bool, alt: bool, shift: bool, win: bool, released: bool) -> Capture {
    match (released, key_name(text)) {
        (true, Some(_)) => Capture { text: crate::strings::HOTKEY_IN_USE.to_owned(), complete: false, is_key: true },
        (true, None) => compose(ctrl, alt, shift, win, None),
        (false, key) => compose(ctrl, alt, shift, win, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn parses_shortcuts() {
        let hk = parse("Ctrl+Alt+S").unwrap();
        assert_eq!(hk, HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyS));
        let hk = parse("Win+Shift+F2").unwrap();
        assert_eq!(hk, HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::F2));
        assert!(parse("PrintScreen").is_some());
        assert!(parse("F9").is_some());
        assert!(parse("S").is_none(), "a bare letter would swallow typing");
        assert!(parse("").is_none());
        assert!(parse("Ctrl+").is_none());
    }

    #[test]
    fn every_recordable_key_parses() {
        for (_, name) in KEYS {
            let text = format!("Ctrl+Alt+{name}");
            assert!(parse(&text).is_some(), "{text}");
        }
    }

    #[test]
    fn names_keys_from_slint_events() {
        assert_eq!(key_name("s"), Some("S"));
        assert_eq!(key_name("S"), Some("S"));
        assert_eq!(key_name("7"), Some("7"));
        assert_eq!(key_name("\u{F704}"), Some("F1"));
        assert_eq!(key_name("\u{F71B}"), Some("F24"));
        assert_eq!(key_name("\u{F72C}"), Some("PageUp"));
        assert_eq!(key_name(" "), Some("Space"));
        assert_eq!(key_name("\u{0011}"), None, "Control alone");
        assert_eq!(key_name("\u{001b}"), None, "Escape");
        assert_eq!(key_name(""), None);
        assert_eq!(key_name("ab"), None);
        assert_eq!(capture("x", true, false, true, false, false).text, "Ctrl+Shift+X");
        let released = capture("x", true, false, true, false, true);
        assert!(released.is_key && !released.complete);
        assert!(!capture("\u{0011}", true, false, false, false, true).is_key);
    }

    #[test]
    fn composes_recorder_state() {
        assert_eq!(
            compose(true, true, false, false, Some("S")),
            Capture { text: "Ctrl+Alt+S".into(), complete: true, is_key: true }
        );
        assert_eq!(compose(false, false, true, true, Some("F2")).text, "Shift+Win+F2");
        assert_eq!(compose(true, false, false, false, None), Capture { text: "Ctrl+…".into(), complete: false, is_key: false });
        assert!(!compose(false, false, false, false, Some("S")).complete);
        assert!(compose(false, false, false, false, Some("F8")).complete);
        assert_eq!(compose(false, false, false, false, None).text, "");
    }
}
