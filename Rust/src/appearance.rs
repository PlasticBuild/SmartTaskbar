use crate::{
    desktop::Monitor,
    integrations::search,
    settings::{Alignment, MediaSide, PROVIDERS, Settings, Zone},
    updates,
};
use std::{
    mem,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::mpsc,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW},
        WindowsAndMessaging::*,
    },
};

#[path = "appearance/controls.rs"]
mod controls;
#[path = "appearance/preview.rs"]
mod preview;
#[path = "appearance/zone_preview.rs"]
mod zone_preview;
use preview::Paint;
pub(crate) fn app_label_colors() -> (u32, u32) {
    let palette = unsafe { preview::Palette::load() };
    (palette.card, palette.text)
}

pub(crate) fn taskbar_menu_colors() -> Option<(u32, u32, u32)> {
    let p = unsafe { preview::Palette::load() };
    (!p.high_contrast).then_some((p.card, p.text, preview::mix(p.card, p.text, 0.13)))
}
const LAYOUT: u16 = 101;
const ALIGN: u16 = 102;
const RADIUS: u16 = 103;
const TRANSPARENCY: u16 = 104;
const HOVER: u16 = 105;
const PRESS: u16 = 106;
const ZONE: u16 = 107;
const MOTION: u16 = 108;
const SIGN_IN: u16 = 109;
const FULLSCREEN: u16 = 112;
const APPLY: u16 = 114;
const ENGINE: u16 = 115;
const MEDIA: u16 = 116;
const MEDIA_SIDE: u16 = 117;
const ICON_SIZE: u16 = 118;
const MONITOR: u16 = 119;
const OVERRIDE: u16 = 120;
const OUTSIDE: u16 = 121;
const SHARED: u16 = 122;
const TABS: u16 = 123;
const CHECK_UPDATES: u16 = 124;
const RELEASES: u16 = 125;
const REPOSITORY: u16 = 126;
const MATCH_MENU_WIDTH: u16 = 127;
const REDIRECT: u16 = 128;
const REDIRECT_PROVIDER: u16 = 129;
const REMOVE_REDIRECT: u16 = 130;
const SHOW_ZONE: u16 = 131;
const SETTINGS_SCROLL: u16 = 132;
const APP_LABEL_DELAY: u16 = 133;
const WINDOWS_TASKBAR_SETTINGS: u16 = 134;
const ZONE_DEPTH: u16 = 135;
const ZONE_WIDTH: u16 = 136;
const SHOW_APP_LABELS: u16 = 137;
const TIMER: usize = 1;
const ICON_TIMER: usize = 2;
const UPDATE_TIMER: usize = 3;
const ENGINE_TIMER: usize = 4;
const REDIRECT_TIMER: usize = 5;
const REFRESH_ZONE_PREVIEW: u32 = WM_APP + 8;
const LABEL: u16 = 1000;
const HEADER_HEIGHT: i32 = 100;
const TBM_GETPOS: u32 = WM_USER;
const LABELS: &[(u16, &str)] = &[
    (LAYOUT, "&Layout"),
    (ALIGN, "Island &position"),
    (RADIUS, "Corner &radius"),
    (TRANSPARENCY, "&Transparency"),
    (ICON_SIZE, "App icon si&ze"),
    (HOVER, "&Hover lift"),
    (PRESS, "Click &depth"),
    (MONITOR, "Reveal settings for &display"),
    (ZONE, "Activation &zone"),
    (ZONE_DEPTH, "Activation depth"),
    (ZONE_WIDTH, "Activation width"),
    (OUTSIDE, "Outside edge &delay"),
    (SHARED, "Shared edge de&lay"),
    (APP_LABEL_DELAY, "App name hover delay"),
    (MEDIA_SIDE, "Media player &side"),
    (REDIRECT_PROVIDER, "Redirect search provider"),
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Copy, Debug, Default)]
struct Box2 {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}
impl Box2 {
    fn at(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Copy)]
struct Page {
    preview: Box2,
    settings: Box2,
    footer: i32,
    width: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Appearance,
    Reveal,
    Options,
    Updates,
}

impl Tab {
    fn shows(self, id: u16) -> bool {
        match id {
            ENGINE | TABS => true,
            APPLY | WINDOWS_TASKBAR_SETTINGS => self != Self::Updates,
            CHECK_UPDATES | RELEASES | REPOSITORY => self == Self::Updates,
            LAYOUT | ALIGN | RADIUS | TRANSPARENCY | ICON_SIZE | HOVER | PRESS
            | MATCH_MENU_WIDTH => self == Self::Appearance,
            FULLSCREEN => self.preview(),
            MONITOR | OVERRIDE | ZONE | SHOW_ZONE | ZONE_DEPTH | ZONE_WIDTH | OUTSIDE | SHARED => {
                self == Self::Reveal
            }
            MOTION | SIGN_IN | SHOW_APP_LABELS | APP_LABEL_DELAY | MEDIA | MEDIA_SIDE
            | REDIRECT | REDIRECT_PROVIDER | REMOVE_REDIRECT => self == Self::Options,
            _ => false,
        }
    }

    fn preview(self) -> bool {
        matches!(self, Self::Appearance | Self::Reveal)
    }
}

impl Page {
    fn layout(width: i32, height: i32, tab: Tab) -> Self {
        let inset = if width < 600 { 16 } else { 24 };
        let usable = (width - 2 * inset).max(180);
        let show_preview = width >= 780 && tab.preview();
        let controls_height = match tab {
            Tab::Appearance => 412,
            Tab::Reveal => 393,
            Tab::Options => 400,
            Tab::Updates => 228,
        };
        let (preview, settings) = if show_preview {
            let right = (usable * 39 / 100).clamp(330, 445);
            let left = usable - right - 20;
            (
                Box2::at(inset, 102, left, (height - 155).clamp(225, 550)),
                Box2::at(inset + left + 20, 102, right, controls_height),
            )
        } else {
            (
                Box2::at(inset, 102, usable, 0),
                Box2::at(inset, 102, usable.min(650), controls_height + 45),
            )
        };
        Self {
            preview,
            settings,
            footer: (preview.y + preview.h).max(settings.y + settings.h) + 13,
            width,
        }
    }

