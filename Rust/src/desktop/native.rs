use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};

use crate::join::{MenuKind, Panel};
use crate::policy::{Edge, NativeBar, Point, Rect, WindowSample};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::{Dwm::*, Gdi::*};
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*};

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub(super) fn rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}
pub(super) fn winrect(r: Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

pub(super) fn window_class(h: HWND) -> String {
    let mut buf = [0u16; 192];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub(super) fn window_pid(h: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    pid
}

pub(super) fn process_path(pid: u32) -> Option<std::path::PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let path = path_for_handle(handle);
    unsafe { CloseHandle(handle) };
    path
}

fn path_for_handle(handle: HANDLE) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 32768];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
    (ok != 0).then(|| std::ffi::OsString::from_wide(&buf[..len as usize]).into())
}

struct Owner {
    handle: HANDLE,
    name: String,
    seen: bool,
}
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}
#[derive(Default)]
pub(super) struct ProcessNames(HashMap<u32, Owner>);
impl ProcessNames {
    fn lookup(&mut self, pid: u32) -> &str {
        // An open process handle pins the kernel process identity, including its PID, until
        // the cache releases it. Prune owners with no visible windows at the end of each scan.
        if let std::collections::hash_map::Entry::Vacant(slot) = self.0.entry(pid) {
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if h.is_null() {
                return "";
            }
            let name = path_for_handle(h)
                .and_then(|p| Path::file_name(&p).map(|s| s.to_string_lossy().to_ascii_lowercase()))
                .unwrap_or_default();
            slot.insert(Owner {
                handle: h,
                name,
                seen: true,
            });
        }
        let entry = self.0.get_mut(&pid).unwrap();
        entry.seen = true;
        &entry.name
    }
}

pub(super) fn window_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    (unsafe { GetWindowRect(h, &mut r) } != 0 && r.right > r.left && r.bottom > r.top)
        .then(|| rect(r))
}

fn visual_bounds(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    if unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut r as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    } >= 0
        && r.right > r.left
        && r.bottom > r.top
    {
        return Some(rect(r));
    }
    window_rect(h)
}

pub(super) fn cloaked(h: HWND) -> bool {
    let mut flags = 0u32;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, (&mut flags as *mut u32).cast(), 4) };
    flags != 0
}

#[derive(Clone, Copy)]
pub(super) struct Display {
    pub bounds: Rect,
    pub work: Rect,
    pub primary: bool,
}

pub(super) fn display_for(h: HWND) -> Option<Display> {
    let mon = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONULL) };
    display_info(mon)
}

fn display_info(h: HMONITOR) -> Option<Display> {
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    (!h.is_null() && unsafe { GetMonitorInfoW(h, &mut info) } != 0).then(|| Display {
        bounds: rect(info.rcMonitor),
        work: rect(info.rcWork),
        primary: info.dwFlags & 1 != 0,
    })
}

pub(super) fn displays() -> Vec<Rect> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        if let Some(info) = display_info(h) {
            unsafe { (&mut *(data as *mut Vec<Rect>)).push(info.bounds) };
        }
        1
    }
    let mut out = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(each),
            (&mut out as *mut Vec<Rect>) as LPARAM,
        )
    };
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Monitor {
    pub bounds: Rect,
    pub id: String,
    pub label: String,
}

pub(super) fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        unsafe {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(h, &mut info.monitorInfo) != 0 {
                let name = from_wide(&info.szDevice);
                let mut device = DISPLAY_DEVICEW {
                    cb: size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                let mut model = String::new();
                let mut id = String::new();
                for index in 0..16 {
                    if EnumDisplayDevicesW(
                        info.szDevice.as_ptr(),
                        index,
                        &mut device,
                        EDD_GET_DEVICE_INTERFACE_NAME,
                    ) == 0
                    {
                        break;
                    }
                    if device.StateFlags & DISPLAY_DEVICE_ACTIVE != 0 {
                        model = from_wide(&device.DeviceString);
                        id = from_wide(&device.DeviceID);
                        break;
                    }
                }
                let number = name.rsplit("DISPLAY").next().unwrap_or(&name);
                let primary = if info.monitorInfo.dwFlags & 1 != 0 {
                    " (primary)"
                } else {
                    ""
                };
                let mut label = format!("Display {number}{primary}");
                if !model.is_empty() && !model.to_ascii_lowercase().starts_with("generic") {
                    label.push_str(" · ");
                    label.push_str(&model);
                }
                (&mut *(data as *mut Vec<Monitor>)).push(Monitor {
                    bounds: rect(info.monitorInfo.rcMonitor),
                    id: if id.is_empty() { name } else { id }.to_ascii_lowercase(),
                    label,
                });
            }
        }
        1
    }
    let mut monitors = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(each),
            (&mut monitors as *mut Vec<Monitor>) as LPARAM,
        )
    };
    monitors.sort_by_key(|display: &Monitor| (display.bounds.top, display.bounds.left));
    monitors
}

