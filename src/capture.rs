//! Freezing the screen: one GDI capture per monitor, written straight into the pixel
//! buffer the overlay displays. Frames never touch the disk and are dropped together
//! with the overlay.

use slint::{Rgb8Pixel, SharedPixelBuffer};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, EnumDisplayMonitors, GdiFlush, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

use crate::error::{Error, Result};

/// Physical pixel rectangle in virtual-screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PhysRect {
    fn from_rect(r: RECT) -> Self {
        Self { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub bounds: PhysRect,
    /// Bounds minus the taskbar and docked toolbars.
    pub work_area: PhysRect,
    pub scale: f32,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    handles
        .into_iter()
        .filter_map(|handle| {
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            unsafe { GetMonitorInfoW(handle, &mut info) }.ok().ok()?;
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            Some(Monitor {
                bounds: PhysRect::from_rect(info.rcMonitor),
                work_area: PhysRect::from_rect(info.rcWork),
                scale: dpi_x.max(96) as f32 / 96.0,
            })
        })
        .collect()
}

/// Releases GDI objects in reverse order of creation.
struct GdiCapture {
    screen: HDC,
    memory: HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
}

impl Drop for GdiCapture {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

/// A monitor's image (3 bytes per pixel), and the same image at half brightness: the
/// overlay draws the dimmed copy and shows the original only inside the selection, so
/// moving the selection repaints just those spots instead of a full-screen
/// scrim.
pub struct Frame {
    pub pixels: SharedPixelBuffer<Rgb8Pixel>,
    pub dimmed: SharedPixelBuffer<Rgb8Pixel>,
}

/// Copies the monitor's current image into RGB buffers.
pub fn capture(bounds: PhysRect) -> Result<Frame> {
    let (width, height) = (bounds.width, bounds.height);
    if width <= 0 || height <= 0 {
        return Err(Error::new("A monitor reported an empty size."));
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(width as u32, height as u32);
    let mut dimmed = SharedPixelBuffer::<Rgb8Pixel>::new(width as u32, height as u32);
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(Error::new("The screen is not available for capture."));
        }
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(e) => {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return Err(e.into());
            }
        };
        let previous = SelectObject(memory, bitmap.into());
        let guard = GdiCapture { screen, memory, bitmap, previous };

        // CAPTUREBLT includes layered (translucent) windows in the copy.
        BitBlt(memory, 0, 0, width, height, Some(screen), bounds.x, bounds.y, SRCCOPY | CAPTUREBLT)?;
        let _ = GdiFlush();

        // BGRX from GDI to RGB for Slint; the fourth byte is undefined in screen captures.
        // Halving matches a 50% black scrim.
        let source = std::slice::from_raw_parts(bits as *const u8, width as usize * height as usize * 4);
        let originals = pixels.make_mut_bytes().as_chunks_mut::<3>().0.iter_mut();
        let darks = dimmed.make_mut_bytes().as_chunks_mut::<3>().0.iter_mut();
        for ((dst, dark), src) in originals.zip(darks).zip(source.as_chunks::<4>().0) {
            *dst = [src[2], src[1], src[0]];
            *dark = [src[2] >> 1, src[1] >> 1, src[0] >> 1];
        }
        drop(guard);
    }
    Ok(Frame { pixels, dimmed })
}

/// Converts a selection in logical overlay coordinates to physical pixels of the
/// frame, clamped to it. `None` when nothing usable is left.
pub fn logical_to_frame(x: f32, y: f32, w: f32, h: f32, scale: f32, frame_w: u32, frame_h: u32) -> Option<(u32, u32, u32, u32)> {
    let left = (x * scale).round().clamp(0.0, frame_w as f32) as u32;
    let top = (y * scale).round().clamp(0.0, frame_h as f32) as u32;
    let right = ((x + w) * scale).round().clamp(0.0, frame_w as f32) as u32;
    let bottom = ((y + h) * scale).round().clamp(0.0, frame_h as f32) as u32;
    (right > left + 1 && bottom > top + 1).then(|| (left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_logical_selection_to_frame_pixels() {
        assert_eq!(logical_to_frame(10.0, 20.0, 100.0, 50.0, 1.5, 1920, 1080), Some((15, 30, 150, 75)));
        // Clamped to the frame.
        assert_eq!(logical_to_frame(-5.0, 0.0, 30.0, 30.0, 1.0, 20, 20), Some((0, 0, 20, 20)));
        // Too small after clamping.
        assert_eq!(logical_to_frame(19.5, 0.0, 10.0, 10.0, 1.0, 20, 20), None);
    }

    #[test]
    fn rect_contains_points() {
        let r = PhysRect { x: -1920, y: 0, width: 1920, height: 1080 };
        assert!(r.contains(-1, 0));
        assert!(!r.contains(0, 0));
        assert_eq!(r.right(), 0);
    }
}