    fn control(self, id: u16) -> Box2 {
        let r = self.settings;
        let x = r.x + 15;
        let w = r.w - 30;
        let (row, height) = match id {
            LAYOUT => (45, 25),
            ALIGN => (94, 25),
            RADIUS => (146, 25),
            TRANSPARENCY => (192, 25),
            ICON_SIZE => (238, 25),
            HOVER => (284, 25),
            PRESS => (330, 25),
            MATCH_MENU_WIDTH => (365, 23),
            MONITOR => (45, 25),
            OVERRIDE => (77, 23),
            ZONE => (123, 25),
            SHOW_ZONE => (153, 23),
            ZONE_DEPTH => (199, 25),
            ZONE_WIDTH => (247, 25),
            OUTSIDE => (295, 25),
            SHARED => (343, 25),
            SIGN_IN => (45, 23),
            MOTION => (71, 23),
            SHOW_APP_LABELS => (97, 23),
            APP_LABEL_DELAY => (145, 25),
            MEDIA => (177, 23),
            MEDIA_SIDE => (223, 25),
            REDIRECT => (261, 23),
            REDIRECT_PROVIDER => (308, 25),
            REMOVE_REDIRECT => (348, 31),
            WINDOWS_TASKBAR_SETTINGS => {
                let (card, left) = if self.preview.h > 0 {
                    (self.preview, 136)
                } else {
                    (self.settings, 13)
                };
                return Box2::at(
                    card.x + left,
                    card.y + card.h - 43,
                    240.min(card.w - left - 13),
                    29,
                );
            }
            FULLSCREEN => {
                return Box2::at(
                    self.preview.x + 13,
                    self.preview.y + self.preview.h - 43,
                    115,
                    29,
                );
            }
            APPLY => {
                return Box2::at(self.width - self.preview.x - 112, 63, 112, 32);
            }
            ENGINE => return Box2::at(self.width - self.preview.x - 142, 17, 142, 30),
            TABS => {
                return Box2::at(
                    self.preview.x,
                    63,
                    self.width - 2 * self.preview.x - 122,
                    32,
                );
            }
            CHECK_UPDATES => return Box2::at(self.preview.x + 14, 235, 118, 32),
            RELEASES => return Box2::at(self.preview.x + 140, 235, 116, 32),
            REPOSITORY => return Box2::at(self.preview.x + 14, 277, 148, 32),
            _ => return Box2::default(),
        };
        Box2::at(x, r.y + row + 10, w, height)
    }
    fn scene(self) -> Box2 {
        let p = self.preview;
        Box2::at(p.x + 12, p.y + 45, p.w - 24, (p.h - 100).max(70))
    }
}

#[derive(Default)]
struct Demo {
    full: bool,
    menu: bool,
    over_edge: bool,
    hot: Option<usize>,
    down: bool,
    shown: f32,
    open: f32,
    last: Option<Instant>,
}
impl Demo {
    fn tick(&mut self, animated: bool, elapsed: f32) -> bool {
        let visible = !self.full || self.over_edge || self.menu;
        let move_to = |v: f32, on: bool| {
            let target = if on { 1.0 } else { 0.0 };
            if !animated {
                target
            } else {
                v + (target - v).clamp(-elapsed / 0.18, elapsed / 0.18)
            }
        };
        self.shown = move_to(self.shown, visible);
        self.open = move_to(self.open, self.menu);
        self.shown != if visible { 1.0 } else { 0.0 }
            || self.open != if self.menu { 1.0 } else { 0.0 }
    }
}

struct State {
    path: PathBuf,
    old: Settings,
    draft: Settings,
    page: Page,
    scroll: i32,
    scroll_max: i32,
    dpi: i32,
    paint: Paint,
    demo: Demo,
    status: String,
    ready: bool,
    engine: bool,
    paused: bool,
    transition_until: Option<Instant>,
    monitors: Vec<Monitor>,
    monitor: Option<String>,
    tab: Tab,
    update_job: Option<mpsc::Receiver<Result<updates::Status, String>>>,
    update_status: String,
    update_available: bool,
    redirect_installed: bool,
    redirect_removal: Option<mpsc::Receiver<Result<(), String>>>,
}

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}

const WINDOW_CLASS: &str = "SmartTaskbar.Rust.Appearance";

pub fn window() -> HWND {
    unsafe { FindWindowW(wide(WINDOW_CLASS).as_ptr(), null()) }
}

