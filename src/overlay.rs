//! The capture session: one frozen, dimmed full-screen window per monitor. Created on
//! the hotkey and destroyed as soon as the selection is taken or cancelled.

use slint::{ComponentHandle, Image, PhysicalPosition, PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

use crate::app::{self, Action};
use crate::capture::{self, Monitor, PhysRect};
use crate::error::Result;
use crate::imaging::RgbImage;
use crate::window_role::{self, Role};
use crate::{OverlayWindow, win32};

struct Screen {
    monitor: Monitor,
    frame: SharedPixelBuffer<Rgb8Pixel>,
    window: OverlayWindow,
}

pub struct Session {
    screens: Vec<Screen>,
}

/// What the user selected: the pixels, and where they were on screen.
pub struct Selection {
    pub image: RgbImage,
    pub bounds: PhysRect,
    pub monitor: Monitor,
}

/// Margin added around the crop before recognition, in frame pixels. Selections are
/// often drawn tight to the text; the padding, colored like the background at the
/// crop's edge, keeps border glyphs from being cut off.
const RECOGNITION_PAD: u32 = 16;

impl Session {
    /// Captures every monitor first, then covers each with its frozen frame.
    pub fn open() -> Result<Self> {
        let monitors = capture::monitors();
        let mut frames = Vec::with_capacity(monitors.len());
        for monitor in monitors {
            frames.push((monitor, capture::capture(monitor.bounds)?));
        }

        let cursor = win32::cursor_pos();
        let mut screens = Vec::with_capacity(frames.len());
        for (index, (monitor, capture::Frame { pixels: frame, dimmed })) in frames.into_iter().enumerate() {
            let window = window_role::create(Role::Overlay, OverlayWindow::new)?;
            window.set_frame(Image::from_rgb8(frame.clone()));
            // The window owns the dimmed copy; it is freed with the window.
            window.set_dimmed(Image::from_rgb8(dimmed));
            let b = monitor.bounds;
            window.window().set_position(PhysicalPosition::new(b.x, b.y));
            window.window().set_size(PhysicalSize::new(b.width as u32, b.height as u32));
            window.on_cancel(|| app::post(Action::OverlayCancel));
            window.on_selected(move |x, y, w, h| app::post(Action::OverlaySelected { screen: index, x, y, w, h }));
            window.window().on_close_requested(|| {
                app::post(Action::OverlayCancel);
                slint::CloseRequestResponse::KeepWindowShown
            });
            window.show()?;
            screens.push(Screen { monitor, frame, window });
        }

        // Keyboard focus goes to the monitor under the cursor, so Esc works right away.
        let focused = cursor.and_then(|(x, y)| screens.iter().find(|s| s.monitor.bounds.contains(x, y)));
        if let Some(screen) = focused.or(screens.first()) {
            win32::activate_when_created(screen.window.as_weak(), 50);
        }
        Ok(Self { screens })
    }

    /// Cuts the selection out of the original frame (not the dimmed overlay), so the
    /// scrim and hint never reach recognition.
    pub fn selection(&self, screen: usize, x: f32, y: f32, w: f32, h: f32) -> Option<Selection> {
        let screen = self.screens.get(screen)?;
        let scale = screen.window.window().scale_factor();
        let (fw, fh) = (screen.frame.width(), screen.frame.height());
        let (px, py, pw, ph) = capture::logical_to_frame(x, y, w, h, scale, fw, fh)?;
        let image = RgbImage::crop_from(screen.frame.as_bytes(), fw, px, py, pw, ph).padded(RECOGNITION_PAD);
        let b = screen.monitor.bounds;
        Some(Selection {
            image,
            bounds: PhysRect { x: b.x + px as i32, y: b.y + py as i32, width: pw as i32, height: ph as i32 },
            monitor: screen.monitor,
        })
    }

    /// Hides and destroys the windows; the frames are freed with them.
    pub fn close(self) {
        for screen in &self.screens {
            let _ = screen.window.hide();
        }
        drop(self);
    }
}
