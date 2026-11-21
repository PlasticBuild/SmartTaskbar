use crate::{
    desktop::Monitor,
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
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

#[path = "appearance/preview.rs"]
mod preview;
use preview::Paint;

const LAYOUT: u16 = 101;
const ALIGN: u16 = 102;
const RADIUS: u16 = 103;
const TRANSPARENCY: u16 = 104;
const HOVER: u16 = 105;
const PRESS: u16 = 106;
const ZONE: u16 = 107;
const MOTION: u16 = 108;
const SIGN_IN: u16 = 109;
const SEARCH: u16 = 110;
const PROVIDER: u16 = 111;
const FULLSCREEN: u16 = 112;
const MENU: u16 = 113;
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
const TIMER: usize = 1;
const ICON_TIMER: usize = 2;
const UPDATE_TIMER: usize = 3;
const ENGINE_TIMER: usize = 4;
const LABEL: u16 = 1000;
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
    (OUTSIDE, "Outside edge &delay"),
    (SHARED, "Shared edge de&lay"),
    (PROVIDER, "Search pro&vider"),
    (MEDIA_SIDE, "Media player &side"),
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

impl Page {
    fn layout(width: i32, height: i32) -> Self {
        let inset = if width < 600 { 16 } else { 24 };
        let usable = (width - 2 * inset).max(180);
        let (preview, settings) = if width >= 780 {
            let right = (usable * 39 / 100).clamp(330, 445);
            let left = usable - right - 20;
            (
                Box2::at(inset, 102, left, (height - 175).clamp(360, 610)),
                Box2::at(inset + left + 20, 102, right, 820),
            )
        } else {
            let h = (usable * 53 / 100).clamp(215, 340);
            (
                Box2::at(inset, 102, usable, h),
                Box2::at(inset, 102 + h + 14, usable, 820),
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
            MONITOR => (380, 25),
            OVERRIDE => (411, 23),
            ZONE => (459, 25),
            OUTSIDE => (507, 25),
            SHARED => (555, 25),
            MOTION => (587, 23),
            SIGN_IN => (613, 23),
            SEARCH => (639, 23),
            PROVIDER => (686, 25),
            MEDIA => (722, 23),
            MEDIA_SIDE => (768, 25),
            FULLSCREEN => {
                return Box2::at(
                    self.preview.x + 13,
                    self.preview.y + self.preview.h - 43,
                    115,
                    29,
                );
            }
            MENU => {
                return Box2::at(
                    self.preview.x + 135,
                    self.preview.y + self.preview.h - 43,
                    112,
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
        Box2::at(p.x + 12, p.y + 45, p.w - 24, (p.h - 100).max(80))
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
    choices: Vec<String>,
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
    updates_tab: bool,
    settings_scroll: i32,
    update_job: Option<mpsc::Receiver<Result<updates::Status, String>>>,
    update_status: String,
    update_available: bool,
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
    let mut choices: Vec<_> = PROVIDERS.iter().map(|p| p.to_string()).collect();
    if !choices.contains(&saved.web_engine) {
        choices.push(saved.web_engine.clone());
    }
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
            choices,
            page: Page::layout(1000, 640),
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
            updates_tab: false,
            settings_scroll: 0,
            update_job: None,
            update_status: String::new(),
            update_available: false,
        });
        let mut area: RECT = mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut _ as _, 0);
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("SmartTaskbar Settings").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (1010 * scale / 96).min((area.right - area.left).max(440)),
            (695 * scale / 96).min((area.bottom - area.top).max(360)),
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
        let mut msg: MSG = mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, null_mut(), 0, 0);
            if r == -1 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if r == 0 {
                break;
            }
            let prior = GetFocus();
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let current = GetFocus();
            if msg.message == WM_KEYDOWN && current != prior && IsChild(hwnd, current) != 0 {
                let id = GetDlgCtrlID(current) as u16;
                if (LAYOUT..=SHARED).contains(&id) && id != ENGINE && !state.updates_tab {
                    let b = state.page.control(id);
                    let mut visible = RECT::default();
                    GetClientRect(hwnd, &mut visible);
                    let view_height = visible.bottom * 96 / state.dpi;
                    if b.y - 20 < state.scroll {
                        let dy = b.y - 20 - state.scroll;
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
                    layout(hwnd, ptr);
                }
                0
            }
            WM_GETMINMAXINFO => {
                let limits = &mut *(lp as *mut MINMAXINFO);
                limits.ptMinTrackSize.x = 360 * (*ptr).dpi / 96;
                limits.ptMinTrackSize.y = 280 * (*ptr).dpi / 96;
                0
            }
            WM_ACTIVATE if (*ptr).ready && (wp & 0xffff) != WA_INACTIVE as usize => {
                if !(*ptr).updates_tab && (*ptr).paint.refresh_icons() {
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
                        LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE | OUTSIDE | SHARED
                    ) {
                        read_controls(hwnd, ptr);
                    }
                } else if note == BN_CLICKED {
                    match id {
                        MOTION | SIGN_IN | SEARCH | MEDIA => read_controls(hwnd, ptr),
                        MENU | FULLSCREEN => toggle_preview(hwnd, ptr, id),
                        APPLY => save(hwnd, ptr),
                        ENGINE => engine(hwnd, ptr),
                        OVERRIDE => change_override(hwnd, ptr),
                        CHECK_UPDATES => check_updates(hwnd, ptr),
                        RELEASES | REPOSITORY => open_github(hwnd, ptr, id),
                        _ => {}
                    }
                }
                0
            }
            WM_HSCROLL if (*ptr).ready => {
                read_controls(hwnd, ptr);
                0
            }
            WM_VSCROLL => {
                let s = &*ptr;
                let step = match (wp & 0xffff) as i32 {
                    SB_LINEUP => -32,
                    SB_LINEDOWN => 32,
                    SB_PAGEUP => -200,
                    SB_PAGEDOWN => 200,
                    SB_TOP => -s.scroll,
                    SB_BOTTOM => s.scroll_max - s.scroll,
                    SB_THUMBPOSITION | SB_THUMBTRACK => ((wp >> 16) & 0xffff) as i32 - s.scroll,
                    _ => 0,
                };
                scroll(hwnd, ptr, step);
                0
            }
            WM_MOUSEWHEEL => {
                scroll(hwnd, ptr, -(((wp >> 16) as i16) as i32) * 48 / 120);
                0
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP if !(*ptr).updates_tab => {
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
            WM_MOUSELEAVE | WM_CAPTURECHANGED if !(*ptr).updates_tab => {
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

unsafe fn create_controls(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut failed = false;
        for id in (LAYOUT..=TRANSPARENCY)
            .chain([ICON_SIZE])
            .chain(HOVER..=MEDIA_SIDE)
            .chain(MONITOR..=REPOSITORY)
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
                LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE | MONITOR | OUTSIDE | SHARED => (
                    "COMBOBOX",
                    match id {
                        LAYOUT => "Layout",
                        ALIGN => "Island position",
                        ZONE => "Activation zone",
                        MONITOR => "Reveal settings for display",
                        OUTSIDE => "Outside edge delay",
                        SHARED => "Shared edge delay",
                        MEDIA_SIDE => "Media player side",
                        _ => "Search provider",
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
                MEDIA => (
                    "BUTTON",
                    "Show taskbar media player",
                    BS_AUTOCHECKBOX as u32,
                ),
                SIGN_IN => (
                    "BUTTON",
                    "Start SmartTaskbar when I sign in",
                    BS_AUTOCHECKBOX as u32,
                ),
                SEARCH => (
                    "BUTTON",
                    "Redirect Start web searches",
                    BS_AUTOCHECKBOX as u32,
                ),
                OVERRIDE => (
                    "BUTTON",
                    "Use custom reveal settings",
                    BS_AUTOCHECKBOX as u32,
                ),
                TABS => ("SysTabControl32", "", 0),
                CHECK_UPDATES => ("BUTTON", "Check again", BS_PUSHBUTTON as u32),
                RELEASES => ("BUTTON", "View releases", BS_PUSHBUTTON as u32),
                REPOSITORY => ("BUTTON", "Open GitHub repository", BS_PUSHBUTTON as u32),
                FULLSCREEN => ("BUTTON", "Go fullscreen", BS_PUSHBUTTON as u32),
                MENU => ("BUTTON", "Open menu", BS_PUSHBUTTON as u32),
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
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
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
        let choices: Vec<_> = s.choices.iter().map(String::as_str).collect();
        add(
            MEDIA_SIDE as i32,
            &["Left", "Right"],
            usize::from(s.draft.media_side == MediaSide::Right),
        );
        add(
            PROVIDER as i32,
            &choices,
            s.choices
                .iter()
                .position(|v| *v == s.draft.web_engine)
                .unwrap_or_default(),
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
            (MOTION, s.draft.animate),
            (SIGN_IN, s.draft.start_at_sign_in),
            (SEARCH, s.draft.search_redirect),
            (MEDIA, s.draft.media_player),
        ] {
            SendDlgItemMessageW(hwnd, id as i32, BM_SETCHECK, on as usize, 0);
        }
        (*state).paint.fonts((*state).dpi);
        (*state).paint.theme(hwnd);
        tab_item(hwnd, TCM_INSERTITEMW, 0, "Appearance");
        tab_item(hwnd, TCM_INSERTITEMW, 1, "Updates");
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
        for id in LAYOUT..=REPOSITORY {
            let c = GetDlgItem(hwnd, id as i32);
            if c.is_null() {
                continue;
            }
            SetWindowTheme(c, theme.as_ptr(), null());
            SendMessageW(c, WM_SETFONT, (*state).paint.normal as usize, 1);
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
        let page = Page::layout(w, h);
        let total = if (*state).updates_tab {
            344.max(h)
        } else {
            page.footer + 52
        };
        (*state).page = page;
        (*state).scroll_max = (total - h).max(0);
        (*state).scroll = (*state).scroll.clamp(0, (*state).scroll_max);
        let offset = (*state).scroll;
        let info = SCROLLINFO {
            cbSize: mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: total - 1,
            nPage: h.max(1) as u32,
            nPos: offset,
            nTrackPos: 0,
        };
        SetScrollInfo(hwnd, SB_VERT, &info, 1);
        for id in LAYOUT..=REPOSITORY {
            let b = page.control(id);
            let label = GetDlgItem(hwnd, (LABEL + id) as i32);
            let fixed = matches!(id, ENGINE | TABS | APPLY);
            let show = match id {
                ENGINE | TABS => true,
                CHECK_UPDATES | RELEASES | REPOSITORY => (*state).updates_tab,
                _ => !(*state).updates_tab && (fixed || b.y - offset >= 101),
            };
            let control = GetDlgItem(hwnd, id as i32);
            let visibility = if show { SW_SHOWNA } else { SW_HIDE };
            ShowWindow(control, visibility);
            if !label.is_null() {
                ShowWindow(
                    label,
                    if b.y - 19 - offset >= 101 {
                        visibility
                    } else {
                        SW_HIDE
                    },
                );
                SetWindowPos(
                    label,
                    null_mut(),
                    b.x * dpi / 96,
                    (b.y - 19 - offset) * dpi / 96,
                    (b.w - 50) * dpi / 96,
                    18 * dpi / 96,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            let height = if matches!(
                id,
                LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE | MONITOR | OUTSIDE | SHARED
            ) {
                180
            } else {
                b.h
            };
            SetWindowPos(
                control,
                null_mut(),
                b.x * dpi / 96,
                (b.y - if fixed { 0 } else { offset }) * dpi / 96,
                b.w * dpi / 96,
                height * dpi / 96,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        redraw(hwnd);
    }
}

unsafe fn scroll(hwnd: HWND, state: *mut State, delta: i32) {
    unsafe {
        let pos = ((*state).scroll + delta).clamp(0, (*state).scroll_max);
        if pos != (*state).scroll {
            (*state).scroll = pos;
            layout(hwnd, state);
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
                OUTSIDE,
                reveal.outside_ms,
                &[0, 80, 120, 180, 250, 400, 600, 800, 1000][..],
            ),
            (
                SHARED,
                reveal.shared_ms,
                &[150, 220, 300, 400, 600, 800, 1000][..],
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
                let label = if ms == 0 {
                    "Instant".into()
                } else {
                    format!("{ms} ms")
                };
                SendMessageW(combo, CB_ADDSTRING, 0, wide(&label).as_ptr() as isize);
                SendMessageW(combo, CB_SETITEMDATA, index, ms as isize);
                if ms == selected {
                    SendMessageW(combo, CB_SETCURSEL, index, 0);
                }
            }
            EnableWindow(combo, editable);
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
        // TBM_GETPOS is the trackbar's WM_USER + 0 message.
        let slider = |id| SendDlgItemMessageW(hwnd, id, WM_USER, 0, 0) as u8;
        let check = |id| SendDlgItemMessageW(hwnd, id, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        let s = &mut *state;
        s.draft.island = sel(LAYOUT as i32) == 1;
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
        if let Some(key) = &s.monitor {
            if let Some(setting) = s.draft.monitor_reveal.get_mut(key) {
                *setting = reveal;
            }
        } else {
            s.draft.zone = reveal.zone;
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
        s.draft.search_redirect = check(SEARCH as i32);
        s.draft.media_player = check(MEDIA as i32);
        s.draft.media_side = if sel(MEDIA_SIDE as i32) == 0 {
            MediaSide::Left
        } else {
            MediaSide::Right
        };
        if let Some(provider) = s.choices.get(sel(PROVIDER as i32) as usize) {
            s.draft.web_engine = provider.clone();
        }
        s.status.clear();
        reflect(hwnd, state);
        update_animation(hwnd, state);
    }
}
unsafe fn reflect(hwnd: HWND, state: *mut State) {
    unsafe {
        let s = &*state;
        EnableWindow(GetDlgItem(hwnd, ALIGN as i32), s.draft.island as i32);
        EnableWindow(
            GetDlgItem(hwnd, MEDIA_SIDE as i32),
            s.draft.media_player as i32,
        );
        EnableWindow(
            GetDlgItem(hwnd, PROVIDER as i32),
            s.draft.search_redirect as i32,
        );
        EnableWindow(GetDlgItem(hwnd, APPLY as i32), (s.draft != s.old) as i32);
        redraw(hwnd);
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
        let updates = SendDlgItemMessageW(hwnd, TABS as i32, TCM_GETCURSEL, 0, 0) == 1;
        if updates == s.updates_tab {
            return;
        }
        s.updates_tab = updates;
        if updates {
            s.settings_scroll = s.scroll;
            s.scroll = 0;
            s.demo.hot = None;
            s.demo.down = false;
            s.demo.over_edge = false;
            s.demo.last = None;
            KillTimer(hwnd, TIMER);
            KillTimer(hwnd, ICON_TIMER);
        } else {
            s.scroll = s.settings_scroll;
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
            1,
            if s.update_available {
                "Updates • New"
            } else {
                "Updates"
            },
        );
        redraw(hwnd);
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

unsafe fn toggle_preview(hwnd: HWND, state: *mut State, id: u16) {
    unsafe {
        let demo = &mut (*state).demo;
        if id == FULLSCREEN {
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
        SetWindowTextW(
            GetDlgItem(hwnd, MENU as i32),
            wide(if demo.menu { "Close menu" } else { "Open menu" }).as_ptr(),
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
                    toggle_preview(hwnd, state, MENU);
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
    fn controls_reflow_below_a_narrow_preview() {
        let wide = Page::layout(1000, 640);
        let narrow = Page::layout(600, 640);
        assert!(wide.settings.x >= wide.preview.x + wide.preview.w);
        assert!(narrow.settings.y >= narrow.preview.y + narrow.preview.h);
        assert!(narrow.control(APPLY).y < narrow.preview.y);
        let tabs = narrow.control(TABS);
        assert!(tabs.x + tabs.w < narrow.control(APPLY).x);
        assert!(narrow.control(SHARED).y < narrow.control(MEDIA_SIDE).y);
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