pub fn show(settings_path: &Path) -> Result<(), String> {
    let _instance = match crate::SingleInstance::lock(r"Local\SmartTaskbarRust.Appearance") {
        Ok(instance) => instance,
        Err(error) => {
            let hwnd = window();
            if hwnd.is_null() {
                return Err(error);
            }
            unsafe {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
    };
    let saved = Settings::load(settings_path)?;
    unsafe {
        let _dpi = DpiContext(SetThreadDpiAwarenessContext(
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ));
        let instance = GetModuleHandleW(null());
        let class = wide(WINDOW_CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: LoadIconW(instance, std::ptr::without_provenance(1)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES | ICC_TAB_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err("Windows could not initialize the settings controls.".into());
        }
        let mut state = Box::new(State {
            path: settings_path.to_owned(),
            old: saved.clone(),
            draft: saved,
            page: Page::layout(1000, 640, Tab::Appearance),
            scroll: 0,
            scroll_max: 0,
            dpi: 96,
            paint: Paint::new(96),
            demo: Demo {
                shown: 1.0,
                ..Demo::default()
            },
            status: String::new(),
            ready: false,
            engine: crate::engine_running(),
            paused: crate::is_paused(settings_path),
            transition_until: None,
            monitors: crate::desktop::monitor_choices(),
            monitor: None,
            tab: Tab::Appearance,
            update_job: None,
            update_status: String::new(),
            update_available: false,
            redirect_installed: search::installed(settings_path),
            redirect_removal: None,
        });
        let mut area: RECT = mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut _ as _, 0);
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("SmartTaskbar Settings").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (1010 * scale / 96).min((area.right - area.left).max(440)),
            (659 * scale / 96).min((area.bottom - area.top).max(360)),
            null_mut(),
            null_mut(),
            instance,
            &mut *state as *mut _ as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        check_updates(hwnd, &mut *state);
        let mut zone_preview = zone_preview::Overlay::default();
        let mut msg: MSG = mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, null_mut(), 0, 0);
            if r == -1 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if r == 0 {
                break;
            }
            if msg.hwnd == hwnd && msg.message == REFRESH_ZONE_PREVIEW {
                let show = IsIconic(hwnd) == 0
                    && SendDlgItemMessageW(hwnd, SHOW_ZONE.into(), BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize;
                let bands = if show {
                    crate::desktop::activation_preview(
                        &state.draft,
                        &state.monitors,
                        state.monitor.as_deref(),
                    )
                } else {
                    Vec::new()
                };
                zone_preview.show(hwnd, bands);
                continue;
            }
            let prior = GetFocus();
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let current = GetFocus();
            if msg.message == WM_KEYDOWN && current != prior && IsChild(hwnd, current) != 0 {
                let id = GetDlgCtrlID(current) as u16;
                if ((LAYOUT..=SHARED).contains(&id)
                    || (MATCH_MENU_WIDTH..=SHOW_APP_LABELS).contains(&id))
                    && !matches!(id, ENGINE | APPLY)
                    && state.tab.shows(id)
                {
                    let b = state.page.control(id);
                    let mut visible = RECT::default();
                    GetClientRect(hwnd, &mut visible);
                    let view_height = visible.bottom * 96 / state.dpi;
                    if b.y - 20 < state.scroll + HEADER_HEIGHT {
                        let dy = b.y - 20 - HEADER_HEIGHT - state.scroll;
                        scroll(hwnd, &mut *state, dy);
                    } else if b.y + b.h + 8 > state.scroll + view_height {
                        let dy = b.y + b.h + 8 - state.scroll - view_height;
                        scroll(hwnd, &mut *state, dy);
                    }
                }
            }
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        // Win32 can reenter this procedure synchronously on control creation or layout.
        // Respond to these no-op messages without reborrowing the application state.
        match msg {
            WM_ERASEBKGND => return 1,
            WM_DESTROY => {
                PostQuitMessage(0);
                return 0;
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            _ => (),
        }
        match msg {
            WM_CREATE => {
                (*ptr).dpi = GetDpiForWindow(hwnd).max(96) as i32;
                create_controls(hwnd, ptr);
                0
            }
            WM_SIZE => {
                if (*ptr).ready {
                    if wp == SIZE_MINIMIZED as usize {
                        SendDlgItemMessageW(hwnd, SHOW_ZONE.into(), BM_SETCHECK, 0, 0);
                        PostMessageW(hwnd, REFRESH_ZONE_PREVIEW, 0, 0);
                    } else {
                        layout(hwnd, ptr);
                    }
                }
                0
            }
            WM_GETMINMAXINFO => {
                let limits = &mut *(lp as *mut MINMAXINFO);
                limits.ptMinTrackSize.x = 550 * (*ptr).dpi / 96;
                limits.ptMinTrackSize.y = 280 * (*ptr).dpi / 96;
                0
            }
            WM_ACTIVATE if (*ptr).ready && (wp & 0xffff) != WA_INACTIVE as usize => {
                if (*ptr).tab == Tab::Appearance && (*ptr).paint.refresh_icons() {
                    SetTimer(hwnd, ICON_TIMER, 90, None);
                }
                sync_engine(hwnd, ptr);
                redraw(hwnd);
                0
            }
            WM_PAINT => {
                preview::paint(hwnd, &*ptr);
                0
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                if (*ptr).ready {
                    (*ptr).paint.theme(hwnd);
                    style_controls(hwnd, ptr);
                    redraw(hwnd);
                }
                0
            }
            WM_DISPLAYCHANGE if (*ptr).ready => {
                (*ptr).monitors = crate::desktop::monitor_choices();
                monitor_controls(hwnd, ptr);
                PostMessageW(hwnd, REFRESH_ZONE_PREVIEW, 0, 0);
                0
            }
            WM_NOTIFY if (*ptr).ready && lp != 0 => {
                let note = &*(lp as *const NMHDR);
                if note.idFrom == TABS as usize && note.code == TCN_SELCHANGE {
                    switch_tab(hwnd, ptr);
                }
                0
            }
            WM_DPICHANGED => {
                let rect = &*(lp as *const RECT);
                (*ptr).dpi = (wp & 0xffff) as i32;
                (*ptr).paint.fonts((*ptr).dpi);
                style_controls(hwnd, ptr);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                let s = &*ptr;
                SetTextColor(wp as HDC, s.paint.palette.text);
                SetBkColor(wp as HDC, s.paint.palette.card);
                s.paint.brush as isize
            }
            WM_COMMAND if (*ptr).ready => {
                let id = (wp & 0xffff) as u16;
                let note = ((wp >> 16) & 0xffff) as u32;
                if note == CBN_SELCHANGE {
                    if id == MONITOR {
                        select_monitor(hwnd, ptr);
                    } else if matches!(
                        id,
                        LAYOUT
                            | ALIGN
                            | ZONE
                            | MEDIA_SIDE
                            | OUTSIDE
                            | SHARED
                            | REDIRECT_PROVIDER
                            | APP_LABEL_DELAY
                            | ZONE_DEPTH
                            | ZONE_WIDTH
                    ) {
                        read_controls(hwnd, ptr);
                    }
                } else if note == BN_CLICKED {
                    match id {
                        MATCH_MENU_WIDTH | MOTION | SIGN_IN | MEDIA | REDIRECT
                        | SHOW_APP_LABELS => read_controls(hwnd, ptr),
                        REMOVE_REDIRECT => remove_redirect(hwnd, ptr),
                        WINDOWS_TASKBAR_SETTINGS => open_windows_taskbar_settings(hwnd, ptr),
                        FULLSCREEN => toggle_preview(hwnd, ptr, true),
                        APPLY => save(hwnd, ptr),
                        ENGINE => engine(hwnd, ptr),
                        OVERRIDE => change_override(hwnd, ptr),
                        CHECK_UPDATES => check_updates(hwnd, ptr),
                        RELEASES | REPOSITORY => open_github(hwnd, ptr, id),
                        _ => {}
                    }
                }
                if matches!(
                    id,
                    ZONE | ZONE_DEPTH
                        | ZONE_WIDTH
                        | MONITOR
                        | OVERRIDE
                        | SHOW_ZONE
                        | LAYOUT
                        | ALIGN
                        | APPLY
                ) {
                    PostMessageW(hwnd, REFRESH_ZONE_PREVIEW, 0, 0);
                }
                0
            }
            WM_HSCROLL if (*ptr).ready => {
                read_controls(hwnd, ptr);
                0
            }
            WM_VSCROLL if lp == GetDlgItem(hwnd, SETTINGS_SCROLL.into()) as LPARAM => {
                let s = &*ptr;
                let step = match (wp & 0xffff) as i32 {
                    SB_LINEUP => -32,
                    SB_LINEDOWN => 32,
                    SB_PAGEUP => -200,
                    SB_PAGEDOWN => 200,
                    SB_TOP => -s.scroll,
                    SB_BOTTOM => s.scroll_max - s.scroll,
                    SB_THUMBTRACK => {
                        let mut info = SCROLLINFO {
                            cbSize: mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_TRACKPOS,
                            ..Default::default()
                        };
                        if GetScrollInfo(lp as HWND, SB_CTL, &mut info) != 0 {
                            info.nTrackPos - s.scroll
                        } else {
                            0
                        }
                    }
                    SB_THUMBPOSITION => ((wp >> 16) & 0xffff) as i32 - s.scroll,
                    _ => 0,
                };
                scroll(hwnd, ptr, step);
                0
            }
            WM_MOUSEWHEEL => {
                scroll(hwnd, ptr, -(((wp >> 16) as i16) as i32) * 48 / 120);
                0
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP
                if (*ptr).tab.preview() && (*ptr).page.preview.h > 0 =>
            {
                if (*ptr).ready {
                    pointer(
                        hwnd,
                        ptr,
                        msg,
                        (lp as i16) as i32,
                        ((lp >> 16) as i16) as i32,
                    );
                }
                0
            }
            WM_MOUSELEAVE | WM_CAPTURECHANGED if (*ptr).tab.preview() => {
                (*ptr).demo.hot = None;
                (*ptr).demo.over_edge = false;
                (*ptr).demo.down = false;
                update_animation(hwnd, ptr);
                0
            }
            WM_TIMER if wp == ICON_TIMER => {
                if (*ptr).paint.receive_icons() {
                    KillTimer(hwnd, ICON_TIMER);
                    redraw(hwnd);
                }
                0
            }
            WM_TIMER if wp == TIMER => {
                update_animation(hwnd, ptr);
                0
            }
            WM_TIMER if wp == UPDATE_TIMER => {
                receive_updates(hwnd, ptr);
                0
            }
            WM_TIMER if wp == ENGINE_TIMER => {
                sync_engine(hwnd, ptr);
                0
            }
            WM_TIMER if wp == REDIRECT_TIMER => {
                finish_redirect_removal(hwnd, ptr);
                0
            }
            WM_CLOSE => {
                if (*ptr).draft == (*ptr).old
                    || MessageBoxW(
                        hwnd,
                        wide("Close without applying your changes?").as_ptr(),
                        wide("SmartTaskbar Appearance").as_ptr(),
                        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                    ) == IDYES
                {
                    DestroyWindow(hwnd);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe extern "system" fn slider_wheel(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEWHEEL => {
                SendMessageW(GetParent(hwnd), msg, wp, lp);
                return 0;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(slider_wheel), id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe fn create_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut failed = false;
        for id in (LAYOUT..=TRANSPARENCY)
            .chain([ICON_SIZE])
            .chain(HOVER..=ZONE)
            .chain([SHOW_ZONE, ZONE_DEPTH, ZONE_WIDTH])
            .chain(MOTION..=SIGN_IN)
            .chain([FULLSCREEN, APPLY, ENGINE, MEDIA, MEDIA_SIDE])
            .chain(MONITOR..=REMOVE_REDIRECT)
            .chain([
                SHOW_APP_LABELS,
                APP_LABEL_DELAY,
                SETTINGS_SCROLL,
                WINDOWS_TASKBAR_SETTINGS,
            ])
        {
            if let Some((_, caption)) = LABELS.iter().find(|(control, _)| *control == id) {
                let label = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide(caption).as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    hwnd,
                    (LABEL + id) as usize as HMENU,
                    GetModuleHandleW(null()),
                    null_mut(),
                );
                failed |= label.is_null();
            }
            let (class, text, style) = match id {
                LAYOUT | ALIGN | ZONE | MEDIA_SIDE | MONITOR | OUTSIDE | SHARED
                | REDIRECT_PROVIDER | APP_LABEL_DELAY | ZONE_DEPTH | ZONE_WIDTH => (
                    "COMBOBOX",
                    match id {
                        LAYOUT => "Layout",
                        ALIGN => "Island position",
                        ZONE => "Activation zone",
                        MONITOR => "Reveal settings for display",
                        OUTSIDE => "Outside edge delay",
                        SHARED => "Shared edge delay",
                        MEDIA_SIDE => "Media player side",
                        APP_LABEL_DELAY => "App name hover delay",
                        ZONE_DEPTH => "Activation depth",
                        ZONE_WIDTH => "Activation width",
                        _ => "Redirect search provider",
                    },
                    (CBS_DROPDOWNLIST | CBS_HASSTRINGS) as u32 | WS_VSCROLL,
                ),
                RADIUS..=PRESS | ICON_SIZE => (
                    "msctls_trackbar32",
                    match id {
                        RADIUS => "Corner radius",
                        TRANSPARENCY => "Transparency",
                        HOVER => "Hover lift",
                        ICON_SIZE => "App icon size",
                        _ => "Click depth",
                    },
                    TBS_NOTICKS,
                ),
                MOTION => (
                    "BUTTON",
                    "Animate menus and media player",
                    BS_AUTOCHECKBOX as u32,
                ),
                MATCH_MENU_WIDTH => (
                    "BUTTON",
                    "Match Start to island width",
                    BS_AUTOCHECKBOX as u32,
                ),
                MEDIA => (
                    "BUTTON",
                    "Show taskbar media player",
                    BS_AUTOCHECKBOX as u32,
                ),
                REDIRECT => ("BUTTON", "Enable MSEdgeRedirect", BS_AUTOCHECKBOX as u32),
                REMOVE_REDIRECT => ("BUTTON", "Uninstall MSEdgeRedirect", BS_PUSHBUTTON as u32),
                WINDOWS_TASKBAR_SETTINGS => (
                    "BUTTON",
                    "Open Windows taskbar settings",
                    BS_PUSHBUTTON as u32,
                ),
                SIGN_IN => (
                    "BUTTON",
                    "Start SmartTaskbar when I sign in",
                    BS_AUTOCHECKBOX as u32,
                ),
                OVERRIDE => (
                    "BUTTON",
                    "Use custom reveal settings",
                    BS_AUTOCHECKBOX as u32,
                ),
                SHOW_ZONE => ("BUTTON", "Show activation zone", BS_AUTOCHECKBOX as u32),
                SHOW_APP_LABELS => ("BUTTON", "Show app names on hover", BS_AUTOCHECKBOX as u32),
                TABS => ("SysTabControl32", "", 0),
                SETTINGS_SCROLL => ("SCROLLBAR", "", SBS_VERT as u32),
                CHECK_UPDATES => ("BUTTON", "Check again", BS_PUSHBUTTON as u32),
                RELEASES => ("BUTTON", "View releases", BS_PUSHBUTTON as u32),
                REPOSITORY => ("BUTTON", "Open GitHub repository", BS_PUSHBUTTON as u32),
                FULLSCREEN => ("BUTTON", "Go fullscreen", BS_PUSHBUTTON as u32),
                ENGINE => (
                    "BUTTON",
                    if (*state).engine {
                        "Pause SmartTaskbar"
                    } else {
                        "Resume SmartTaskbar"
                    },
                    BS_PUSHBUTTON as u32,
                ),
                _ => ("BUTTON", "Apply changes", BS_DEFPUSHBUTTON as u32),
            };
            let control = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | if id == SETTINGS_SCROLL {
                        style
                    } else {
                        WS_TABSTOP | style
                    },
                0,
                0,
                1,
                1,
                hwnd,
                id as usize as HMENU,
                GetModuleHandleW(null()),
                null_mut(),
            );
            failed |= control.is_null();
            if !control.is_null() {
                failed |=
                    SetWindowSubclass(control, Some(controls::paint_control), id.into(), 0) == 0;
            }
            if !control.is_null() && matches!(id, RADIUS | TRANSPARENCY | ICON_SIZE | HOVER | PRESS)
            {
                failed |= SetWindowSubclass(control, Some(slider_wheel), id.into(), 0) == 0;
            }
        }
        if failed {
            (*state).status = "Windows could not initialize all of the settings controls.".into();
        }
        let s = &*state;
        let add = |id, items: &[&str], selected: usize| {
            for name in items {
                SendDlgItemMessageW(hwnd, id, CB_ADDSTRING, 0, wide(name).as_ptr() as isize);
            }
            SendDlgItemMessageW(hwnd, id, CB_SETCURSEL, selected, 0);
        };
        add(
            LAYOUT as i32,
            &["Full width", "Floating island"],
            s.draft.island as usize,
        );
        add(
            ALIGN as i32,
            &["Left", "Center", "Right"],
            match s.draft.alignment {
                Alignment::Left => 0,
                Alignment::Center => 1,
                Alignment::Right => 2,
            },
        );
        add(
            ZONE as i32,
            &["Entire edge", "Center half", "Taskbar width"],
            0,
        );
        add(
            MEDIA_SIDE as i32,
            &["Left", "Right"],
            usize::from(s.draft.media_side == MediaSide::Right),
        );
        add(
            REDIRECT_PROVIDER as i32,
            PROVIDERS,
            PROVIDERS
                .iter()
                .position(|provider| *provider == s.draft.web_engine)
                .unwrap_or(0),
        );
        SendDlgItemMessageW(
            hwnd,
            ICON_SIZE as i32,
            TBM_SETRANGEMIN,
            0,
            Settings::MIN_ICON_SIZE as isize,
        );
        for (id, max, value) in [
            (RADIUS, 24, s.draft.radius),
            (TRANSPARENCY, 85, s.draft.transparency),
            (
                ICON_SIZE,
                Settings::MAX_ICON_SIZE as isize,
                s.draft.icon_size,
            ),
            (HOVER, 8, s.draft.hover_lift),
            (PRESS, 5, s.draft.click_depth),
        ] {
            SendDlgItemMessageW(hwnd, id as i32, TBM_SETRANGEMAX, 0, max);
            SendDlgItemMessageW(
                hwnd,
                id as i32,
                TBM_SETPAGESIZE,
                0,
                if matches!(id, TRANSPARENCY | ICON_SIZE) {
                    5
                } else {
                    1
                },
            );
            SendDlgItemMessageW(hwnd, id as i32, TBM_SETPOS, 1, value as isize);
        }
        for (id, on) in [
            (MATCH_MENU_WIDTH, s.draft.match_start_width),
            (REDIRECT, s.redirect_installed && s.draft.search_redirect),
            (MOTION, s.draft.animate),
            (SIGN_IN, s.draft.start_at_sign_in),
            (MEDIA, s.draft.media_player),
            (SHOW_APP_LABELS, s.draft.show_app_labels),
        ] {
            SendDlgItemMessageW(hwnd, id as i32, BM_SETCHECK, on as usize, 0);
        }
        (*state).paint.fonts((*state).dpi);
        (*state).paint.theme(hwnd);
        tab_item(hwnd, TCM_INSERTITEMW, 0, "Appearance");
        tab_item(hwnd, TCM_INSERTITEMW, 1, "Reveal");
        tab_item(hwnd, TCM_INSERTITEMW, 2, "Options");
        tab_item(hwnd, TCM_INSERTITEMW, 3, "Updates");
        style_controls(hwnd, state);
        (*state).ready = true;
        monitor_controls(hwnd, state);
        layout(hwnd, state);
        reflect(hwnd, state);
    }
}

unsafe fn style_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let dark = (*state).paint.palette.dark;
        for &(id, _) in LABELS {
            SendDlgItemMessageW(
                hwnd,
                (LABEL + id) as i32,
                WM_SETFONT,
                (*state).paint.normal as usize,
                1,
            );
        }
        let theme = wide(if dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        for id in LAYOUT..=SHOW_APP_LABELS {
            let c = GetDlgItem(hwnd, id as i32);
            if c.is_null() {
                continue;
            }
            SetWindowTheme(c, theme.as_ptr(), null());
            SendMessageW(c, WM_SETFONT, (*state).paint.normal as usize, 1);
            if matches!(
                id,
                LAYOUT
                    | ALIGN
                    | ZONE
                    | MEDIA_SIDE
                    | MONITOR
                    | OUTSIDE
                    | SHARED
                    | REDIRECT_PROVIDER
                    | APP_LABEL_DELAY
                    | ZONE_DEPTH
                    | ZONE_WIDTH
            ) {
                let mut combo = COMBOBOXINFO {
                    cbSize: mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                if GetComboBoxInfo(c, &mut combo) != 0 && !combo.hwndList.is_null() {
                    SetWindowTheme(combo.hwndList, theme.as_ptr(), null());
                }
            }
        }
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        let dpi = (*state).dpi;
        let w = client.right * 96 / dpi;
        let h = client.bottom * 96 / dpi;
        let page = Page::layout(w.saturating_sub(14), h, (*state).tab);
        let total = if (*state).tab == Tab::Updates {
            344
        } else {
            page.footer + 32
        };
        (*state).page = page;
        (*state).scroll_max = (total - h).max(0);
        (*state).scroll = (*state).scroll.clamp(0, (*state).scroll_max);
        let info = SCROLLINFO {
            cbSize: mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: total - 1,
            nPage: h.max(1) as u32,
            nPos: (*state).scroll,
            nTrackPos: 0,
        };
        let scrollbar = GetDlgItem(hwnd, SETTINGS_SCROLL.into());
        SetWindowPos(
            scrollbar,
            null_mut(),
            (w - 14).max(0) * dpi / 96,
            0,
            14 * dpi / 96,
            client.bottom,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        SetScrollInfo(scrollbar, SB_CTL, &info, 1);
        ShowWindow(
            scrollbar,
            if (*state).scroll_max == 0 {
                SW_HIDE
            } else {
                SW_SHOWNOACTIVATE
            },
        );
        position_controls(hwnd, &*state);
    }
}

unsafe fn position_controls(hwnd: HWND, state: &State) {
    unsafe {
        let State {
            page,
            dpi,
            scroll: offset,
            tab,
            ..
        } = *state;
        let mut placements =
            Vec::with_capacity((APP_LABEL_DELAY - LAYOUT + 1) as usize + LABELS.len());
        for id in (LAYOUT..=SHOW_APP_LABELS).filter(|&id| id != SETTINGS_SCROLL) {
            let b = page.control(id);
            let label = GetDlgItem(hwnd, (LABEL + id) as i32);
            let fixed = matches!(id, ENGINE | TABS | APPLY);
            let y = b.y - if fixed { 0 } else { offset };
            let show = tab.shows(id)
                && (fixed || y > HEADER_HEIGHT)
                && (id != FULLSCREEN || page.preview.h > 0);
            let control = GetDlgItem(hwnd, id as i32);
            if !label.is_null() {
                let y_label = y - 19;
                placements.push((
                    label,
                    Box2::at(b.x, y_label, b.w - 50, 18),
                    show && y_label > HEADER_HEIGHT,
                ));
            }
            let height = if matches!(
                id,
                LAYOUT
                    | ALIGN
                    | ZONE
                    | MEDIA_SIDE
                    | MONITOR
                    | OUTSIDE
                    | SHARED
                    | REDIRECT_PROVIDER
                    | APP_LABEL_DELAY
                    | ZONE_DEPTH
                    | ZONE_WIDTH
            ) {
                180
            } else {
                b.h
            };
            if !control.is_null() {
                placements.push((control, Box2::at(b.x, y, b.w, height), show));
            }
        }

        // Move the entire control list without copying pixels from its old
        // position, then invalidate the parent and controls together.
        let px = |dip| dip * dpi / 96;
        let flags = |show| {
            SWP_NOACTIVATE
                | SWP_NOZORDER
                | SWP_NOCOPYBITS
                | SWP_NOREDRAW
                | if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW }
        };
        let mut batch = BeginDeferWindowPos(placements.len() as i32);
        for &(control, b, show) in &placements {
            if batch.is_null() {
                break;
            }
            batch = DeferWindowPos(
                batch,
                control,
                null_mut(),
                px(b.x),
                px(b.y),
                px(b.w),
                px(b.h),
                flags(show),
            );
        }
        if batch.is_null() || EndDeferWindowPos(batch) == 0 {
            for (control, b, show) in placements {
                SetWindowPos(
                    control,
                    null_mut(),
                    px(b.x),
                    px(b.y),
                    px(b.w),
                    px(b.h),
                    flags(show),
                );
            }
        }
        RedrawWindow(
            hwnd,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_ERASE,
        );
    }
}

unsafe fn scroll(hwnd: HWND, state: *mut State, delta: i32) {
    unsafe {
        let pos = (*state)
            .scroll
            .saturating_add(delta)
            .clamp(0, (*state).scroll_max);
        if pos != (*state).scroll {
            (*state).scroll = pos;
            SetScrollPos(GetDlgItem(hwnd, SETTINGS_SCROLL.into()), SB_CTL, pos, 1);
            position_controls(hwnd, &*state);
        }
    }
}

unsafe fn monitor_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let combo = GetDlgItem(hwnd, MONITOR as i32);
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        SendMessageW(
            combo,
            CB_ADDSTRING,
            0,
            wide("All displays · defaults").as_ptr() as isize,
        );
        let mut selected = 0;
        for (i, monitor) in s.monitors.iter().enumerate() {
            SendMessageW(
                combo,
                CB_ADDSTRING,
                0,
                wide(&monitor.label).as_ptr() as isize,
            );
            if s.monitor.as_deref() == Some(monitor.id.as_str()) {
                selected = i + 1;
            }
        }
        if selected == 0 {
            s.monitor = None;
        }
        SendMessageW(combo, CB_SETCURSEL, selected, 0);
        reveal_controls(hwnd, state);
    }
}

unsafe fn select_monitor(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let selected = SendDlgItemMessageW(hwnd, MONITOR as i32, CB_GETCURSEL, 0, 0);
        s.monitor = selected
            .checked_sub(1)
            .and_then(|i| s.monitors.get(i as usize))
            .map(|m| m.id.clone());
        reveal_controls(hwnd, state);
    }
}

unsafe fn reveal_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &*state;
        let reveal = s.draft.reveal(s.monitor.as_deref());
        let custom = s
            .monitor
            .as_ref()
            .is_some_and(|key| s.draft.monitor_reveal.contains_key(key));
        let override_box = GetDlgItem(hwnd, OVERRIDE as i32);
        EnableWindow(override_box, s.monitor.is_some() as i32);
        SendMessageW(override_box, BM_SETCHECK, custom as usize, 0);
        let zone = match reveal.zone {
            Zone::Entire => 0,
            Zone::Center => 1,
            Zone::Island => 2,
        };
        SendDlgItemMessageW(hwnd, ZONE as i32, CB_SETCURSEL, zone, 0);
        let editable = (s.monitor.is_none() || custom) as i32;
        EnableWindow(GetDlgItem(hwnd, ZONE as i32), editable);
        for (id, selected, options) in [
            (
                ZONE_DEPTH,
                reveal.depth_dip,
                &[2, 4, 6, 8, 12, 16, 20, 24, 32, 48, 64][..],
            ),
            (
                ZONE_WIDTH,
                reveal.width_percent,
                &[10, 25, 50, 75, 90, 100, 125, 150, 175, 200][..],
            ),
            (
                OUTSIDE,
                reveal.outside_ms,
                &[0, 80, 120, 180, 250, 400, 600, 800, 1000][..],
            ),
            (
                SHARED,
                reveal.shared_ms,
                &[150, 220, 300, 400, 600, 800, 1000][..],
            ),
            (
                APP_LABEL_DELAY,
                s.draft.app_label_delay_ms,
                &[0, 100, 150, 250, 350, 500, 750, 1000, 1500][..],
            ),
        ] {
            let combo = GetDlgItem(hwnd, id as i32);
            SendMessageW(combo, CB_RESETCONTENT, 0, 0);
            let mut values = options.to_vec();
            if !values.contains(&selected) {
                values.push(selected);
                values.sort_unstable();
            }
            for (index, ms) in values.into_iter().enumerate() {
                let label = match id {
                    ZONE_DEPTH => format!("{ms} px at 100% scaling"),
                    ZONE_WIDTH => format!("{ms}% of preset width"),
                    _ if ms == 0 => "Instant".into(),
                    _ => format!("{ms} ms"),
                };
                SendMessageW(combo, CB_ADDSTRING, 0, wide(&label).as_ptr() as isize);
                SendMessageW(combo, CB_SETITEMDATA, index, ms as isize);
                if ms == selected {
                    SendMessageW(combo, CB_SETCURSEL, index, 0);
                }
            }
            EnableWindow(combo, if id == APP_LABEL_DELAY { 1 } else { editable });
        }
        reflect(hwnd, state);
    }
}

unsafe fn change_override(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        if let Some(key) = &s.monitor {
            if SendDlgItemMessageW(hwnd, OVERRIDE as i32, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
            {
                let inherited = s.draft.reveal(None);
                s.draft
                    .monitor_reveal
                    .entry(key.clone())
                    .or_insert(inherited);
            } else {
                s.draft.monitor_reveal.remove(key);
            }
        }
        reveal_controls(hwnd, state);
    }
}

unsafe fn read_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let sel = |id| SendDlgItemMessageW(hwnd, id, CB_GETCURSEL, 0, 0);
        let slider = |id| SendDlgItemMessageW(hwnd, id, TBM_GETPOS, 0, 0) as u8;
        let check = |id| SendDlgItemMessageW(hwnd, id, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        let s = &mut *state;
        s.draft.island = sel(LAYOUT as i32) == 1;
        s.draft.match_start_width = check(MATCH_MENU_WIDTH as i32);
        s.draft.alignment = match sel(ALIGN as i32) {
            0 => Alignment::Left,
            2 => Alignment::Right,
            _ => Alignment::Center,
        };
        let delay = |id| {
            u16::try_from(SendDlgItemMessageW(
                hwnd,
                id,
                CB_GETITEMDATA,
                sel(id) as usize,
                0,
            ))
            .ok()
        };
        let mut reveal = s.draft.reveal(s.monitor.as_deref());
        reveal.zone = match sel(ZONE as i32) {
            1 => Zone::Center,
            2 => Zone::Island,
            _ => Zone::Entire,
        };
        reveal.outside_ms = delay(OUTSIDE as i32).unwrap_or(reveal.outside_ms);
        reveal.shared_ms = delay(SHARED as i32).unwrap_or(reveal.shared_ms);
        reveal.depth_dip = delay(ZONE_DEPTH.into()).unwrap_or(reveal.depth_dip);
        reveal.width_percent = delay(ZONE_WIDTH.into()).unwrap_or(reveal.width_percent);
        s.draft.app_label_delay_ms =
            delay(APP_LABEL_DELAY.into()).unwrap_or(s.draft.app_label_delay_ms);
        s.draft.show_app_labels = check(SHOW_APP_LABELS.into());
        if let Some(key) = &s.monitor {
            if let Some(setting) = s.draft.monitor_reveal.get_mut(key) {
                *setting = reveal;
            }
        } else {
            s.draft.zone = reveal.zone;
            s.draft.activation_depth_dip = reveal.depth_dip;
            s.draft.activation_width_percent = reveal.width_percent;
            s.draft.outside_dwell_ms = reveal.outside_ms;
            s.draft.shared_dwell_ms = reveal.shared_ms;
        }
        s.draft.radius = slider(RADIUS as i32);
        s.draft.transparency = slider(TRANSPARENCY as i32);
        s.draft.icon_size = slider(ICON_SIZE as i32);
        s.draft.hover_lift = slider(HOVER as i32);
        s.draft.click_depth = slider(PRESS as i32);
        s.draft.animate = check(MOTION as i32);
        s.draft.start_at_sign_in = check(SIGN_IN as i32);
        if s.redirect_removal.is_none() {
            s.draft.search_redirect = s.redirect_installed && check(REDIRECT as i32);
            if let Some(provider) = PROVIDERS.get(sel(REDIRECT_PROVIDER as i32) as usize) {
                s.draft.web_engine = (*provider).to_owned();
            }
        }
        s.draft.media_player = check(MEDIA as i32);
        s.draft.media_side = if sel(MEDIA_SIDE as i32) == 0 {
            MediaSide::Left
        } else {
            MediaSide::Right
        };
        s.status.clear();
        reflect(hwnd, state);
        update_animation(hwnd, state);
    }
}
unsafe fn reflect(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &*state;
        EnableWindow(
            GetDlgItem(hwnd, APP_LABEL_DELAY.into()),
            s.draft.show_app_labels as i32,
        );
        EnableWindow(GetDlgItem(hwnd, ALIGN as i32), s.draft.island as i32);
        EnableWindow(
            GetDlgItem(hwnd, MATCH_MENU_WIDTH as i32),
            s.draft.island as i32,
        );
        EnableWindow(
            GetDlgItem(hwnd, MEDIA_SIDE as i32),
            s.draft.media_player as i32,
        );
        let redirect_available = s.redirect_installed && s.redirect_removal.is_none();
        for (id, enabled) in [
            (REDIRECT, redirect_available),
            (
                REDIRECT_PROVIDER,
                redirect_available && s.draft.search_redirect,
            ),
            (REMOVE_REDIRECT, redirect_available),
            (APPLY, s.redirect_removal.is_none() && s.draft != s.old),
        ] {
            EnableWindow(GetDlgItem(hwnd, id as i32), enabled as i32);
        }
        let caption = if s.redirect_installed {
            "Enable MSEdgeRedirect"
        } else {
            "MSEdgeRedirect (install via Setup)"
        };
        SetWindowTextW(GetDlgItem(hwnd, REDIRECT as i32), wide(caption).as_ptr());
        redraw(hwnd);
    }
}

