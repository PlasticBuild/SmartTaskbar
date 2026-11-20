use crate::settings::{Alignment, MediaSide, PROVIDERS, Settings, Zone};
use std::{
    mem,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
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
const TIMER: usize = 1;
const ICON_TIMER: usize = 2;
const LABEL: u16 = 1000;
const LABELS: &[(u16, &str)] = &[
    (LAYOUT, "&Layout"),
    (ALIGN, "Island &position"),
    (RADIUS, "Corner &radius"),
    (TRANSPARENCY, "&Transparency"),
    (ICON_SIZE, "App icon si&ze"),
    (HOVER, "&Hover lift"),
    (PRESS, "Click &depth"),
    (ZONE, "Activation &zone"),
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
                Box2::at(inset, 65, left, (height - 145).clamp(360, 610)),
                Box2::at(inset + left + 20, 65, right, 632),
            )
        } else {
            let h = (usable * 53 / 100).clamp(215, 340);
            (
                Box2::at(inset, 65, usable, h),
                Box2::at(inset, 65 + h + 14, usable, 632),
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
            ZONE => (380, 25),
            MOTION => (413, 23),
            SIGN_IN => (439, 23),
            SEARCH => (465, 23),
            PROVIDER => (512, 25),
            MEDIA => (548, 23),
            MEDIA_SIDE => (594, 25),
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
                return Box2::at(
                    self.width - if self.width < 600 { 16 } else { 24 } - 112,
                    self.footer,
                    112,
                    32,
                );
            }
            ENGINE => return Box2::at(self.preview.x, self.footer, 112, 32),
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

pub fn show(settings_path: &Path) -> Result<(), String> {
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
        let class = wide("SmartTaskbar.Rust.Appearance");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
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
        });
        let mut area: RECT = mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut _ as _, 0);
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("SmartTaskbar Appearance").as_ptr(),
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
                if (LAYOUT..=ICON_SIZE).contains(&id) {
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
                if (*ptr).paint.refresh_icons() {
                    SetTimer(hwnd, ICON_TIMER, 90, None);
                }
                (*ptr).engine = crate::engine_running();
                SetWindowTextW(
                    GetDlgItem(hwnd, ENGINE as i32),
                    wide(if (*ptr).engine {
                        "Stop engine"
                    } else {
                        "Start engine"
                    })
                    .as_ptr(),
                );
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
                if matches!(id, LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE)
                    && note == CBN_SELCHANGE
                {
                    read_controls(hwnd, ptr);
                } else if note == BN_CLICKED {
                    match id {
                        MOTION | SIGN_IN | SEARCH | MEDIA => read_controls(hwnd, ptr),
                        MENU | FULLSCREEN => toggle_preview(hwnd, ptr, id),
                        APPLY => save(hwnd, ptr),
                        ENGINE => engine(hwnd, ptr),
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
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
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
            WM_MOUSELEAVE | WM_CAPTURECHANGED => {
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
                LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE => (
                    "COMBOBOX",
                    match id {
                        LAYOUT => "Layout",
                        ALIGN => "Island position",
                        ZONE => "Activation zone",
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
                FULLSCREEN => ("BUTTON", "Go fullscreen", BS_PUSHBUTTON as u32),
                MENU => ("BUTTON", "Open menu", BS_PUSHBUTTON as u32),
                ENGINE => (
                    "BUTTON",
                    if (*state).engine {
                        "Stop engine"
                    } else {
                        "Start engine"
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
            &["Entire edge", "Center half", "Island width"],
            match s.draft.zone {
                Zone::Entire => 0,
                Zone::Center => 1,
                Zone::Island => 2,
            },
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
        style_controls(hwnd, state);
        (*state).ready = true;
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
        for id in LAYOUT..=ICON_SIZE {
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
        let total = page.footer + 52;
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
        for id in LAYOUT..=ICON_SIZE {
            let b = page.control(id);
            let label = GetDlgItem(hwnd, (LABEL + id) as i32);
            if !label.is_null() {
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
            let height = if matches!(id, LAYOUT | ALIGN | ZONE | PROVIDER | MEDIA_SIDE) {
                180
            } else {
                b.h
            };
            SetWindowPos(
                GetDlgItem(hwnd, id as i32),
                null_mut(),
                b.x * dpi / 96,
                (b.y - offset) * dpi / 96,
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
        s.draft.zone = match sel(ZONE as i32) {
            1 => Zone::Center,
            2 => Zone::Island,
            _ => Zone::Entire,
        };
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
        let running = crate::engine_running();
        let result = if running {
            crate::desktop::request_stop()
        } else {
            crate::start_engine(&(*state).path)
        };
        match result {
            Ok(()) => {
                (*state).engine = !running;
                (*state).status = if running {
                    "Engine stop requested."
                } else {
                    "Engine start requested."
                }
                .into();
            }
            Err(error) => {
                (*state).engine = crate::engine_running();
                MessageBoxW(
                    hwnd,
                    wide(&error).as_ptr(),
                    wide("SmartTaskbar engine").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        SetWindowTextW(
            GetDlgItem(hwnd, ENGINE as i32),
            wide(if (*state).engine {
                "Stop engine"
            } else {
                "Start engine"
            })
            .as_ptr(),
        );
        redraw(hwnd);
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
        assert!(narrow.control(APPLY).y > narrow.control(MEDIA_SIDE).y);
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