fn from_wide(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

pub(super) fn shell_pid() -> u32 {
    window_pid(unsafe { GetShellWindow() })
}
pub(super) fn is_bar(h: HWND, pid: u32) -> bool {
    pid != 0
        && window_pid(h) == pid
        && matches!(
            window_class(h).as_str(),
            "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
}

#[derive(Clone, Copy)]
pub(super) struct Bar {
    pub hwnd: HWND,
    pub pid: u32,
    pub strip: NativeBar,
    pub display: Display,
    pub dpi: u32,
}

pub(super) fn bar_info(h: HWND, pid: u32) -> Option<Bar> {
    if !is_bar(h, pid) {
        return None;
    }
    let display = display_for(h)?;
    let strip = NativeBar::from_bounds(display.bounds, window_rect(h)?)?;
    Some(Bar {
        hwnd: h,
        pid,
        strip,
        display,
        dpi: unsafe { GetDpiForWindow(h) }.max(96),
    })
}

pub(super) fn top_windows() -> Vec<HWND> {
    let mut out = Vec::new();
    collect_top_windows(&mut out);
    out
}

fn collect_top_windows(out: &mut Vec<HWND>) {
    unsafe extern "system" fn each(h: HWND, data: LPARAM) -> i32 {
        unsafe { (&mut *(data as *mut Vec<HWND>)).push(h) };
        1
    }
    out.clear();
    unsafe { EnumWindows(Some(each), (out as *mut Vec<HWND>) as LPARAM) };
}

pub(super) fn bars() -> Vec<Bar> {
    let pid = shell_pid();
    if pid == 0 {
        return Vec::new();
    }
    let mut bars = Vec::new();
    for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
        let class = wide(class);
        let mut after = null_mut();
        loop {
            let h = unsafe { FindWindowExW(null_mut(), after, class.as_ptr(), null()) };
            if h.is_null() || h == after {
                break;
            }
            if let Some(bar) = bar_info(h, pid) {
                bars.push(bar);
            }
            after = h;
        }
    }
    bars
}

pub(super) fn tablet() -> bool {
    unsafe {
        GetSystemMetrics(SM_CONVERTIBLESLATEMODE) == 0
            && (GetSystemMetrics(SM_TABLETPC) != 0 || GetSystemMetrics(SM_DIGITIZER) & 1 != 0)
    }
}

pub(super) fn explorer_auto_hide() -> bool {
    let mut data: APPBARDATA = unsafe { zeroed() };
    data.cbSize = size_of::<APPBARDATA>() as u32;
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) & ABS_AUTOHIDE as usize != 0 }
}

pub(super) fn cursor() -> Option<Point> {
    let mut p = POINT::default();
    (unsafe { GetCursorPos(&mut p) } != 0).then_some(Point { x: p.x, y: p.y })
}

fn thumbnail_class(class: &str) -> bool {
    matches!(class, "TaskListThumbnailWnd" | "TaskListOverlayWnd")
}

fn thumbnail_group(class: &str, owner_class: &str, owned_by_tray: bool, bounds: Rect) -> bool {
    class == "TaskListThumbnailWnd"
        || (class == "Xaml_WindowedPopupClass"
            && (thumbnail_class(owner_class)
                || owned_by_tray && bounds.width() >= 144 && bounds.height() >= 72))
}

fn thumbnail_display(hwnd: HWND, mut owner: HWND, shell: u32) -> Option<Display> {
    for _ in 0..4 {
        if owner.is_null() || window_pid(owner) != shell {
            break;
        }
        if is_bar(owner, shell) {
            return display_for(owner);
        }
        owner = unsafe { GetWindow(owner, GW_OWNER) };
    }
    display_for(hwnd)
}

pub(super) fn hovered_thumbnail(point: Point, shell: u32) -> Option<Rect> {
    if shell == 0 {
        return None;
    }
    let hwnd = unsafe {
        GetAncestor(
            WindowFromPhysicalPoint(POINT {
                x: point.x,
                y: point.y,
            }),
            GA_ROOT,
        )
    };
    if hwnd.is_null() || window_pid(hwnd) != shell || !thumbnail_class(&window_class(hwnd)) {
        return None;
    }
    thumbnail_display(hwnd, unsafe { GetWindow(hwnd, GW_OWNER) }, shell).map(|d| d.bounds)
}