unsafe fn remove_redirect(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        if !s.redirect_installed || s.redirect_removal.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let path = s.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(search::uninstall(&path));
        });
        s.redirect_removal = Some(rx);
        s.status = "Uninstalling MSEdgeRedirect.".into();
        SetTimer(hwnd, REDIRECT_TIMER, 125, None);
        reflect(hwnd, state);
    }
}

unsafe fn finish_redirect_removal(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let Some(job) = &s.redirect_removal else {
            return;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("MSEdgeRedirect removal was interrupted.".into())
            }
        };
        KillTimer(hwnd, REDIRECT_TIMER);
        s.redirect_removal = None;
        s.redirect_installed = search::installed(&s.path);
        if let Ok(saved) = Settings::load(&s.path) {
            s.old.search_redirect = saved.search_redirect;
            s.draft.search_redirect = saved.search_redirect;
            SendDlgItemMessageW(
                hwnd,
                REDIRECT as i32,
                BM_SETCHECK,
                (s.redirect_installed && saved.search_redirect) as usize,
                0,
            );
        }
        s.status = result
            .err()
            .unwrap_or_else(|| "MSEdgeRedirect uninstalled.".into());
        reflect(hwnd, state);
    }
}
unsafe fn save(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &*state;
        if s.old == s.draft {
            return;
        }
        let result = crate::apply_settings(&s.path, &s.old, &s.draft);
        match result {
            Ok(()) => {
                (*state).old = (*state).draft.clone();
                (*state).status = "Changes applied.".into();
            }
            Err(error) => {
                (*state).status = "Changes could not be applied.".into();
                MessageBoxW(
                    hwnd,
                    wide(&error).as_ptr(),
                    wide("SmartTaskbar Appearance").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        reflect(hwnd, state);
    }
}

unsafe fn engine(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let result = if crate::engine_running() {
            crate::pause(&s.path)
        } else {
            crate::resume(&s.path)
        };
        match result {
            Ok(()) => {
                s.transition_until = Some(Instant::now() + std::time::Duration::from_secs(30));
                SetTimer(hwnd, ENGINE_TIMER, 250, None);
            }
            Err(error) => s.status = error,
        }
        sync_engine(hwnd, state);
    }
}

unsafe fn sync_engine(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        s.paused = crate::is_paused(&s.path);
        s.engine = crate::engine_running();
        let settling = s.paused && s.engine;
        if settling && s.transition_until.is_none() {
            s.transition_until = Some(Instant::now() + std::time::Duration::from_secs(30));
        }
        let pausing = settling
            && s.transition_until
                .is_some_and(|until| Instant::now() < until);
        if !pausing {
            KillTimer(hwnd, ENGINE_TIMER);
            if !settling {
                s.transition_until = None;
            }
        } else {
            SetTimer(hwnd, ENGINE_TIMER, 250, None);
        }
        let button = GetDlgItem(hwnd, ENGINE as i32);
        SetWindowTextW(
            button,
            wide(if pausing {
                "Pausing…"
            } else if s.engine {
                "Pause SmartTaskbar"
            } else {
                "Resume SmartTaskbar"
            })
            .as_ptr(),
        );
        EnableWindow(button, (!pausing) as i32);
        redraw(hwnd);
    }
}

