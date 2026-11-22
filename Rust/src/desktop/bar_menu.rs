use crate::policy::{Point, Rect};
use std::{
    cell::RefCell,
    path::Path,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::*},
};

#[derive(Default)]
struct State {
    surface: Option<(HWND, Rect, Rc<[Rect]>)>,
    down: Option<POINT>,
    pending: Option<POINT>,
}

thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

#[derive(Default)]
pub(super) struct Hook(HHOOK);

impl Hook {
    pub fn track(&mut self, surface: Option<(HWND, Rect, Rc<[Rect]>)>) {
        let needed = surface.is_some() || STATE.with(|state| state.borrow().down.is_some());
        STATE.with(|state| state.borrow_mut().surface = surface);
        unsafe {
            if needed && self.0.is_null() {
                self.0 = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), GetModuleHandleW(null()), 0);
            } else if !needed && !self.0.is_null() {
                UnhookWindowsHookEx(self.0);
                self.0 = null_mut();
            }
        }
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { UnhookWindowsHookEx(self.0) };
        }
        STATE.with(|state| *state.borrow_mut() = State::default());
    }
}

unsafe extern "system" fn mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && matches!(message as u32, WM_RBUTTONDOWN | WM_RBUTTONUP) {
            let pt = (*(data as *const MSLLHOOKSTRUCT)).pt;
            let point = Point { x: pt.x, y: pt.y };
            let consume = STATE.with(|state| {
                let mut state = state.borrow_mut();
                let empty = state.surface.as_ref().is_some_and(|(bar, area, controls)| {
                    area.contains(point)
                        && !controls.iter().any(|r| r.contains(point))
                        && GetAncestor(WindowFromPoint(pt), GA_ROOT) == *bar
                });
                if message as u32 == WM_RBUTTONDOWN && empty {
                    state.down = Some(pt);
                    return true;
                }
                if message as u32 == WM_RBUTTONUP
                    && let Some(down) = state.down.take()
                {
                    if empty && pt.x.abs_diff(down.x) <= 6 && pt.y.abs_diff(down.y) <= 6 {
                        state.pending = Some(pt);
                        PostThreadMessageW(
                            super::OBSERVER_THREAD.load(std::sync::atomic::Ordering::Relaxed),
                            WM_NULL,
                            0,
                            0,
                        );
                    }
                    return true;
                }
                false
            });
            if consume {
                return 1;
            }
        }
        CallNextHookEx(null_mut(), code, message, data)
    }
}

pub(super) fn show_pending(owner: HWND, config: &Path) {
    if owner.is_null() {
        return;
    }
    let Some(point) = STATE.with(|state| state.borrow_mut().pending.take()) else {
        return;
    };
    let action = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(menu, MF_STRING, 1, windows_sys::w!("Task Manager"));
        AppendMenuW(menu, MF_STRING, 2, windows_sys::w!("Taskbar settings"));
        SetForegroundWindow(owner);
        let action = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            owner,
            null(),
        );
        PostMessageW(owner, WM_NULL, 0, 0);
        DestroyMenu(menu);
        action
    };
    match action {
        1 => unsafe {
            ShellExecuteW(
                owner,
                windows_sys::w!("open"),
                windows_sys::w!("Taskmgr.exe"),
                null(),
                null(),
                SW_SHOWNORMAL,
            );
        },
        2 => {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe)
                    .args(["--settings", "--config"])
                    .arg(config)
                    .spawn();
            }
        }
        _ => {}
    }
}
