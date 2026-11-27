use crate::{
    policy::{Edge, Point, Rect},
    settings::Settings,
};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute},
        Gdi::*,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, ODT_MENU},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW},
        WindowsAndMessaging::*,
    },
};

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    down: Option<POINT>,
    pending: Option<Invocation>,
}

struct Surface {
    bar: HWND,
    area: Rect,
    edge: Edge,
    dpi: u32,
    controls: Rc<[Rect]>,
}

#[derive(Clone, Copy)]
struct Invocation {
    point: POINT,
    area: Rect,
    edge: Edge,
    dpi: u32,
}

impl Invocation {
    fn placement(self) -> (POINT, u32) {
        let gap = dip(4, self.dpi);
        let mut point = self.point;
        let align = match self.edge {
            Edge::Bottom => {
                point.y = self.area.top - gap;
                TPM_BOTTOMALIGN
            }
            Edge::Top => {
                point.y = self.area.bottom + gap;
                TPM_TOPALIGN
            }
            Edge::Left => {
                point.x = self.area.right + gap;
                TPM_LEFTALIGN
            }
            Edge::Right => {
                point.x = self.area.left - gap;
                TPM_RIGHTALIGN
            }
        };
        let orientation = if self.edge.horizontal() {
            TPM_VERTICAL
        } else {
            TPM_HORIZONTAL
        };
        (point, align | orientation)
    }
}

thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }
thread_local! {
    static ACTIVE_FOREGROUND: Cell<Option<HWND>> = const { Cell::new(None) };
    static MENU_SHAPE: Cell<Option<(HWND, i32)>> = const { Cell::new(None) };
}

fn dip(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

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
    fn scale(self, value: i32) -> i32 {
        dip(value, self.dpi).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_opens_inward_with_a_gap_on_every_edge() {
        let area = Rect::new(-210, -80, -150, -20);
        let point = POINT { x: -180, y: -50 };
        for (edge, x, y, align) in [
            (Edge::Bottom, -180, -88, TPM_BOTTOMALIGN | TPM_VERTICAL),
            (Edge::Top, -180, -12, TPM_TOPALIGN | TPM_VERTICAL),
            (Edge::Left, -142, -50, TPM_LEFTALIGN | TPM_HORIZONTAL),
            (Edge::Right, -218, -50, TPM_RIGHTALIGN | TPM_HORIZONTAL),
        ] {
            let (anchor, flags) = Invocation {
                point,
                area,
                edge,
                dpi: 192,
            }
            .placement();
            assert_eq!((anchor.x, anchor.y, flags), (x, y, align));
            assert!(!area.contains(Point {
                x: anchor.x,
                y: anchor.y
            }));
        }
    }

    #[test]
    fn popup_region_rounds_only_the_top_corners() {
        unsafe {
            let region = popup_region(120, 70, 20);
            assert!(!region.is_null());
            let points = [(0, 0, 0), (119, 0, 0), (60, 0, 1), (0, 69, 1), (119, 69, 1)];
            for (x, y, inside) in points {
                assert_eq!(PtInRegion(region, x, y), inside);
            }
            assert_eq!(PtInRegion(region, 120, 69), 0);
            assert_eq!(PtInRegion(region, 0, 70), 0);
            DeleteObject(region);
        }
    }

    #[test]
    fn menu_clicks_use_the_tracked_window_bounds() {
        let _dpi = super::super::DpiScope::enter();
        let popup = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP | WS_VISIBLE,
                -16_000,
                -16_000,
                360,
                136,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!popup.is_null());
        let mut frame = RECT::default();
        assert_ne!(unsafe { GetWindowRect(popup, &mut frame) }, 0);
        let first_row = POINT {
            x: frame.left + 30,
            y: frame.top + 30,
        };
        let second_row = POINT {
            x: frame.left + 30,
            y: frame.top + 100,
        };
        let outside = POINT {
            x: frame.right,
            y: frame.top + 30,
        };
        let pending_click = clicked_outside_popup(outside);
        MENU_SHAPE.set(Some((popup, 0)));
        let clicks = [
            clicked_outside_popup(first_row),
            clicked_outside_popup(second_row),
            clicked_outside_popup(outside),
        ];
        MENU_SHAPE.set(None);
        unsafe { DestroyWindow(popup) };
        assert!(!pending_click);
        assert_eq!(clicks, [false, false, true]);
    }
}

