//! Native title bar with only the window buttons: no icon, no caption text, and coloured to match
//! the app header. Keeps the real Windows frame, so snap layouts, resizing and the shadow still work.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows_sys::Win32::UI::Controls::{
        SetWindowThemeAttribute, WTA_NONCLIENT, WTA_OPTIONS, WTNCA_NODRAWCAPTION, WTNCA_NODRAWICON, WTNCA_NOSYSMENU,
    };

    /// The header's background (`--base` in styles.css) as a COLORREF (0x00BBGGRR).
    const DARK: u32 = 0x002E1E1E; // #1E1E2E
    const LIGHT: u32 = 0x00F9F5F5; // #F5F5F9

    pub fn hide_caption(hwnd: *mut c_void) {
        let flags = WTNCA_NODRAWCAPTION | WTNCA_NODRAWICON | WTNCA_NOSYSMENU;
        let opts = WTA_OPTIONS { dwFlags: flags, dwMask: flags };
        // Cosmetic only: if it fails the window just keeps its normal caption.
        unsafe {
            SetWindowThemeAttribute(hwnd, WTA_NONCLIENT, &opts as *const _ as *const c_void, size_of::<WTA_OPTIONS>() as u32);
        }
    }

    pub fn set_colors(hwnd: *mut c_void, dark: bool) {
        let dark_mode: i32 = dark.into();
        let color = if dark { DARK } else { LIGHT };
        // Windows 10 ignores the colour attributes and keeps its default bar, which is fine.
        unsafe {
            let set = |attr: i32, value: *const c_void, size: usize| DwmSetWindowAttribute(hwnd, attr as u32, value, size as u32);
            set(DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_mode as *const _ as *const c_void, size_of::<i32>());
            set(DWMWA_CAPTION_COLOR, &color as *const _ as *const c_void, size_of::<u32>());
            set(DWMWA_BORDER_COLOR, &color as *const _ as *const c_void, size_of::<u32>());
        }
    }
}

/// Hide the caption and colour the bar to match the given theme.
pub fn style(window: &tauri::WebviewWindow, dark: bool) {
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        imp::hide_caption(hwnd.0);
        imp::set_colors(hwnd.0, dark);
    }
    #[cfg(not(windows))]
    let _ = (window, dark);
}
