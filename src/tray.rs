//! Tray icon and its context menu (native Win32 through `tray-icon`/`muda`).

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::app::{self, Action};
use crate::error::{Error, Result};
use crate::{strings, win32};

/// Resource ordinals from assets/glance.rc.
const ICON_FOR_DARK_TASKBAR: u16 = 2;
const ICON_FOR_LIGHT_TASKBAR: u16 = 3;

pub struct Tray {
    icon: TrayIcon,
    capture: MenuItem,
}

impl Tray {
    pub fn create(hotkey: &str) -> Result<Self> {
        let capture = MenuItem::with_id("capture", capture_label(hotkey), true, None);
        let settings = MenuItem::with_id("settings", strings::TRAY_SETTINGS, true, None);
        let quit = MenuItem::with_id("quit", strings::TRAY_QUIT, true, None);
        let menu = Menu::with_items(&[&capture, &settings, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| Error::new(format!("Could not create the tray menu: {e}")))?;

        let ordinal = if win32::taskbar_is_light() { ICON_FOR_LIGHT_TASKBAR } else { ICON_FOR_DARK_TASKBAR };
        let icon = Icon::from_resource(ordinal, None)
            .map_err(|e| Error::new(format!("Could not load the tray icon: {e}")))?;

        let icon = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(strings::tray_tooltip(hotkey))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| Error::new(format!("Could not create the tray icon: {e}")))?;

        // Handlers run inside the tray window procedure; defer the work to the event loop.
        TrayIconEvent::set_event_handler(Some(|event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                app::post(Action::Capture);
            }
        }));
        MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
            // Let the menu fade out before the screen is frozen.
            "capture" => app::post(Action::CaptureAfterMenu),
            "settings" => app::post(Action::OpenSettings),
            "quit" => app::post(Action::Quit),
            _ => {}
        }));

        Ok(Self { icon, capture })
    }

    pub fn set_hotkey(&self, hotkey: &str) {
        self.capture.set_text(capture_label(hotkey));
        let _ = self.icon.set_tooltip(Some(strings::tray_tooltip(hotkey)));
    }
}

/// Win32 menus right-align the text after a tab, like an accelerator.
fn capture_label(hotkey: &str) -> String {
    format!("{}\t{hotkey}", strings::TRAY_CAPTURE)
}
