//! The result card under the selection. It shows the recognized text, toggles between
//! the original and the translation (translating once), and closes on Esc or when it
//! loses focus.

use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, PhysicalPosition};

use crate::app::{self, Action};
use crate::capture::PhysRect;
use crate::error::{Error, Result};
use crate::overlay::Selection;
use crate::window_role::{self, Role};
use crate::{EditKeys, PopupState, ResultPopup, links, strings};

const MIN_WIDTH: f32 = 360.0;
const MAX_WIDTH: f32 = 560.0;
const GAP: f32 = 8.0;

pub struct Session {
    window: ResultPopup,
    anchor: PhysRect,
    work_area: PhysRect,
    scale: f32,
    engine: String,
    original: String,
    translation: Option<String>,
    translation_label: String,
    link: Option<String>,
    showing_translation: bool,
    translating: bool,
}

impl Session {
    /// Shows the card in its "recognizing" state. Called while the overlay still has
    /// the foreground, so the card can take it over (Esc and click-outside need focus).
    pub fn open(selection: &Selection, engine: String, translation_label: String) -> Result<Self> {
        let window = window_role::create(Role::Popup, ResultPopup::new)?;
        let scale = selection.monitor.scale;
        window.set_popup_width((selection.bounds.width as f32 / scale).clamp(MIN_WIDTH, MAX_WIDTH));
        window.set_state(PopupState::Recognizing);
        window.set_status(engine.clone().into());

        window.global::<EditKeys>().on_action(|text, ctrl, shift| crate::hotkey::edit_action(&text, ctrl, shift).into());
        window.on_dismiss(|| app::post(Action::PopupClose));
        window.on_copy(|| app::post(Action::PopupCopy));
        window.on_translate(|| app::post(Action::PopupTranslate));
        window.on_open_link(|| app::post(Action::PopupOpenLink));
        window.window().on_close_requested(|| {
            app::post(Action::PopupClose);
            slint::CloseRequestResponse::KeepWindowShown
        });
        window.window().on_winit_window_event(|_, event| {
            if let winit::event::WindowEvent::Focused(false) = event {
                app::post(Action::PopupFocusLost);
            }
            EventResult::Propagate
        });

        let session = Self {
            window,
            anchor: selection.bounds,
            work_area: selection.monitor.work_area,
            scale,
            engine,
            original: String::new(),
            translation: None,
            translation_label,
            link: None,
            showing_translation: false,
            translating: false,
        };
        session.place();
        session.window.show()?;
        Ok(session)
    }

    pub fn show_text(&mut self, text: String, copied: bool) {
        self.link = links::find_link(&text);
        self.original = text;
        if self.original.trim().is_empty() {
            self.window.set_state(PopupState::Empty);
            self.window.set_actions_enabled(false);
        } else {
            self.window.set_state(PopupState::Text);
            self.window.set_text(self.original.as_str().into());
            self.window.set_actions_enabled(true);
            self.window.set_has_link(self.link.is_some());
        }
        if copied {
            self.show_copied();
        }
        self.place();
    }

    pub fn show_error(&mut self, error: &Error) {
        self.window.set_state(PopupState::Error);
        self.window.set_error(error.to_string().into());
        self.window.set_actions_enabled(false);
        self.place();
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn has_text(&self) -> bool {
        !self.original.trim().is_empty()
    }

    /// The text currently on display, for the Copy button.
    pub fn displayed_text(&self) -> &str {
        match (&self.translation, self.showing_translation) {
            (Some(translation), true) => translation,
            _ => &self.original,
        }
    }

    pub fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    pub fn show_copied(&self) {
        self.window.set_status(strings::COPIED.into());
        self.window.set_status_ok(true);
    }

    /// Switches between original and translation. Returns true when the translation
    /// still has to be requested; the result then arrives via `show_translation`.
    pub fn toggle_translation(&mut self) -> bool {
        if self.translating || !self.has_text() {
            return false;
        }
        if self.showing_translation {
            self.showing_translation = false;
            self.show_current();
            return false;
        }
        if self.translation.is_some() {
            self.showing_translation = true;
            self.show_current();
            return false;
        }
        self.translating = true;
        self.window.set_translating(true);
        true
    }

    pub fn show_translation(&mut self, result: Result<String>) {
        self.translating = false;
        self.window.set_translating(false);
        self.showing_translation = true;
        match result {
            Ok(text) => {
                self.translation = Some(text);
                self.show_current();
            }
            Err(error) => {
                // Stay on the "Original" side so one click returns to the text; the next
                // Translate retries.
                self.window.set_state(PopupState::Error);
                self.window.set_error(error.to_string().into());
                self.window.set_showing_translation(true);
                self.window.set_status(self.translation_label.as_str().into());
                self.window.set_status_ok(false);
                self.place();
            }
        }
    }

    fn show_current(&self) {
        self.window.set_state(PopupState::Text);
        self.window.set_text(self.displayed_text().into());
        self.window.set_showing_translation(self.showing_translation);
        self.window.set_status_ok(false);
        let status = if self.showing_translation { &self.translation_label } else { &self.engine };
        self.window.set_status(status.as_str().into());
        self.place();
    }

    /// Positions the card under the selection, or above it when there is no room.
    fn place(&self) {
        let width = (self.window.get_popup_width() * self.scale).round() as i32;
        let height = (self.window.get_desired_height() * self.scale).round() as i32;
        let gap = (GAP * self.scale).round() as i32;
        let (x, y) = place(self.anchor, self.work_area, width, height, gap);
        self.window.window().set_position(PhysicalPosition::new(x, y));
    }

    pub fn close(self) {
        let _ = self.window.hide();
    }
}

/// Top-left corner for a card of `width`×`height` next to `anchor` inside `work`.
fn place(anchor: PhysRect, work: PhysRect, width: i32, height: i32, gap: i32) -> (i32, i32) {
    let x = anchor.x.clamp(work.x, (work.right() - width).max(work.x));
    let below = anchor.bottom() + gap;
    let above = anchor.y - gap - height;
    let y = if below + height <= work.bottom() {
        below
    } else if above >= work.y {
        above
    } else {
        (work.bottom() - height).max(work.y)
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: PhysRect = PhysRect { x: 0, y: 0, width: 1920, height: 1040 };

    #[test]
    fn goes_below_the_selection_when_it_fits() {
        let anchor = PhysRect { x: 100, y: 100, width: 300, height: 50 };
        assert_eq!(place(anchor, WORK, 400, 200, 8), (100, 158));
    }

    #[test]
    fn flips_above_near_the_bottom() {
        let anchor = PhysRect { x: 100, y: 900, width: 300, height: 50 };
        assert_eq!(place(anchor, WORK, 400, 200, 8), (100, 692));
    }

    #[test]
    fn stays_inside_the_work_area() {
        let anchor = PhysRect { x: 1800, y: 0, width: 120, height: 1040 };
        let (x, y) = place(anchor, WORK, 400, 200, 8);
        assert_eq!(x, 1520);
        assert_eq!(y, 840);
        let second = PhysRect { x: 1920, y: 0, width: 1920, height: 1040 };
        let anchor = PhysRect { x: 1900, y: 10, width: 50, height: 20 };
        assert_eq!(place(anchor, second, 400, 200, 8).0, 1920);
    }
}
