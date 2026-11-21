mod pane;
mod provider;

use crate::{
    join::{MenuKind, Panel},
    policy::Rect,
};
use std::{
    ffi::OsString,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationValuePattern, UIA_EditControlTypeId,
    UIA_ValuePatternId,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
        },
        WindowsAndMessaging::*,
    },
};

const CHIP_CLASS: *const u16 = windows_sys::w!("SmartTaskbar.Rust.WebSearchButton");
const PANE_CLASS: *const u16 = windows_sys::w!("SmartTaskbar.Rust.WebSearchPane");
const WEB_HOTKEY: i32 = 1;

fn pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

fn search_process(id: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id) };
    if handle.is_null() {
        return false;
    }
    let mut path = vec![0; 32_768];
    let mut len = path.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return false;
    }
    let path = PathBuf::from(OsString::from_wide(&path[..len as usize]));
    let Some(system) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let system_apps = Path::new(&system).join("SystemApps");
    path.ancestors().any(|parent| {
        parent
            .as_os_str()
            .eq_ignore_ascii_case(system_apps.as_os_str())
    }) && path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("SearchHost.exe") || name.eq_ignore_ascii_case("SearchApp.exe")
    })
}

fn focused_query(source: u32) -> Option<String> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let field = automation.GetFocusedElement().ok()?;
        if field.CurrentProcessId().ok()? as u32 != source
            || field.CurrentControlType().ok()? != UIA_EditControlTypeId
        {
            return None;
        }
        let value: IUIAutomationValuePattern =
            field.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        Some(
            value
                .CurrentValue()
                .ok()?
                .to_string()
                .trim()
                .chars()
                .take(512)
                .collect(),
        )
    }
}

fn choose_bounds(host: Rect, monitor: Rect, width: i32, dpi: i32) -> Rect {
    let width = width.max(260 * dpi / 96).min(monitor.width());
    let center = host.left + host.width() / 2;
    let left = (center - width / 2).clamp(monitor.left, monitor.right - width);
    let top = host.top.max(monitor.top);
    let bottom = host.bottom.min(monitor.bottom);
    Rect::new(left, top, left + width, bottom)
}

fn search_width(hwnd: HWND, host: Rect, dpi: i32, match_width: bool) -> i32 {
    if !match_width {
        return host.width().min(960 * dpi / 96);
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let primary =
        unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) }
            != 0
            && info.dwFlags & MONITORINFOF_PRIMARY != 0;
    let width = if primary {
        crate::desktop::primary_island().map_or(600, |island| island.search_width)
    } else {
        600
    };
    (width * dpi / 96).min(host.width())
}

pub fn show(hwnd: usize, config: &Path) -> Result<(), String> {
    let hwnd = hwnd as HWND;
    let id = pid(hwnd);
    if id == 0 || pid(unsafe { GetForegroundWindow() }) != id || !search_process(id) {
        return Ok(());
    }
    let _single = match crate::SingleInstance::lock(r"Local\SmartTaskbarRust.EmbeddedSearch") {
        Ok(lock) => lock,
        Err(_) => {
            let pane = window();
            if !pane.is_null() {
                unsafe { SetForegroundWindow(pane) };
            }
            return Ok(());
        }
    };
    let mut host = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        use windows_sys::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
        };
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if GetWindowRect(hwnd, &mut host) == 0
            || GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) == 0
        {
            return Err("Could not measure the Start Search window.".into());
        }
    }
    let host = Rect::new(host.left, host.top, host.right, host.bottom);
    let work = monitor.rcWork;
    let monitor = Rect::new(work.left, work.top, work.right, work.bottom);
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    let settings = crate::settings::Settings::load(config)?;
    let width = search_width(
        hwnd,
        host,
        dpi,
        settings.island && settings.match_start_search_width,
    );
    let bounds = choose_bounds(host, monitor, width, dpi);
    pane::run(bounds, dpi, config, || focused_query(id))
}

pub fn window() -> HWND {
    unsafe { FindWindowW(PANE_CLASS, null()) }
}

#[derive(Default)]
pub(crate) struct Button {
    hwnd: HWND,
    target: HWND,
    bounds: Rect,
}

impl Button {
    pub fn update(&mut self, panels: &[Panel], settings: &crate::settings::Settings) {
        let foreground = unsafe { GetForegroundWindow() };
        let id = pid(foreground);
        let Some(panel) = panels
            .iter()
            .find(|panel| id != 0 && panel.kind == MenuKind::Search && pid(panel.hwnd) == id)
        else {
            if !self.target.is_null() {
                unsafe {
                    UnregisterHotKey(self.hwnd, WEB_HOTKEY);
                    ShowWindow(self.hwnd, SW_HIDE);
                }
                self.target = null_mut();
            }
            return;
        };
        let dpi = unsafe { GetDpiForWindow(panel.hwnd) }.max(96) as i32;
        let surface = choose_bounds(
            panel.bounds,
            panel.monitor,
            search_width(
                panel.hwnd,
                panel.bounds,
                dpi,
                settings.island && settings.match_start_search_width,
            ),
            dpi,
        );
        let margin = 15 * dpi / 96;
        let width = 105 * dpi / 96;
        let height = 32 * dpi / 96;
        let bottom = surface.bottom - margin;
        let right = surface.right - margin;
        let bounds = Rect::new(right - width, bottom - height, right, bottom);
        if bounds != self.bounds || panel.hwnd != self.target {
            unsafe {
                if self.hwnd.is_null() {
                    self.hwnd = create_button();
                }
                if self.hwnd.is_null() {
                    return;
                }
                if self.target.is_null() {
                    RegisterHotKey(
                        self.hwnd,
                        WEB_HOTKEY,
                        MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                        u32::from(b'W'),
                    );
                }
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, panel.hwnd as isize);
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    bounds.left,
                    bounds.top,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            self.target = panel.hwnd;
            self.bounds = bounds;
        }
    }
}

impl Drop for Button {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

unsafe extern "system" fn button_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_SIZE => {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                MoveWindow(GetDlgItem(hwnd, 1), 0, 0, rect.right, rect.bottom, 1);
                0
            }
            WM_COMMAND | WM_HOTKEY if wp == 1 => {
                let source = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND;
                if pid(source) != 0
                    && pid(source) == pid(GetForegroundWindow())
                    && let Ok(exe) = std::env::current_exe()
                {
                    let mut command = Command::new(exe);
                    command
                        .arg("--start-web-search")
                        .arg((source as usize).to_string());
                    if let Ok((_, Some(config))) = crate::parse(std::env::args_os().skip(1)) {
                        command.arg("--config").arg(config);
                    }
                    let _ = command.creation_flags(0x08000000).spawn();
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn create_button() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(button_proc),
            lpszClassName: CHIP_CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return null_mut();
        }
        let popup = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            CHIP_CLASS,
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if popup.is_null() {
            return popup;
        }
        let child = CreateWindowExW(
            0,
            windows_sys::w!("BUTTON"),
            windows_sys::w!("Search the web"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            popup,
            std::ptr::without_provenance_mut(1),
            instance,
            null(),
        );
        if child.is_null() {
            DestroyWindow(popup);
            return null_mut();
        }
        SendMessageW(
            child,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
        popup
    }
}
