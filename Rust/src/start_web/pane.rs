use super::{
    PANE_CLASS,
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
        Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Gdi::*,
    },
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{
        Controls::{EM_SETLIMITTEXT, SetWindowTheme},
        Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_RETURN},
        WindowsAndMessaging::*,
    },
};

const QUERY: i32 = 101;
const SUBMIT: i32 = 102;
const STATUS: i32 = 103;

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct State {
    dpi: i32,
    dark: i32,
    provider: Result<Provider, &'static str>,
    controller: Option<ICoreWebView2Controller>,
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
    query: impl FnOnce() -> Option<String>,
) -> Result<(), String> {
    if bounds.width() < 190 || bounds.height() < 130 {
        return Err("There is not enough room for web results on this display.".into());
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
        .map_err(|err| err.to_string())?;
    let _apartment = Apartment;
    let query = query().unwrap_or_default();
    let mut state = State {
        dpi,
        dark: dark_theme(),
        provider: Provider::current(),
        controller: None,
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
            WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
            PANE_CLASS,
            windows_sys::w!("Web Search · SmartTaskbar"),
            WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_CLIPCHILDREN,
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
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&state.dark as *const i32).cast(),
            4,
        );
        let input = GetDlgItem(hwnd, QUERY);
        SetWindowTextW(input, wide(&query).as_ptr());
        let browser = config.with_file_name("web-search-data");
        if state.provider.is_ok() {
            match attach_browser(hwnd, &browser) {
                Ok(controller) => state.controller = Some(controller),
                Err(_) => status(hwnd, "The embedded WebView2 browser could not start."),
            }
        } else if let Err(message) = state.provider {
            status(hwnd, message);
        }
        layout(hwnd, &state);
        if !query.is_empty() && state.controller.is_some() {
            submit(hwnd, &state);
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetForegroundWindow(hwnd);
        SetFocus(input);
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
            if msg.message == WM_KEYDOWN && msg.wParam == usize::from(VK_ESCAPE) {
                DestroyWindow(hwnd);
                continue;
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam == usize::from(VK_RETURN)
                && GetFocus() == input
            {
                if msg.lParam & (1 << 30) == 0 {
                    submit(hwnd, &state);
                }
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

fn dark_theme() -> i32 {
    let mut value = 1u32;
    let mut size = 4;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows_sys::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
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
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_CREATE => {
                let instance = GetModuleHandleW(null());
                for (class, text, style, id) in [
                    (
                        "EDIT",
                        "",
                        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                        QUERY,
                    ),
                    (
                        "BUTTON",
                        "Search",
                        WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                        SUBMIT,
                    ),
                    ("STATIC", "", 0, STATUS),
                ] {
                    let child = CreateWindowExW(
                        0,
                        wide(class).as_ptr(),
                        wide(text).as_ptr(),
                        WS_CHILD | WS_VISIBLE | style,
                        0,
                        0,
                        0,
                        0,
                        hwnd,
                        std::ptr::without_provenance_mut(id as usize),
                        instance,
                        null(),
                    );
                    if child.is_null() {
                        return -1;
                    }
                    if (*state).dark != 0 {
                        SetWindowTheme(child, windows_sys::w!("DarkMode_Explorer"), null());
                    }
                    SendMessageW(
                        child,
                        WM_SETFONT,
                        GetStockObject(DEFAULT_GUI_FONT) as usize,
                        1,
                    );
                }
                SendDlgItemMessageW(hwnd, QUERY, EM_SETLIMITTEXT, 512, 0);
                layout(hwnd, &*state);
                0
            }
            WM_ERASEBKGND if (*state).dark != 0 => {
                let dc = wp as HDC;
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                SetDCBrushColor(dc, 0x1b1816);
                FillRect(dc, &bounds, GetStockObject(DC_BRUSH));
                1
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT if (*state).dark != 0 => {
                let dc = wp as HDC;
                let background = if msg == WM_CTLCOLOREDIT {
                    0x2c2724
                } else {
                    0x1b1816
                };
                SetBkColor(dc, background);
                SetDCBrushColor(dc, background);
                SetTextColor(dc, 0xf7f3f1);
                GetStockObject(DC_BRUSH) as isize
            }
            WM_SIZE => {
                layout(hwnd, &*state);
                0
            }
            WM_DPICHANGED => {
                (*state).dpi = (wp & 0xffff) as i32;
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
            WM_COMMAND if wp & 0xffff == SUBMIT as usize => {
                submit(hwnd, &*state);
                0
            }
            WM_DESTROY => {
                if let Some(controller) = (*state).controller.take() {
                    let _ = controller.Close();
                }
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn status(hwnd: HWND, message: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, STATUS), wide(message).as_ptr());
    }
}

unsafe fn submit(hwnd: HWND, state: &State) {
    unsafe {
        let Some(controller) = &state.controller else {
            return;
        };
        let Ok(provider) = &state.provider else {
            return;
        };
        let mut buf = [0; 513];
        let len = GetWindowTextW(GetDlgItem(hwnd, QUERY), buf.as_mut_ptr(), buf.len() as i32);
        let url = match provider.url(&String::from_utf16_lossy(&buf[..len as usize])) {
            Ok(url) => url,
            Err(message) => {
                status(hwnd, message);
                return;
            }
        };
        if controller
            .CoreWebView2()
            .and_then(|view| view.Navigate(&HSTRING::from(url)))
            .is_err()
        {
            status(hwnd, "The search engine page could not be opened.");
        } else {
            status(hwnd, "");
        }
    }
}

unsafe fn layout(hwnd: HWND, state: &State) {
    unsafe {
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let px = |dip| dip * state.dpi / 96;
        let margin = px(12);
        let height = px(28);
        let button = px(76);
        let input = (area.right - 3 * margin - button).max(1);
        MoveWindow(GetDlgItem(hwnd, QUERY), margin, margin, input, height, 1);
        MoveWindow(
            GetDlgItem(hwnd, SUBMIT),
            2 * margin + input,
            margin,
            button,
            height,
            1,
        );
        let status_top = 2 * margin + height;
        MoveWindow(
            GetDlgItem(hwnd, STATUS),
            margin,
            status_top,
            (area.right - 2 * margin).max(1),
            px(20),
            1,
        );
        if let Some(controller) = &state.controller {
            let top = status_top + px(21);
            let _ = controller.SetBounds(ComRect {
                left: 0,
                top,
                right: area.right.max(1),
                bottom: area.bottom.max(top + 1),
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
