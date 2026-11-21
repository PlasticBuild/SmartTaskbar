mod apps;
mod field;
mod pane;
mod pill;
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
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_RETURN,
        },
        WindowsAndMessaging::*,
    },
};

const CHIP_CLASS: *const u16 = windows_sys::w!("SmartTaskbar.Rust.WebSearchButton");
const PANE_CLASS: *const u16 = windows_sys::w!("SmartTaskbar.Rust.WebSearchPane");
const WEB_HOTKEY: i32 = 1;
const PREFIX_HOTKEY: i32 = 2;
const FIELD_PREFIX: u32 = WM_APP + 1;

fn web_prefix(value: &str) -> Option<&str> {
    let value = value.trim_start();
    let (command, query) = value.split_once(':')?;
    command.eq_ignore_ascii_case("web").then(|| query.trim())
}

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

fn focused_query(source: u32, prefixed: bool) -> Option<String> {
    let (value, _) = field::read(&field::automation()?, source)?;
    let query = match web_prefix(&value) {
        Some(query) => query,
        None if !prefixed => value.trim(),
        None => return None,
    };
    Some(query.chars().take(512).collect())
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

pub fn show(hwnd: usize, config: &Path, prefixed: bool) -> Result<(), String> {
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
    pane::run(bounds, dpi, config, true, || focused_query(id, prefixed))
}

pub fn open_start(config: &Path) -> Result<(), String> {
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
    let settings = crate::settings::Settings::load(config)?;
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let (bounds, dpi) = crate::desktop::start_bounds(&settings)
        .ok_or("SmartTaskbar Start could not locate the active display.")?;
    pane::run(bounds, dpi, config, false, || None)
}

pub fn window() -> HWND {
    unsafe { FindWindowW(PANE_CLASS, null()) }
}

pub(crate) fn launch_start() {
    let pane = window();
    if !pane.is_null() {
        unsafe { SetForegroundWindow(pane) };
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut command = Command::new(exe);
        command.arg("--start");
        if let Ok((_, Some(config))) = crate::parse(std::env::args_os().skip(1)) {
            command.arg("--config").arg(config);
        }
        let _ = command.creation_flags(0x08000000).spawn();
    }
}

#[derive(Default)]
pub(crate) struct Button {
    hwnd: HWND,
    target: HWND,
    bounds: Rect,
    watch: Option<field::Watch>,
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
                    UnregisterHotKey(self.hwnd, PREFIX_HOTKEY);
                    ShowWindow(self.hwnd, SW_HIDE);
                    SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                }
                self.target = null_mut();
                if let Some(watch) = &self.watch {
                    watch.select(null_mut());
                }
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
        let field = self
            .watch
            .as_ref()
            .and_then(|watch| watch.current(panel.hwnd).bounds);
        let bounds = button_bounds(surface, panel.monitor, field, dpi);
        if bounds != self.bounds || panel.hwnd != self.target {
            unsafe {
                if self.hwnd.is_null() {
                    self.hwnd = create_button();
                }
                if self.hwnd.is_null() {
                    return;
                }
                if self.watch.is_none() {
                    self.watch = field::Watch::start(self.hwnd as usize);
                }
                if let Some(watch) = &self.watch
                    && self.target != panel.hwnd
                {
                    UnregisterHotKey(self.hwnd, PREFIX_HOTKEY);
                    watch.select(panel.hwnd);
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
                if pill::paint(self.hwnd, bounds, dpi, pane::dark_theme() != 0) {
                    SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
            }
            self.target = panel.hwnd;
            self.bounds = bounds;
        }
    }
}

fn button_bounds(surface: Rect, monitor: Rect, field: Option<Rect>, dpi: i32) -> Rect {
    let dip = |value| value * dpi / 96;
    let width = dip(64);
    let height = dip(31);
    let margin = dip(8);
    let search = field.unwrap_or(Rect::new(
        surface.left + dip(45),
        surface.top + dip(20),
        surface.right - dip(45),
        surface.top + dip(51),
    ));
    // Leave room at the right end of the filter row for the native options menu.
    let left = (search.right - dip(48) - width - margin)
        .clamp(monitor.left, (monitor.right - width).max(monitor.left));
    let top =
        (search.bottom + dip(18)).clamp(monitor.top, (monitor.bottom - height).max(monitor.top));
    Rect::new(left, top, left + width, top + height)
}

impl Drop for Button {
    fn drop(&mut self) {
        self.watch.take();
        if !self.hwnd.is_null() {
            unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

fn launch(hwnd: HWND, prefixed: bool) {
    let source = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as HWND;
    if pid(source) != 0
        && pid(source) == pid(unsafe { GetForegroundWindow() })
        && let Ok(exe) = std::env::current_exe()
    {
        let mut command = Command::new(exe);
        command
            .arg(if prefixed {
                "--start-prefixed-web-search"
            } else {
                "--start-web-search"
            })
            .arg((source as usize).to_string());
        if let Ok((_, Some(config))) = crate::parse(std::env::args_os().skip(1)) {
            command.arg("--config").arg(config);
        }
        let _ = command.creation_flags(0x08000000).spawn();
    }
}

unsafe extern "system" fn button_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_DRAWITEM => 1, // The layered parent paints the accessible child.
            WM_SIZE => {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                MoveWindow(GetDlgItem(hwnd, 1), 0, 0, rect.right, rect.bottom, 1);
                0
            }
            FIELD_PREFIX => {
                if lp == GetWindowLongPtrW(hwnd, GWLP_USERDATA) {
                    crate::desktop::search_field_changed();
                    UnregisterHotKey(hwnd, PREFIX_HOTKEY);
                    if wp != 0 && pid(lp as HWND) == pid(GetForegroundWindow()) {
                        RegisterHotKey(hwnd, PREFIX_HOTKEY, MOD_NOREPEAT, u32::from(VK_RETURN));
                    }
                }
                0
            }
            WM_COMMAND | WM_HOTKEY if wp == WEB_HOTKEY as usize => {
                launch(hwnd, false);
                0
            }
            WM_HOTKEY if wp == PREFIX_HOTKEY as usize => {
                launch(hwnd, true);
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
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
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
            windows_sys::w!("Web"),
            WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
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
        popup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_web_prefix_preserves_the_query_and_leaves_local_terms_alone() {
        assert_eq!(
            web_prefix(" Web: windows: keyboard shortcuts "),
            Some("windows: keyboard shortcuts")
        );
        assert_eq!(web_prefix("wEb:"), Some(""));
        assert_eq!(web_prefix("spider web: photos"), None);
        assert_eq!(web_prefix("web browser"), None);
    }

    #[test]
    fn web_button_follows_the_filter_row_on_a_high_dpi_display() {
        let screen = Rect::new(0, 0, 2880, 1800);
        let search = Rect::new(840, 235, 2040, 1670);
        let field = Rect::new(890, 265, 1990, 331);
        let chip = button_bounds(search, screen, Some(field), 192);
        assert_eq!(chip, Rect::new(1750, 367, 1878, 429));
        assert!(chip.bottom < search.top + 220);
    }
}