unsafe fn tab_item(hwnd: HWND, message: u32, index: usize, caption: &str) {
    unsafe {
        let mut name = wide(caption);
        let mut item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: name.as_mut_ptr(),
            ..Default::default()
        };
        SendDlgItemMessageW(
            hwnd,
            TABS as i32,
            message,
            index,
            (&mut item as *mut TCITEMW) as isize,
        );
    }
}

unsafe fn switch_tab(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let selected = match SendDlgItemMessageW(hwnd, TABS as i32, TCM_GETCURSEL, 0, 0) {
            0 => Tab::Appearance,
            1 => Tab::Reveal,
            2 => Tab::Options,
            3 => Tab::Updates,
            _ => return,
        };
        if selected == s.tab {
            return;
        }
        if s.tab.preview() {
            s.demo.hot = None;
            s.demo.down = false;
            s.demo.over_edge = false;
            s.demo.last = None;
            KillTimer(hwnd, TIMER);
            KillTimer(hwnd, ICON_TIMER);
        }
        s.tab = selected;
        s.scroll = 0;
        if selected.preview() {
            if !s.paint.receive_icons() || s.paint.refresh_icons() {
                SetTimer(hwnd, ICON_TIMER, 90, None);
            }
            update_animation(hwnd, state);
        }
        layout(hwnd, state);
    }
}

