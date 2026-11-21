use super::{
    PANE_CLASS, apps,
    provider::{Provider, is_https},
};
use crate::policy::Rect;
use std::{
    path::Path,
    ptr::{null, null_mut},
    sync::mpsc,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Controller,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, take_pwstr,
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HWND as ComHwnd, RECT as ComRect},
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    },
    core::{BOOL as ComBool, HSTRING, PCWSTR, PWSTR},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::{
            DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            DwmSetWindowAttribute,
        },
        Gdi::*,
    },
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{
        Controls::{
            DRAWITEMSTRUCT, EM_SETCUEBANNER, EM_SETLIMITTEXT, EM_SETMARGINS, EM_SETSEL,
            MEASUREITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, SetWindowTheme,
        },
        Input::KeyboardAndMouse::{
            GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_RETURN,
        },
        WindowsAndMessaging::*,
    },
};

const QUERY: i32 = 101;
const APP_TAB: i32 = 102;
const STATUS: i32 = 103;
const WEB_TAB: i32 = 104;
const RESULTS: i32 = 105;
const BROWSER: i32 = 106;
const SETTINGS: i32 = 107;
const CLOSE: i32 = 108;
const SEARCH_TIMER: usize = 1;
const SEARCH_DELAY_MS: u32 = 450;

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct State {
    dpi: i32,
    dark: i32,
    font: HFONT,
    apps: Vec<apps::App>,
    matches: Vec<usize>,
    web_mode: bool,
    showing_web: bool,
    provider: Option<Result<Provider, &'static str>>,
    controller: Option<ICoreWebView2Controller>,
    last_query: String,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.font) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn attach_browser(
    hwnd: HWND,
    directory: &Path,
) -> Result<ICoreWebView2Controller, webview2_com::Error> {
    let (tx, rx) = mpsc::channel();
    let directory = HSTRING::from(directory.as_os_str());
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |callback| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                &directory,
                None::<&ICoreWebView2EnvironmentOptions>,
                &callback,
            )
            .map_err(Into::into)
        }),
        Box::new(move |status, environment| {
            status?;
            let _ = tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )?;
    let environment = rx.recv().map_err(|_| webview2_com::Error::SendError)??;
    let parent = ComHwnd(hwnd);
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |callback| unsafe {
            environment
                .CreateCoreWebView2Controller(parent, &callback)
                .map_err(Into::into)
        }),
        Box::new(move |status, controller| {
            status?;
            let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )?;
    let controller = rx.recv().map_err(|_| webview2_com::Error::SendError)??;
    unsafe {
        let view = controller.CoreWebView2()?;
        let options = view.Settings()?;
        options.SetAreDevToolsEnabled(false)?;
        options.SetIsWebMessageEnabled(false)?;
        options.SetAreHostObjectsAllowed(false)?;
        let mut token = 0;
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    let mut url = PWSTR::null();
                    args.Uri(&mut url)?;
                    let url = take_pwstr(url);
                    if url != "about:blank" && !is_https(&url) {
                        args.SetCancel(true)?;
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|sender, args| {
                if let (Some(view), Some(args)) = (sender, args) {
                    args.SetHandled(true)?;
                    let mut initiated = ComBool::default();
                    args.IsUserInitiated(&mut initiated)?;
                    if initiated.as_bool() {
                        let mut url = PWSTR::null();
                        args.Uri(&mut url)?;
                        let url = take_pwstr(url);
                        if is_https(&url) {
                            view.Navigate(&HSTRING::from(url))?;
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        controller.SetIsVisible(true)?;
    }
    Ok(controller)
}

pub(super) fn run(
    bounds: Rect,
    dpi: i32,
    config: &Path,
    web_mode: bool,
    query: impl FnOnce() -> Option<String>,
) -> Result<(), String> {
    if bounds.width() < 240 || bounds.height() < 200 {
        return Err("There is not enough room for SmartTaskbar Start on this display.".into());
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
        .map_err(|err| err.to_string())?;
    let _apartment = Apartment;
    let query = query().unwrap_or_default();
    let mut state = State {
        dpi,
        dark: dark_theme(),
        font: menu_font(dpi),
        apps: apps::installed(),
        matches: Vec::new(),
        web_mode,
        showing_web: false,
        provider: None,
        controller: None,
        last_query: String::new(),
    };
    unsafe {
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PANE_CLASS,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_TOPMOST,
            PANE_CLASS,
            windows_sys::w!("SmartTaskbar Start"),
            WS_POPUP | WS_BORDER | WS_CLIPCHILDREN,
            bounds.left,
            bounds.top,
            bounds.width(),
            bounds.height(),
            null_mut(),
            null_mut(),
            instance,
            (&mut state as *mut State).cast(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        for (attr, value) in [
            (DWMWA_USE_IMMERSIVE_DARK_MODE, state.dark),
            (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND),
        ] {
            DwmSetWindowAttribute(hwnd, attr as u32, (&value as *const i32).cast(), 4);
        }
        let input = GetDlgItem(hwnd, QUERY);
        SetWindowTextW(input, wide(&query).as_ptr());
        refresh(hwnd, &mut state);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(hwnd);
        SetFocus(input);
        let caret = query.encode_utf16().count();
        SendMessageW(input, EM_SETSEL, caret, caret as isize);
        let browser = config.with_file_name("web-search-data");
        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == -1 {
                DestroyWindow(hwnd);
                return Err(std::io::Error::last_os_error().to_string());
            }
            if result == 0 {
                break;
            }
            if msg.hwnd == hwnd && msg.message == WM_TIMER && msg.wParam == SEARCH_TIMER {
                KillTimer(hwnd, SEARCH_TIMER);
                if let Some(terms) = web_query(&text(input), state.web_mode) {
                    submit(hwnd, &mut state, &browser, &terms);
                }
                continue;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == usize::from(VK_ESCAPE) {
                DestroyWindow(hwnd);
                continue;
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam == usize::from(b'A')
                && GetFocus() == input
                && GetKeyState(i32::from(VK_CONTROL)) < 0
            {
                SendMessageW(input, EM_SETSEL, 0, -1);
                continue;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == usize::from(VK_RETURN) {
                let focus = GetFocus();
                if [input, GetDlgItem(hwnd, RESULTS)].contains(&focus) {
                    if !state.showing_web {
                        launch_selected(hwnd, &state);
                    }
                    continue;
                }
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam == usize::from(VK_DOWN)
                && GetFocus() == input
                && !state.showing_web
                && !state.matches.is_empty()
            {
                SetFocus(GetDlgItem(hwnd, RESULTS));
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

pub(super) fn dark_theme() -> i32 {
    let mut value = 1u32;
    let mut size = 4;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows_sys::w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            windows_sys::w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    i32::from(value == 0)
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_CREATE => {
                if create_controls(hwnd, &*ptr) {
                    0
                } else {
                    -1
                }
            }
            WM_ERASEBKGND if (*ptr).dark != 0 => {
                let dc = wp as HDC;
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                SetDCBrushColor(dc, 0x231e1c);
                FillRect(dc, &bounds, GetStockObject(DC_BRUSH));
                1
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX if (*ptr).dark != 0 => {
                let dc = wp as HDC;
                let color = if msg == WM_CTLCOLORSTATIC {
                    0x231e1c
                } else {
                    0x302a28
                };
                SetBkColor(dc, color);
                SetDCBrushColor(dc, color);
                SetTextColor(dc, 0xf7f3f1);
                GetStockObject(DC_BRUSH) as isize
            }
            WM_SIZE => {
                layout(hwnd, &*ptr);
                0
            }
            WM_DPICHANGED => {
                (*ptr).dpi = (wp & 0xffff) as i32;
                let font = menu_font((*ptr).dpi);
                for id in [
                    QUERY, APP_TAB, WEB_TAB, BROWSER, SETTINGS, CLOSE, STATUS, RESULTS,
                ] {
                    SendDlgItemMessageW(hwnd, id, WM_SETFONT, font as usize, 1);
                }
                SendDlgItemMessageW(
                    hwnd,
                    RESULTS,
                    LB_SETITEMHEIGHT,
                    0,
                    (34 * (*ptr).dpi / 96) as isize,
                );
                DeleteObject((*ptr).font);
                (*ptr).font = font;
                let window = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    window.left,
                    window.top,
                    window.right - window.left,
                    window.bottom - window.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            WM_MEASUREITEM if wp == RESULTS as usize => {
                (*(lp as *mut MEASUREITEMSTRUCT)).itemHeight = (34 * (*ptr).dpi / 96) as u32;
                1
            }
            WM_DRAWITEM => {
                draw_control(&*(lp as *const DRAWITEMSTRUCT), &*ptr);
                1
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as i32;
                let note = (wp >> 16) as u32;
                match (id, note) {
                    (QUERY, EN_CHANGE) => refresh(hwnd, &mut *ptr),
                    (APP_TAB | WEB_TAB, BN_CLICKED) => {
                        (*ptr).web_mode = id == WEB_TAB;
                        let input = GetDlgItem(hwnd, QUERY);
                        if id == APP_TAB
                            && let Some(plain) = super::web_prefix(&text(input))
                        {
                            SetWindowTextW(input, wide(plain).as_ptr());
                        }
                        refresh(hwnd, &mut *ptr);
                        SetFocus(input);
                    }
                    (RESULTS, LBN_DBLCLK) => launch_selected(hwnd, &*ptr),
                    (BROWSER, BN_CLICKED) => {
                        if let Some(terms) =
                            web_query(&text(GetDlgItem(hwnd, QUERY)), (*ptr).web_mode)
                        {
                            match crate::appearance::web_search::open(&terms) {
                                Ok(()) => {
                                    DestroyWindow(hwnd);
                                }
                                Err(err) => status(hwnd, err),
                            }
                        }
                    }
                    (SETTINGS, BN_CLICKED) => {
                        if let Ok(exe) = std::env::current_exe() {
                            let mut command = std::process::Command::new(exe);
                            command.arg("--settings");
                            if let Ok((_, Some(config))) = crate::parse(std::env::args_os().skip(1))
                            {
                                command.arg("--config").arg(config);
                            }
                            let _ = command.spawn();
                        }
                        DestroyWindow(hwnd);
                    }
                    (CLOSE, BN_CLICKED) => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, SEARCH_TIMER);
                if let Some(controller) = (*ptr).controller.take() {
                    let _ = controller.Close();
                }
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn create_controls(hwnd: HWND, state: &State) -> bool {
    unsafe {
        let instance = GetModuleHandleW(null());
        for (class, label, style, id) in [
            (
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                QUERY,
            ),
            ("BUTTON", "Apps", WS_TABSTOP | BS_OWNERDRAW as u32, APP_TAB),
            ("BUTTON", "Web", WS_TABSTOP | BS_OWNERDRAW as u32, WEB_TAB),
            (
                "BUTTON",
                "Open in browser",
                WS_TABSTOP | BS_OWNERDRAW as u32,
                BROWSER,
            ),
            (
                "BUTTON",
                "Settings",
                WS_TABSTOP | BS_OWNERDRAW as u32,
                SETTINGS,
            ),
            ("BUTTON", "Close", WS_TABSTOP | BS_OWNERDRAW as u32, CLOSE),
            ("STATIC", "", 0, STATUS),
            (
                "LISTBOX",
                "",
                WS_TABSTOP
                    | WS_VSCROLL
                    | (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS)
                        as u32,
                RESULTS,
            ),
        ] {
            let child = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                0,
                0,
                hwnd,
                std::ptr::without_provenance_mut(id as usize),
                instance,
                null_mut(),
            );
            if child.is_null() {
                return false;
            }
            if state.dark != 0 {
                SetWindowTheme(child, windows_sys::w!("DarkMode_Explorer"), null());
            }
            SendMessageW(child, WM_SETFONT, state.font as usize, 1);
        }
        SendDlgItemMessageW(hwnd, QUERY, EM_SETLIMITTEXT, 512, 0);
        SendDlgItemMessageW(
            hwnd,
            QUERY,
            EM_SETCUEBANNER,
            1,
            windows_sys::w!("Search apps or type Web:").cast::<u8>() as isize,
        );
        let inset = 8 * state.dpi / 96;
        SendDlgItemMessageW(
            hwnd,
            QUERY,
            EM_SETMARGINS,
            3,
            (inset | (inset << 16)) as isize,
        );
        layout(hwnd, state);
        true
    }
}

fn text(hwnd: HWND) -> String {
    let mut buf = [0; 513];
    let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) }.max(0);
    String::from_utf16_lossy(&buf[..len as usize])
}

fn menu_font(dpi: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -15 * dpi / 96,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            windows_sys::w!("Segoe UI"),
        )
    }
}

unsafe fn draw_control(item: &DRAWITEMSTRUCT, state: &State) {
    unsafe {
        let dark = state.dark != 0;
        let dc = item.hDC;
        let area = item.rcItem;
        let main = if dark { 0x231e1c } else { 0xf8f6f4 };
        let foreground = if dark { 0xf7f3f1 } else { 0x241e1a };
        let accent = if dark { 0xf1bc47 } else { 0xe2b148 };
        let pressed = item.itemState & ODS_SELECTED != 0;
        let focus = item.itemState & ODS_FOCUS != 0;
        let inset = 8 * state.dpi / 96;
        let old_font = SelectObject(dc, state.font);
        SetBkMode(dc, TRANSPARENT as i32);
        if item.CtlID == RESULTS as u32 {
            SetDCBrushColor(dc, main);
            FillRect(dc, &area, GetStockObject(DC_BRUSH));
            if let Some(&index) = state.matches.get(item.itemID as usize) {
                if pressed {
                    let row = RECT {
                        top: area.top + 1,
                        bottom: area.bottom - 1,
                        ..area
                    };
                    let select = if dark { 0x45362c } else { 0xf3e8d6 };
                    rounded(dc, row, select, if focus { accent } else { select }, inset);
                }
                draw_label(
                    dc,
                    &state.apps[index].name,
                    RECT {
                        left: area.left + 2 * inset,
                        right: area.right - inset,
                        ..area
                    },
                    foreground,
                    DT_LEFT,
                );
            }
        } else {
            SetDCBrushColor(dc, main);
            FillRect(dc, &area, GetStockObject(DC_BRUSH));
            let active = (item.CtlID == APP_TAB as u32 && !state.showing_web)
                || (item.CtlID == WEB_TAB as u32 && state.showing_web);
            let neutral = if dark { 0x3a332f } else { 0xece7e3 };
            let down = if dark { 0x54463e } else { 0xded5cd };
            let fill = if active {
                accent
            } else if pressed {
                down
            } else {
                neutral
            };
            let line = if active || focus {
                accent
            } else if dark {
                0x51463f
            } else {
                0xdcd5cd
            };
            rounded(dc, area, fill, line, area.bottom - area.top);
            let label =
                if item.CtlID == BROWSER as u32 && area.right - area.left < 120 * state.dpi / 96 {
                    "Browser".to_owned()
                } else {
                    text(item.hwndItem)
                };
            draw_label(
                dc,
                &label,
                area,
                if active { 0x241e1a } else { foreground },
                DT_CENTER,
            );
        }
        SelectObject(dc, old_font);
    }
}

unsafe fn rounded(dc: HDC, area: RECT, fill: u32, line: u32, corner: i32) {
    unsafe {
        let old_pen = SelectObject(dc, GetStockObject(DC_PEN));
        let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCPenColor(dc, line);
        SetDCBrushColor(dc, fill);
        RoundRect(
            dc,
            area.left,
            area.top,
            area.right,
            area.bottom,
            corner,
            corner,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
    }
}

unsafe fn draw_label(dc: HDC, label: &str, mut area: RECT, color: u32, alignment: u32) {
    unsafe {
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            wide(label).as_ptr(),
            -1,
            &mut area,
            alignment | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

fn web_query(value: &str, web_mode: bool) -> Option<String> {
    super::web_prefix(value)
        .or_else(|| web_mode.then(|| value.trim()))
        .filter(|terms| !terms.is_empty())
        .map(str::to_owned)
}

unsafe fn refresh(hwnd: HWND, state: &mut State) {
    unsafe {
        let value = text(GetDlgItem(hwnd, QUERY));
        let web = state.web_mode || super::web_prefix(&value).is_some();
        state.showing_web = web;
        for id in [APP_TAB, WEB_TAB] {
            InvalidateRect(GetDlgItem(hwnd, id), null(), 0);
        }
        ShowWindow(
            GetDlgItem(hwnd, BROWSER),
            if web { SW_SHOW } else { SW_HIDE },
        );
        let results = GetDlgItem(hwnd, RESULTS);
        ShowWindow(results, if web { SW_HIDE } else { SW_SHOW });
        KillTimer(hwnd, SEARCH_TIMER);
        if web {
            let terms = web_query(&value, state.web_mode);
            let current = terms
                .as_ref()
                .is_some_and(|terms| *terms == state.last_query);
            if let Some(controller) = &state.controller {
                let _ = controller.SetIsVisible(current);
            }
            if terms.is_some() && !current {
                status(hwnd, "Searching the web when you pause typing");
                SetTimer(hwnd, SEARCH_TIMER, SEARCH_DELAY_MS, None);
            } else {
                status(hwnd, if current { "" } else { "Search the web" });
            }
        } else {
            if let Some(controller) = &state.controller {
                let _ = controller.SetIsVisible(false);
            }
            state.matches = apps::filter(&state.apps, &value);
            SendMessageW(results, WM_SETREDRAW, 0, 0);
            SendMessageW(results, LB_RESETCONTENT, 0, 0);
            for &index in &state.matches {
                SendMessageW(
                    results,
                    LB_ADDSTRING,
                    0,
                    wide(&state.apps[index].name).as_ptr() as isize,
                );
            }
            SendMessageW(results, LB_SETCURSEL, 0, 0);
            SendMessageW(results, WM_SETREDRAW, 1, 0);
            InvalidateRect(results, null(), 1);
            status(
                hwnd,
                if state.matches.is_empty() {
                    "No matching apps"
                } else {
                    "Installed apps"
                },
            );
        }
    }
}

unsafe fn status(hwnd: HWND, message: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, STATUS), wide(message).as_ptr());
    }
}

unsafe fn launch_selected(hwnd: HWND, state: &State) {
    unsafe {
        let selection = SendDlgItemMessageW(hwnd, RESULTS, LB_GETCURSEL, 0, 0) as usize;
        if let Some(&index) = state.matches.get(selection) {
            if apps::launch(&state.apps[index], hwnd) {
                DestroyWindow(hwnd);
            } else {
                status(hwnd, "This app could not be opened.");
            }
        }
    }
}

unsafe fn submit(hwnd: HWND, state: *mut State, directory: &Path, terms: &str) {
    unsafe {
        let provider = (*state).provider.get_or_insert_with(Provider::current);
        let url = match provider
            .as_ref()
            .map_err(|message| *message)
            .and_then(|p| p.url(terms))
        {
            Ok(url) => url,
            Err(message) => {
                status(hwnd, message);
                return;
            }
        };
        if (*state).controller.is_none() {
            let Ok(controller) = attach_browser(hwnd, directory) else {
                status(hwnd, "The embedded browser could not start.");
                return;
            };
            if IsWindow(hwnd) == 0 {
                let _ = controller.Close();
                return;
            }
            (*state).controller = Some(controller);
            layout(hwnd, &*state);
            let query = text(GetDlgItem(hwnd, QUERY));
            if web_query(&query, (*state).web_mode).as_deref() != Some(terms) {
                if let Some(controller) = &(*state).controller {
                    let _ = controller.SetIsVisible(false);
                }
                refresh(hwnd, &mut *state);
                return;
            }
        }
        let Some(controller) = &(*state).controller else {
            return;
        };
        match controller
            .CoreWebView2()
            .and_then(|view| view.Navigate(&HSTRING::from(url)))
        {
            Ok(()) => {
                (*state).last_query = terms.to_owned();
                let _ = controller.SetIsVisible(true);
                status(hwnd, "");
            }
            Err(_) => status(hwnd, "The search engine page could not be opened."),
        }
    }
}

unsafe fn layout(hwnd: HWND, state: &State) {
    unsafe {
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let px = |dip: i32| dip * state.dpi / 96;
        let margin = px(18);
        let width = (area.right - 2 * margin).max(1);
        let button_y = px(66);
        let button_h = px(29);
        let browser_left = margin + px(152);
        let settings_left = area.right - margin - px(124);
        let browser_width = (settings_left - browser_left - px(8)).clamp(1, px(136));
        for (id, left, top, w, h) in [
            (QUERY, margin, px(18), width, px(36)),
            (APP_TAB, margin, button_y, px(62), button_h),
            (WEB_TAB, margin + px(72), button_y, px(62), button_h),
            (BROWSER, browser_left, button_y, browser_width, button_h),
            (SETTINGS, settings_left, button_y, px(60), button_h),
            (
                CLOSE,
                area.right - margin - px(55),
                button_y,
                px(55),
                button_h,
            ),
            (STATUS, margin, px(107), width, px(20)),
            (
                RESULTS,
                margin,
                px(129),
                width,
                (area.bottom - px(147)).max(1),
            ),
        ] {
            MoveWindow(GetDlgItem(hwnd, id), left, top, w, h, 1);
        }
        if let Some(controller) = &state.controller {
            let _ = controller.SetBounds(ComRect {
                left: margin,
                top: px(129),
                right: (area.right - margin).max(margin + 1),
                bottom: (area.bottom - margin).max(px(130)),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::ICoreWebView2NavigationCompletedEventArgs2,
        NavigationCompletedEventHandler,
    };
    use windows::core::Interface;

    #[test]
    #[ignore = "uses the installed WebView2 runtime and the current default search engine"]
    fn embedded_default_engine_returns_results() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let _apartment = Apartment;
            let query = "windows calculator keyboard shortcuts";
            let provider = Provider::current().unwrap();
            let url = provider.url(query).unwrap();
            let parent = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP,
                0,
                0,
                800,
                600,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!parent.is_null());
            let directory = std::path::PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").unwrap())
                .join(format!("embedded-search-test-{}", std::process::id()));
            let controller = attach_browser(parent, &directory).unwrap();
            let view = controller.CoreWebView2().unwrap();
            let (tx, rx) = mpsc::channel();
            let callback = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut code = 0;
                    let mut success = ComBool::default();
                    args.IsSuccess(&mut success)?;
                    args.cast::<ICoreWebView2NavigationCompletedEventArgs2>()?
                        .HttpStatusCode(&mut code)?;
                    if code != 0 || !success.as_bool() {
                        let _ = tx.send((success.as_bool(), code));
                    }
                }
                Ok(())
            }));
            let mut token = 0;
            view.add_NavigationCompleted(&callback, &mut token).unwrap();
            view.Navigate(&HSTRING::from(url)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut msg = MSG::default();
            let result = loop {
                if let Ok(value) = rx.try_recv() {
                    break Some(value);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(15));
            };
            let mut title = PWSTR::null();
            view.DocumentTitle(&mut title).unwrap();
            let title = take_pwstr(title);
            controller.Close().unwrap();
            DestroyWindow(parent);
            assert_eq!(result, Some((true, 200)), "embedded page title: {title}");
            assert!(
                title.to_lowercase().contains(query),
                "embedded page title: {title}"
            );
        }
    }
}
