//! Native window attributes per kind of Slint window. Slint asks the hook for winit
//! attributes when a component is instantiated, so the role is set right before
//! `Component::new()` on the UI thread.

use std::cell::Cell;

use slint::winit_030::winit;
use winit::platform::windows::{Color, CornerPreference, IconExtWindows, WindowAttributesExtWindows};
use winit::window::{Icon, Theme, WindowAttributes};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Default,
    Overlay,
    Popup,
    Settings,
}

thread_local! {
    static NEXT: Cell<Role> = const { Cell::new(Role::Default) };
}

/// Creates a window with the given role.
pub fn create<T>(role: Role, create: impl FnOnce() -> T) -> T {
    NEXT.set(role);
    let window = create();
    NEXT.set(Role::Default);
    window
}

/// Resource ordinal of the app icon (assets/glance.rc).
const APP_ICON: u16 = 1;

pub fn attributes_hook(attributes: WindowAttributes) -> WindowAttributes {
    match NEXT.get() {
        Role::Default => attributes,
        Role::Overlay => attributes
            .with_transparent(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_border_color(None)
            .with_corner_preference(CornerPreference::DoNotRound),
        Role::Popup => attributes
            .with_transparent(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_skip_taskbar(true)
            // Native rounded corners and drop shadow on Windows 11; the rim is drawn by
            // the popup itself instead of the accent-colored system border.
            .with_undecorated_shadow(true)
            .with_border_color(None)
            .with_corner_preference(CornerPreference::RoundSmall),
        Role::Settings => {
            // The window draws its own title bar; keep the native shadow, snap and corners.
            attributes
                .with_transparent(false)
                .with_decorations(false)
                .with_undecorated_shadow(true)
                .with_theme(Some(Theme::Dark))
                .with_corner_preference(CornerPreference::Round)
                .with_border_color(Some(Color::from_rgb(0x24, 0x24, 0x24)))
                .with_window_icon(Icon::from_resource(APP_ICON, None).ok())
                .with_taskbar_icon(Icon::from_resource(APP_ICON, None).ok())
        }
    }
}
