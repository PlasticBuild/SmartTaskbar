use crate::policy::Rect;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND},
    Graphics::Gdi::{COLOR_HIGHLIGHT, GetSysColorBrush},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const CLASS: *const u16 = windows_sys::w!("SmartTaskbar.ActivationZonePreview");

#[derive(Default)]
pub(super) struct Overlay {
    bands: Vec<Rect>,
    windows: Vec<HWND>,
}

impl Overlay {
    pub(super) fn show(&mut self, owner: HWND, bands: Vec<Rect>) {
        if self.bands == bands {
            return;
        }
        self.clear();
        if bands.is_empty() {
            return;
        }
        let instance = unsafe { GetModuleHandleW(null()) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            hbrBackground: unsafe { GetSysColorBrush(COLOR_HIGHLIGHT) },
            lpszClassName: CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS } {
            return;
        }
        for band in &bands {
            unsafe {
                let window = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                    CLASS,
                    null(),
                    WS_POPUP,
                    band.left,
                    band.top,
                    band.width(),
                    band.height(),
                    owner,
                    null_mut(),
                    instance,
                    null(),
                );
                if window.is_null() {
                    continue;
                }
                if SetLayeredWindowAttributes(window, 0, 185, LWA_ALPHA) != 0
                    && SetWindowPos(
                        window,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    ) != 0
                {
                    self.windows.push(window);
                } else {
                    DestroyWindow(window);
                }
            }
        }
        self.bands = bands;
    }

    fn clear(&mut self) {
        for window in self.windows.drain(..) {
            unsafe { DestroyWindow(window) };
        }
        self.bands.clear();
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.clear();
    }
}
