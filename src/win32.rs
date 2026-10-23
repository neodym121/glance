//! Small Win32 helpers: clipboard, shell, registry, process memory, windows.

use std::time::Duration;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, GlobalFree, HANDLE, HWND, POINT};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GetProcessHeap, GlobalAlloc, GlobalLock, GlobalUnlock, HEAP_NONE, HeapCompact};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
    RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateMutexW, GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, MB_ICONINFORMATION, MB_OK,
    MessageBoxW, SW_SHOWNORMAL, SetForegroundWindow,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

use crate::error::{Error, Result};

/// Turns off the DWM open/close animation for every window this thread creates.
/// The attribute only sticks once the window exists (from WM_CREATE on; earlier, at
/// HCBT_CREATEWND, DWM ignores it) and has to be set before the window is first
/// shown, so a thread-local hook on sent messages sets it on WM_CREATE and again on
/// WM_SHOWWINDOW.
pub fn disable_window_animations() {
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{
        CWPSTRUCT, CallNextHookEx, HC_ACTION, SetWindowsHookExW, WH_CALLWNDPROC, WM_CREATE, WM_SHOWWINDOW,
    };

    unsafe extern "system" fn on_message(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let message = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
            if message.message == WM_CREATE || (message.message == WM_SHOWWINDOW && message.wParam.0 != 0) {
                let disable = windows::core::BOOL(1);
                let _ = unsafe {
                    DwmSetWindowAttribute(
                        message.hwnd,
                        DWMWA_TRANSITIONS_FORCEDISABLED,
                        &disable as *const _ as *const _,
                        std::mem::size_of_val(&disable) as u32,
                    )
                };
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
    // The hook lives as long as the UI thread.
    let _ = unsafe { SetWindowsHookExW(WH_CALLWNDPROC, Some(on_message), None, GetCurrentThreadId()) };
}

pub fn message_box(text: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), w!("Glance"), MB_OK | MB_ICONINFORMATION);
    }
}

/// Holds a named mutex for the lifetime of the process; `None` if another
/// instance already owns it.
pub struct SingleInstance(#[allow(dead_code)] HANDLE);

impl SingleInstance {
    pub fn acquire() -> Option<Self> {
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\Glance.SingleInstance")) }.ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return None;
        }
        Some(Self(handle))
    }
}

/// Lets the tray context menu follow the system dark mode. Uses the undocumented
/// uxtheme `SetPreferredAppMode` (ordinal 135), as Explorer-styled apps do; a no-op
/// where it is missing.
pub fn allow_dark_menus() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(set_preferred_app_mode) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 =
                std::mem::transmute(set_preferred_app_mode);
            const ALLOW_DARK: i32 = 1;
            set_preferred_app_mode(ALLOW_DARK);
        }
    }
}

pub fn set_clipboard_text(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Another app may hold the clipboard for a moment.
        let mut opened = OpenClipboard(None).is_ok();
        for _ in 0..5 {
            if opened {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
            opened = OpenClipboard(None).is_ok();
        }
        if !opened {
            return Err(Error::new("The clipboard is busy in another app."));
        }
        let result = (|| -> windows::core::Result<()> {
            EmptyClipboard()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            let target = GlobalLock(memory) as *mut u16;
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            let _ = GlobalUnlock(memory);
            // On success the clipboard owns the memory.
            if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))) {
                let _ = GlobalFree(Some(memory));
                return Err(e);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result.map_err(|e| Error::new(format!("Could not copy to the clipboard: {}", Error::from(e))))
    }
}

/// Opens an http(s) URL in the default browser. Callers validate with `links::validate`.
pub fn shell_open_url(url: &str) -> Result<()> {
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success with a value greater than 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(Error::new("Windows could not open the link."))
    }
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Glance");

pub fn autostart_enabled() -> bool {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)).is_ok()
    }
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    let mut key = HKEY::default();
    unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key)
            .ok()
            .map_err(|e| Error::new(format!("Could not change the startup setting: {}", Error::from(e))))?;
        let result = if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| Error::new(format!("Could not find Glance.exe: {e}")))?;
            let command = format!("\"{}\"", exe.display());
            let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes)).ok()
        } else {
            match RegDeleteValueW(key, RUN_VALUE).ok() {
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => Ok(()),
                other => other,
            }
        };
        let _ = RegCloseKey(key);
        result.map_err(|e| Error::new(format!("Could not change the startup setting: {}", Error::from(e))))
    }
}

/// True when the taskbar uses the light theme (the tray icon then needs a dark glyph).
pub fn taskbar_is_light() -> bool {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
        .is_ok()
            && value == 1
    }
}

pub fn user_locale() -> String {
    let mut buffer = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..len as usize - 1])
}

/// Returns freed heap pages and the working set to the system once the app is idle.
pub fn trim_memory() {
    unsafe {
        if let Ok(heap) = GetProcessHeap() {
            HeapCompact(heap, HEAP_NONE);
        }
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// Waits for the compositor to present the next frame, so a window hidden just before
/// is no longer on screen when the desktop is captured.
pub fn wait_for_compositor() {
    unsafe {
        let _ = DwmFlush();
    }
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

/// Virtual-key code that types `c` in the active keyboard layout.
pub fn virtual_key_for_char(c: char) -> Option<u16> {
    let mut units = [0u16; 2];
    let [unit] = c.encode_utf16(&mut units) else { return None };
    let scan = unsafe { VkKeyScanW(*unit) };
    (scan != -1).then_some((scan & 0xFF) as u16)
}

/// Activates one of our windows even while another app owns the foreground, as it
/// does right after the global hotkey. Windows only lets the foreground thread hand
/// over activation, so this thread briefly shares its input queue.
fn force_foreground(hwnd: HWND) {
    unsafe {
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attach = foreground != 0 && foreground != current;
        if attach {
            let _ = AttachThreadInput(current, foreground, true);
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attach {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}

pub fn bring_to_front(window: &slint::Window) {
    if let Some(hwnd) = hwnd(window) {
        force_foreground(hwnd);
    }
}

/// Activates a window that was just shown. Slint creates the native window on the next
/// event loop turn, so this polls briefly until it exists; holds only a weak handle.
pub fn activate_when_created<C: slint::ComponentHandle + 'static>(component: slint::Weak<C>, attempts: u32) {
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(strong) = component.upgrade() else { return };
        match hwnd(strong.window()) {
            Some(hwnd) => force_foreground(hwnd),
            None if attempts > 0 => activate_when_created(component, attempts - 1),
            None => {}
        }
    });
}

pub fn hwnd(window: &slint::Window) -> Option<HWND> {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        })
        .flatten()
}
