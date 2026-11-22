use crate::policy::{Point, Rect};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, ODT_MENU},
        HiDpi::GetDpiForWindow,
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

#[derive(Default)]
struct State {
    surface: Option<(HWND, Rect, Rc<[Rect]>)>,
    down: Option<POINT>,
    pending: Option<POINT>,
}

thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

const MENU_ITEMS: [(u32, *const u16); 2] = [
    (1, windows_sys::w!("Task Manager")),
    (2, windows_sys::w!("Taskbar settings")),
];

#[derive(Clone, Copy)]
struct MenuPaint {
    bg: u32,
    fg: u32,
    selected: u32,
    dpi: u32,
    font: HFONT,
}

impl MenuPaint {
    fn scale(self, dip: i32) -> i32 {
        (dip * self.dpi as i32 / 96).max(1)
    }
}

thread_local! { static MENU_PAINT: Cell<Option<MenuPaint>> = const { Cell::new(None) }; }

pub(super) unsafe fn paint_message(msg: u32, data: LPARAM) -> bool {
    if data == 0 || !matches!(msg, WM_MEASUREITEM | WM_DRAWITEM) {
        return false;
    }
    let Some(paint) = MENU_PAINT.get() else {
        return false;
    };
    unsafe {
        if msg == WM_MEASUREITEM {
            let item = &mut *(data as *mut MEASUREITEMSTRUCT);
            if item.CtlType != ODT_MENU {
                return false;
            }
            item.itemWidth = paint.scale(180) as u32;
            item.itemHeight = paint.scale(34) as u32;
            return true;
        }
        let item = &*(data as *const DRAWITEMSTRUCT);
        if item.CtlType != ODT_MENU {
            return false;
        }
        let Some((_, label)) = MENU_ITEMS.iter().find(|(id, _)| *id == item.itemID) else {
            return false;
        };
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            return false;
        }
        let mut row = item.rcItem;
        SelectObject(item.hDC, GetStockObject(DC_BRUSH));
        SetDCBrushColor(item.hDC, paint.bg);
        FillRect(item.hDC, &row, GetStockObject(DC_BRUSH) as HBRUSH);
        if item.itemState & ODS_SELECTED != 0 {
            InflateRect(&mut row, -paint.scale(4), -paint.scale(2));
            SelectObject(item.hDC, GetStockObject(DC_PEN));
            SetDCPenColor(item.hDC, paint.selected);
            SetDCBrushColor(item.hDC, paint.selected);
            let radius = paint.scale(8);
            RoundRect(
                item.hDC, row.left, row.top, row.right, row.bottom, radius, radius,
            );
        } else if item.itemState & ODS_FOCUS != 0 {
            InflateRect(&mut row, -paint.scale(4), -paint.scale(2));
            DrawFocusRect(item.hDC, &row);
        }
        let mut text = item.rcItem;
        text.left += paint.scale(14);
        SelectObject(item.hDC, paint.font);
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, paint.fg);
        DrawTextW(
            item.hDC,
            *label,
            -1,
            &mut text,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        RestoreDC(item.hDC, saved);
    }
    true
}

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
            let _dpi = super::DpiScope::enter();
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
        let mut brush = null_mut();
        let mut font = null_mut();
        if let Some((bg, fg, selected)) = crate::appearance::taskbar_menu_colors() {
            let dpi = GetDpiForWindow(WindowFromPoint(point)).max(96);
            brush = CreateSolidBrush(bg);
            font = super::app_labels::font(dpi);
            if !brush.is_null() && !font.is_null() {
                let info = MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND | MIM_STYLE,
                    dwStyle: MNS_NOCHECK,
                    hbrBack: brush,
                    ..Default::default()
                };
                if SetMenuInfo(menu, &info) != 0 {
                    MENU_PAINT.set(Some(MenuPaint {
                        bg,
                        fg,
                        selected,
                        dpi,
                        font,
                    }));
                }
            }
        }
        for (position, &(id, label)) in MENU_ITEMS.iter().enumerate() {
            let item = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_ID | MIIM_STRING | MIIM_FTYPE,
                fType: if MENU_PAINT.get().is_some() {
                    MFT_OWNERDRAW
                } else {
                    MFT_STRING
                },
                wID: id,
                dwTypeData: label.cast_mut(),
                ..Default::default()
            };
            InsertMenuItemW(menu, position as u32, 1, &item);
        }
        SetForegroundWindow(owner);
        let action = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            owner,
            null(),
        );
        PostMessageW(owner, WM_NULL, 0, 0);
        DestroyMenu(menu);
        MENU_PAINT.set(None);
        if !brush.is_null() {
            DeleteObject(brush);
        }
        if !font.is_null() {
            DeleteObject(font);
        }
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