unsafe fn check_updates(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        if s.update_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let launched = std::thread::Builder::new()
            .name("SmartTaskbar updates".into())
            .spawn(move || {
                let _ = tx.send(updates::check());
            });
        if launched.is_ok() {
            s.update_job = Some(rx);
            s.update_status = "Checking GitHub…".into();
            EnableWindow(GetDlgItem(hwnd, CHECK_UPDATES as i32), 0);
            SetTimer(hwnd, UPDATE_TIMER, 175, None);
        } else {
            s.update_status = "The update check could not start.".into();
        }
        redraw(hwnd);
    }
}

unsafe fn receive_updates(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let Some(job) = &s.update_job else { return };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The update check could not finish.".into())
            }
        };
        s.update_job = None;
        KillTimer(hwnd, UPDATE_TIMER);
        EnableWindow(GetDlgItem(hwnd, CHECK_UPDATES as i32), 1);
        if let Ok(status) = &result {
            s.update_available = matches!(status, updates::Status::New(_));
        }
        s.update_status = match result {
            Ok(updates::Status::New(version)) => format!("Version {version} is available."),
            Ok(updates::Status::Current) => "You're up to date.".into(),
            Ok(updates::Status::NoRelease) => "There are no published stable releases yet.".into(),
            Err(error) => error,
        };
        tab_item(
            hwnd,
            TCM_SETITEMW,
            3,
            if s.update_available {
                "Updates • New"
            } else {
                "Updates"
            },
        );
        redraw(hwnd);
    }
}