fn shell_surface(name: &str, class: &str, has_owner: bool) -> bool {
    match name {
        "startmenuexperiencehost.exe" | "searchhost.exe" | "searchapp.exe" => matches!(
            class,
            "Windows.UI.Core.CoreWindow"
                | "Windows.UI.Composition.DesktopWindowContentBridge"
                | "XamlExplorerHostIslandWindow"
                | "Microsoft.UI.Content.DesktopChildSiteBridge"
                | "Windows.UI.Input.InputSite.WindowClass"
        ),
        "shellexperiencehost.exe" | "shellhost.exe" => matches!(
            class,
            "ControlCenterWindow"
                | "Windows.UI.Core.CoreWindow"
                | "Windows.UI.Composition.DesktopWindowContentBridge"
                | "NotificationCenterWindow"
                | "TopLevelWindowForOverflowXamlIsland"
                | "XamlExplorerHostIslandWindow"
        ),
        "explorer.exe" => {
            thumbnail_class(class)
                || matches!(
                    class,
                    "NotifyIconOverflowWindow"
                        | "TopLevelWindowForOverflowXamlIsland"
                        | "DV2ControlHost"
                        | "JumpListHost"
                        | "ControlCenterWindow"
                        | "NotificationCenterWindow"
                        | "ClockFlyoutWindow"
                )
                || (class == "#32768" && has_owner)
        }
        _ => false,
    }
}

fn background_class(class: &str) -> bool {
    matches!(
        class,
        "Progman"
            | "WorkerW"
            | "SHELLDLL_DefView"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "ApplicationManager_ImmersiveShellWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
            | "tooltips_class32"
            | "SysShadow"
            | "#32768"
    )
}

#[derive(Default)]
pub(super) struct Scene {
    pub windows: Vec<WindowSample>,
    pub foreground_fullscreen: Option<Rect>,
    pub menus: Vec<Rect>,
    pub panels: Vec<Panel>,
    pub context_menus: Vec<(usize, Rect)>,
    pub thumbnail_groups: Vec<(Rect, Rect)>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.windows.clear();
        self.foreground_fullscreen = None;
        self.menus.clear();
        self.panels.clear();
        self.context_menus.clear();
        self.thumbnail_groups.clear();
    }
}

fn panel_kind(name: &str, class: &str, bounds: Rect, display: Rect, dpi: u32) -> Option<MenuKind> {
    match (name, class) {
        ("startmenuexperiencehost.exe", _) => Some(MenuKind::Start),
        (_, "ControlCenterWindow") => Some(MenuKind::QuickSettings),
        (_, "ClockFlyoutWindow" | "NotificationCenterWindow") => Some(MenuKind::Notifications),
        (_, "NotifyIconOverflowWindow" | "TopLevelWindowForOverflowXamlIsland") => {
            Some(MenuKind::Overflow)
        }
        (_, "JumpListHost" | "DV2ControlHost") => Some(MenuKind::JumpList),
        ("shellhost.exe", _) => Some(MenuKind::QuickSettings),
        ("shellexperiencehost.exe", "Windows.UI.Core.CoreWindow") => {
            // This host uses the same window class for two visually different surfaces.
            // A modern notification center uses a monitor-height wrapper, a jump list is compact.
            if px_to_dip(bounds.width(), dpi) < 350
                && bounds.height() < display.height().saturating_mul(4) / 5
            {
                Some(MenuKind::JumpList)
            } else {
                Some(MenuKind::Notifications)
            }
        }
        _ => None,
    }
}

fn collect_candidates(windows: &mut Vec<HWND>, foreground: HWND) {
    collect_top_windows(windows);
    if !foreground.is_null() && !windows.contains(&foreground) {
        windows.insert(0, foreground)
    }
    // EnumWindows can omit packaged shell windows. Also enumerate their installed window classes.
    for class in [
        windows_sys::w!("Windows.UI.Core.CoreWindow"),
        windows_sys::w!("ControlCenterWindow"),
        windows_sys::w!("TopLevelWindowForOverflowXamlIsland"),
        windows_sys::w!("NotificationCenterWindow"),
        windows_sys::w!("Xaml_WindowedPopupClass"),
    ] {
        let mut prev = null_mut();
        for _ in 0..128 {
            let h = unsafe { FindWindowExW(null_mut(), prev, class, null()) };
            if h.is_null() || h == prev {
                break;
            }
            if !windows.contains(&h) {
                windows.push(h)
            }
            prev = h;
        }
    }
}