thread_local! { static MENU_PAINT: Cell<Option<MenuPaint>> = const { Cell::new(None) }; }

unsafe fn paint_message(msg: u32, data: LPARAM) -> bool {
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
    pub fn track(&mut self, surface: Option<(&super::native::Bar, Rect, Rc<[Rect]>)>) {
        let needed = surface.is_some() || STATE.with(|state| state.borrow().down.is_some());
        STATE.with(|state| {
            state.borrow_mut().surface = surface.map(|(bar, area, controls)| Surface {
                bar: bar.hwnd,
                area,
                edge: bar.strip.edge,
                dpi: bar.dpi,
                controls,
            });
        });
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
        if code == HC_ACTION as i32 && ACTIVE_FOREGROUND.get().is_some() {
            if matches!(
                message as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
            ) {
                let pt = (*(data as *const MSLLHOOKSTRUCT)).pt;
                if clicked_outside_popup(pt) {
                    // End tracking before letting this same click reach the shell or app.
                    EndMenu();
                }
            }
            return CallNextHookEx(null_mut(), code, message, data);
        }
        if code == HC_ACTION as i32 && matches!(message as u32, WM_RBUTTONDOWN | WM_RBUTTONUP) {
            let _dpi = super::DpiScope::enter();
            let pt = (*(data as *const MSLLHOOKSTRUCT)).pt;
            let point = Point { x: pt.x, y: pt.y };
            let consume = STATE.with(|state| {
                let mut state = state.borrow_mut();
                let invocation = state
                    .surface
                    .as_ref()
                    .filter(|surface| {
                        surface.area.contains(point)
                            && !surface.controls.iter().any(|r| r.contains(point))
                            && GetAncestor(WindowFromPoint(pt), GA_ROOT) == surface.bar
                    })
                    .map(|surface| Invocation {
                        point: pt,
                        area: surface.area,
                        edge: surface.edge,
                        dpi: surface.dpi,
                    });
                if message as u32 == WM_RBUTTONDOWN && invocation.is_some() {
                    state.down = Some(pt);
                    return true;
                }
                if message as u32 == WM_RBUTTONUP
                    && let Some(down) = state.down.take()
                {
                    if invocation.is_some()
                        && pt.x.abs_diff(down.x) <= 6
                        && pt.y.abs_diff(down.y) <= 6
                    {
                        state.pending = invocation;
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

fn is_our_popup(hwnd: HWND) -> bool {
    if hwnd.is_null()
        || unsafe { GetWindowThreadProcessId(hwnd, null_mut()) }
            != super::OBSERVER_THREAD.load(std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    super::native::window_class(hwnd) == "#32768"
}

fn clicked_outside_popup(point: POINT) -> bool {
    let Some((popup, _)) = MENU_SHAPE.get() else {
        return false;
    };
    if popup.is_null() {
        // Until WM_ENTERIDLE identifies the menu, let Windows handle clicks.
        return false;
    }
    let _dpi = super::DpiScope::enter();
    let mut bounds = RECT::default();
    unsafe {
        // Native menus capture the mouse. A window lookup at the pointer can
        // report the app underneath even when the click belongs to a menu row.
        GetWindowRect(popup, &mut bounds) != 0
            && IsWindowVisible(popup) != 0
            && PtInRect(&bounds, point) == 0
    }
}

pub(super) fn foreground_changed(hwnd: HWND) {
    if ACTIVE_FOREGROUND
        .get()
        .is_some_and(|previous| previous != hwnd)
        && unsafe { GetForegroundWindow() } == hwnd
    {
        unsafe { EndMenu() };
    }
}

fn popup_region(width: i32, height: i32, diameter: i32) -> HRGN {
    let diameter = diameter.min(width).min(height);
    unsafe {
        if diameter <= 0 {
            return null_mut();
        }
        let rounded = CreateRoundRectRgn(0, 0, width, height, diameter, diameter);
        let bottom = CreateRectRgn(0, height - diameter / 2, width, height);
        let combined = !rounded.is_null()
            && !bottom.is_null()
            && CombineRgn(rounded, rounded, bottom, RGN_OR) != ERROR;
        if !bottom.is_null() {
            DeleteObject(bottom);
        }
        if combined {
            rounded
        } else {
            if !rounded.is_null() {
                DeleteObject(rounded);
            }
            null_mut()
        }
    }
}

unsafe extern "system" fn popup_frame(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    background: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCPAINT {
            let mut window = RECT::default();
            let mut client = RECT::default();
            if GetWindowRect(hwnd, &mut window) != 0 && GetClientRect(hwnd, &mut client) != 0 {
                let dc = GetWindowDC(hwnd);
                if !dc.is_null() {
                    MapWindowPoints(hwnd, null_mut(), (&raw mut client).cast(), 2);
                    OffsetRect(&mut client, -window.left, -window.top);
                    OffsetRect(&mut window, -window.left, -window.top);
                    ExcludeClipRect(dc, client.left, client.top, client.right, client.bottom);
                    SetDCBrushColor(dc, background as u32);
                    FillRect(dc, &window, GetStockObject(DC_BRUSH) as HBRUSH);
                    ReleaseDC(hwnd, dc);
                    return 0;
                }
            }
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(popup_frame), id);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn menu_owner(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if paint_message(msg, lp) {
            return 1;
        }
        if msg == WM_ENTERIDLE
            && wp == MSGF_MENU as WPARAM
            && let Some((shaped, diameter)) = MENU_SHAPE.get()
        {
            let popup = lp as HWND;
            let mut bounds = RECT::default();
            if shaped != popup && is_our_popup(popup) && GetWindowRect(popup, &mut bounds) != 0 {
                // Windows paints the native menu frame separately from owner-drawn items.
                if let Some(paint) = MENU_PAINT.get() {
                    SetWindowSubclass(popup, Some(popup_frame), 1, paint.bg as usize);
                    DwmSetWindowAttribute(
                        popup,
                        DWMWA_BORDER_COLOR as u32,
                        (&DWMWA_COLOR_NONE as *const u32).cast(),
                        size_of::<u32>() as u32,
                    );
                }
                let region = popup_region(
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    diameter,
                );
                if !region.is_null() && SetWindowRgn(popup, region, 1) == 0 {
                    DeleteObject(region);
                }
                MENU_SHAPE.set(Some((popup, diameter)));
                RedrawWindow(
                    popup,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW,
                );
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn create_owner(point: POINT) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = windows_sys::w!("SmartTaskbar.ContextMenuOwner");
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(menu_owner),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&descriptor) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return null_mut();
        }
        // A visible, activatable owner is required for native outside-click handling.
        let owner = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class,
            null(),
            WS_POPUP,
            point.x,
            point.y,
            1,
            1,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if !owner.is_null() {
            if SetLayeredWindowAttributes(owner, 0, 0, LWA_ALPHA) == 0 {
                DestroyWindow(owner);
                return null_mut();
            }
            ShowWindow(owner, SW_SHOWNOACTIVATE);
        }
        owner
    }
}

pub(super) fn show_pending(settings: &Settings, config: &Path) {
    let Some(invocation) = STATE.with(|state| state.borrow_mut().pending.take()) else {
        return;
    };
    let owner = create_owner(invocation.point);
    if owner.is_null() {
        return;
    }
    let action = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            DestroyWindow(owner);
            return;
        }
        let mut brush = null_mut();
        let mut font = null_mut();
        if let Some((bg, fg, selected)) = crate::appearance::taskbar_menu_colors() {
            let dpi = invocation.dpi;
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
        ACTIVE_FOREGROUND.set(Some(GetForegroundWindow()));
        let radius = dip(i32::from(settings.radius), invocation.dpi);
        MENU_SHAPE.set(Some((null_mut(), radius * 2)));
        let (point, align) = invocation.placement();
        let area = invocation.area;
        let params = TPMPARAMS {
            cbSize: size_of::<TPMPARAMS>() as u32,
            rcExclude: RECT {
                left: area.left,
                top: area.top,
                right: area.right,
                bottom: area.bottom,
            },
        };
        let action = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON
                | TPM_RETURNCMD
                | align
                | if settings.animate { 0 } else { TPM_NOANIMATION },
            point.x,
            point.y,
            owner,
            &params,
        );
        ACTIVE_FOREGROUND.set(None);
        MENU_SHAPE.set(None);
        PostThreadMessageW(
            super::OBSERVER_THREAD.load(std::sync::atomic::Ordering::Relaxed),
            WM_NULL,
            0,
            0,
        );
        DestroyWindow(owner);
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
                null_mut(),
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