unsafe fn open_windows_taskbar_settings(hwnd: HWND, state: *mut State) {
    unsafe {
        let status = ShellExecuteW(
            hwnd,
            windows_sys::w!("open"),
            windows_sys::w!("ms-settings:taskbar"),
            null(),
            null(),
            SW_SHOWNORMAL,
        );
        if status as isize <= 32 {
            (*state).status = "Windows taskbar settings could not be opened.".into();
            redraw(hwnd);
        }
    }
}

unsafe fn open_github(hwnd: HWND, state: *mut State, id: u16) {
    unsafe {
        let target = if id == RELEASES {
            format!("{}/releases", updates::REPOSITORY)
        } else {
            updates::REPOSITORY.into()
        };
        if ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(&target).as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        ) as usize
            <= 32
        {
            (*state).update_status = "Windows could not open GitHub in the browser.".into();
            redraw(hwnd);
        }
    }
}

unsafe fn toggle_preview(hwnd: HWND, state: *mut State, fullscreen: bool) {
    unsafe {
        let demo = &mut (*state).demo;
        if fullscreen {
            demo.full = !demo.full;
            demo.menu = false;
            demo.over_edge = false;
            demo.hot = None;
        } else {
            demo.menu = !demo.menu;
        }
        SetWindowTextW(
            GetDlgItem(hwnd, FULLSCREEN as i32),
            wide(if demo.full {
                "Exit fullscreen"
            } else {
                "Go fullscreen"
            })
            .as_ptr(),
        );
        update_animation(hwnd, state);
    }
}
unsafe fn update_animation(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &mut *state;
        let now = Instant::now();
        let dt = s
            .demo
            .last
            .map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
        if s.demo.tick(s.draft.animate, dt) {
            s.demo.last = Some(now);
            SetTimer(hwnd, TIMER, 16, None);
        } else {
            s.demo.last = None;
            KillTimer(hwnd, TIMER);
        }
        redraw(hwnd);
    }
}
unsafe fn pointer(hwnd: HWND, state: *mut State, msg: u32, px: i32, py: i32) {
    unsafe {
        let s = &mut *state;
        let x = px * 96 / s.dpi;
        let y = py * 96 / s.dpi + s.scroll;
        let geom = preview::geometry(s);
        let on_bar = geom.bar.contains(x, y);
        let touch = geom.activation.contains(x, y);
        let hot = geom.icons.iter().position(|b| b.contains(x, y));
        let over = (s.demo.shown > 0.2 && on_bar) || touch;
        let changed = hot != s.demo.hot || s.demo.over_edge != over;
        s.demo.hot = hot;
        s.demo.over_edge = over;
        match msg {
            WM_MOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
                if changed {
                    update_animation(hwnd, state);
                }
            }
            WM_LBUTTONDOWN if on_bar => {
                s.demo.down = true;
                SetCapture(hwnd);
                redraw(hwnd);
            }
            WM_LBUTTONUP => {
                let clicked = s.demo.down && hot == Some(0);
                s.demo.down = false;
                if GetCapture() == hwnd {
                    ReleaseCapture();
                }
                if clicked {
                    toggle_preview(hwnd, state, false);
                } else {
                    redraw(hwnd);
                }
            }
            _ => {}
        }
    }
}
unsafe fn redraw(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trackbar_forwards_the_wheel_without_changing_its_position() {
        unsafe extern "system" fn count_wheel(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                if msg == WM_MOUSEWHEEL {
                    *(data as *mut i32) = (wp >> 16) as i16 as i32;
                    return 0;
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        unsafe {
            assert_ne!(
                InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                    dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                    dwICC: ICC_BAR_CLASSES,
                }),
                0
            );
            let instance = GetModuleHandleW(null());
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                200,
                60,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!parent.is_null());
            let slider = CreateWindowExW(
                0,
                wide("msctls_trackbar32").as_ptr(),
                null(),
                WS_CHILD,
                0,
                0,
                150,
                35,
                parent,
                null_mut(),
                instance,
                null(),
            );
            assert!(!slider.is_null());
            let mut observed = 0;
            assert_ne!(
                SetWindowSubclass(
                    parent,
                    Some(count_wheel),
                    1,
                    (&mut observed as *mut i32) as usize
                ),
                0
            );
            assert_ne!(SetWindowSubclass(slider, Some(slider_wheel), 1, 0), 0);
            SendMessageW(slider, TBM_SETRANGEMAX, 0, 100);
            SendMessageW(slider, TBM_SETPOS, 0, 50);
            SendMessageW(
                slider,
                WM_MOUSEWHEEL,
                usize::from((-120i16) as u16) << 16,
                0,
            );
            assert_eq!(observed, -120);
            assert_eq!(SendMessageW(slider, TBM_GETPOS, 0, 0), 50);
            SendMessageW(slider, WM_KEYDOWN, VK_RIGHT.into(), 0);
            assert_eq!(SendMessageW(slider, TBM_GETPOS, 0, 0), 51);
            DestroyWindow(parent);
        }
    }

    #[test]
    fn settings_tabs_fit_without_scrolling_at_normal_sizes() {
        let wide = Page::layout(1000, 640, Tab::Appearance);
        let narrow = Page::layout(600, 640, Tab::Appearance);
        assert!(wide.settings.x >= wide.preview.x + wide.preview.w);
        assert_eq!(narrow.preview.h, 0);
        assert!(narrow.control(APPLY).y < narrow.preview.y);
        let tabs = narrow.control(TABS);
        assert!(tabs.x + tabs.w < narrow.control(APPLY).x);
        for tab in [Tab::Appearance, Tab::Reveal, Tab::Options] {
            for width in [600, 1000] {
                let page = Page::layout(width, 640, tab);
                assert!(page.footer + 32 <= 640);
                let button = page.control(WINDOWS_TASKBAR_SETTINGS);
                assert!(button.y + button.h < page.footer);
            }
        }
    }
    #[test]
    fn preview_keeps_menu_visible_until_closed_and_honors_reduced_motion() {
        let mut d = Demo {
            full: true,
            menu: true,
            ..Demo::default()
        };
        assert!(d.tick(true, 0.09));
        assert_eq!((d.shown, d.open), (0.5, 0.5));
        assert!(!d.tick(false, 0.0));
        assert_eq!((d.shown, d.open), (1.0, 1.0));
        d.menu = false;
        assert!(!d.tick(false, 0.0));
        assert_eq!((d.shown, d.open), (0.0, 0.0));
    }
}