pub(super) fn refresh_scene(names: &mut ProcessNames, out: &mut Scene, windows: &mut Vec<HWND>) {
    out.clear();
    for item in names.0.values_mut() {
        item.seen = false;
    }
    let fg = unsafe { GetForegroundWindow() };
    let shell = shell_pid();
    collect_candidates(windows, fg);
    for &h in windows.iter() {
        if unsafe { IsWindowVisible(h) } == 0 || unsafe { IsIconic(h) } != 0 || cloaked(h) {
            continue;
        }
        let Some(bounds) = visual_bounds(h) else {
            continue;
        };
        if bounds.width() < 2 || bounds.height() < 2 {
            continue;
        }
        let class = window_class(h);
        let owner = unsafe { GetWindow(h, GW_OWNER) };
        let pid = window_pid(h);
        let name = names.lookup(pid);
        let owner_class = if owner.is_null() {
            String::new()
        } else {
            window_class(owner)
        };
        if pid == shell
            && class == "Xaml_WindowedPopupClass"
            && !thumbnail_group(&class, &owner_class, is_bar(owner, shell), bounds)
            && unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32 & WS_EX_NOACTIVATE != 0
        {
            continue;
        }
        if matches!(class.as_str(), "#32768" | "Xaml_WindowedPopupClass")
            && let Some(home) = display_for(h)
        {
            out.context_menus.push((h as usize, home.bounds));
        }
        let tray_owned = !owner.is_null()
            && (is_bar(owner, shell)
                || matches!(
                    owner_class.as_str(),
                    "TopLevelWindowForOverflowXamlIsland"
                        | "NotifyIconOverflowWindow"
                        | "TaskListThumbnailWnd"
                ));
        if shell != 0
            && pid == shell
            && thumbnail_group(&class, &owner_class, is_bar(owner, shell), bounds)
            && let Some(home) = thumbnail_display(h, owner, shell)
        {
            out.thumbnail_groups.push((home.bounds, bounds));
        }
        if shell_surface(name, &class, tray_owned)
            || (tray_owned && matches!(class.as_str(), "#32768" | "Xaml_WindowedPopupClass"))
        {
            let home = if thumbnail_class(&class) {
                thumbnail_display(h, owner, shell)
            } else {
                display_for(h)
            };
            if let Some(home) = home {
                out.menus.push(home.bounds);
                if let Some(kind) = panel_kind(
                    name,
                    &class,
                    bounds,
                    home.bounds,
                    unsafe { GetDpiForWindow(h) }.max(96),
                ) {
                    let p = Panel {
                        hwnd: h,
                        monitor: home.bounds,
                        bounds,
                        kind,
                    };
                    if h == fg {
                        out.panels.insert(0, p)
                    } else {
                        out.panels.push(p)
                    }
                }
            }
            continue;
        }
        if background_class(&class)
            || matches!(
                name,
                "searchhost.exe"
                    | "searchapp.exe"
                    | "startmenuexperiencehost.exe"
                    | "shellexperiencehost.exe"
                    | "shellhost.exe"
                    | "textinputhost.exe"
                    | "lockapp.exe"
            )
        {
            continue;
        }
        let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        if ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) != 0 {
            continue;
        }
        if ex & WS_EX_LAYERED != 0 {
            let mut alpha = 255u8;
            let mut flags = 0u32;
            if unsafe { GetLayeredWindowAttributes(h, null_mut(), &mut alpha, &mut flags) } != 0
                && flags & LWA_ALPHA != 0
                && alpha == 0
            {
                continue;
            }
        }
        if let Some(home) = display_for(h) {
            let maximized = unsafe { IsZoomed(h) } != 0;
            if h == fg && bounds.intersection(home.bounds) == Some(home.bounds) {
                // A regular maximized window can fill the monitor after we reclaim the work area.
                let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
                let framed = style & WS_CAPTION == WS_CAPTION || style & WS_THICKFRAME != 0;
                if !maximized || !framed {
                    out.foreground_fullscreen = Some(home.bounds);
                }
            }
            out.windows.push(WindowSample {
                bounds,
                home: home.bounds,
                maximized,
            });
        }
    }
    names.0.retain(|_, item| item.seen);
}

pub(super) fn set_work(r: Rect) -> bool {
    let mut r = winrect(r);
    unsafe {
        SystemParametersInfoW(
            SPI_SETWORKAREA,
            0,
            (&mut r as *mut RECT).cast::<c_void>(),
            0,
        ) != 0
    }
}

pub(super) fn reflow_maximized(display: Rect, before: Rect, after: Rect) {
    for h in top_windows() {
        if unsafe { IsZoomed(h) } == 0
            || unsafe { IsIconic(h) } != 0
            || unsafe { IsWindowVisible(h) } == 0
            || cloaked(h)
            || !display_for(h).is_some_and(|d| d.bounds == display)
        {
            continue;
        }
        let Some(face) = visual_bounds(h) else {
            continue;
        };
        let Some(frame) = window_rect(h) else {
            continue;
        };
        // DWM frame margins vary by application and DPI. Preserve them and leave custom fullscreen
        // windows alone. A native maximized frame can extend a couple pixels beyond its work area.
        if [
            face.left - before.left,
            face.right - before.right,
            face.top - before.top,
            face.bottom - before.bottom,
        ]
        .iter()
        .any(|d| d.abs() > 3)
        {
            continue;
        }
        let target = Rect::new(
            frame.left + after.left - before.left,
            frame.top + after.top - before.top,
            frame.right + after.right - before.right,
            frame.bottom + after.bottom - before.bottom,
        );
        unsafe {
            SetWindowPos(
                h,
                null_mut(),
                target.left,
                target.top,
                target.width(),
                target.height(),
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
            )
        };
    }
}

pub(super) fn expanded_work(bar: NativeBar, old: Rect) -> Option<Rect> {
    let d = bar.display;
    let s = bar.bounds;
    let mut r = old;
    // Respect another appbar on the same edge, or a work area the shell already reclaimed.
    match bar.edge {
        Edge::Bottom if old.bottom == s.top => r.bottom = d.bottom,
        Edge::Top if old.top == s.bottom => r.top = d.top,
        Edge::Left if old.left == s.right => r.left = d.left,
        Edge::Right if old.right == s.left => r.right = d.right,
        _ => return None,
    }
    (r != old).then_some(r)
}

pub(super) fn px_to_dip(px: i32, dpi: u32) -> i32 {
    ((px as f64) * 96.0 / (dpi.max(1) as f64)).round() as i32
}

pub(super) fn reserve_after_move(bar: NativeBar, work: Rect) -> Option<Rect> {
    let mut out = work;
    match bar.edge {
        Edge::Left if work.left == bar.display.left => out.left = bar.bounds.right,
        Edge::Right if work.right == bar.display.right => out.right = bar.bounds.left,
        Edge::Top if work.top == bar.display.top => out.top = bar.bounds.bottom,
        Edge::Bottom if work.bottom == bar.display.bottom => out.bottom = bar.bounds.top,
        _ => return None,
    }
    (!out.is_empty() && out != work).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flyouts_require_shell_identity() {
        assert!(shell_surface(
            "explorer.exe",
            "NotifyIconOverflowWindow",
            false
        ));
        assert!(!shell_surface(
            "example.exe",
            "NotifyIconOverflowWindow",
            true
        ));
        assert!(!shell_surface("explorer.exe", "CabinetWClass", false));
        assert!(!shell_surface("explorer.exe", "#32768", false));
        assert!(shell_surface(
            "searchhost.exe",
            "Windows.UI.Core.CoreWindow",
            false
        ));
    }
    #[test]
    fn reclaim_only_the_taskbars_own_reserved_edge() {
        let d = Rect::new(-1200, -800, 0, 0);
        let bar = NativeBar {
            display: d,
            bounds: Rect::new(-1200, -48, 0, 0),
            edge: Edge::Bottom,
        };
        assert_eq!(
            expanded_work(bar, Rect::new(-1160, -800, 0, -48)),
            Some(Rect::new(-1160, -800, 0, 0))
        );
        assert_eq!(expanded_work(bar, Rect::new(-1200, -800, 0, -100)), None);
        assert_eq!(expanded_work(bar, d), None);
        let moved = NativeBar {
            display: d,
            bounds: Rect::new(-48, -800, 0, 0),
            edge: Edge::Right,
        };
        assert_eq!(
            reserve_after_move(moved, Rect::new(-1160, -800, 0, 0)),
            Some(Rect::new(-1160, -800, -48, 0))
        );
        assert_eq!(
            reserve_after_move(moved, Rect::new(-1160, -800, -130, 0)),
            None
        );
    }
}
