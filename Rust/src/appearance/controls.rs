use super::*;

pub(super) unsafe extern "system" fn paint_control(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        let owner = GetParent(hwnd);
        let ptr = GetWindowLongPtrW(owner, GWLP_USERDATA) as *const State;
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(paint_control), id);
        }
        if ptr.is_null() || (*ptr).paint.palette.high_contrast {
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                if !dc.is_null() {
                    paint(hwnd, dc, id as u16, &*ptr);
                    EndPaint(hwnd, &ps);
                }
                return 0;
            }
            WM_PRINTCLIENT if wp != 0 => {
                paint(hwnd, wp as HDC, id as u16, &*ptr);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_MOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            _ => {}
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if matches!(
            msg,
            WM_MOUSEMOVE
                | WM_MOUSELEAVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_ENABLE
                | WM_SETTEXT
                | WM_CAPTURECHANGED
                | WM_KEYDOWN
                | WM_KEYUP
                | BM_SETCHECK
                | SBM_SETSCROLLINFO
                | SBM_SETPOS
                | TCM_SETCURSEL
                | TCM_SETITEMW
                | CB_SETCURSEL
                | CB_SHOWDROPDOWN
        ) {
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}

unsafe fn paint(hwnd: HWND, dc: HDC, id: u16, state: &State) {
    unsafe {
        let p = state.paint.palette;
        let px = |dip: i32| (dip * state.dpi / 96).max(1);
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r);
        if r.right <= 0 || r.bottom <= 0 {
            return;
        }
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        let enabled = IsWindowEnabled(hwnd) != 0;
        let color = if enabled { p.text } else { p.muted };
        let mut cursor = POINT::default();
        let hot = enabled
            && GetCursorPos(&mut cursor) != 0
            && ScreenToClient(hwnd, &mut cursor) != 0
            && PtInRect(&r, cursor) != 0;
        let focus = GetFocus() == hwnd;
        let background = if matches!(id, TABS | APPLY | ENGINE | SETTINGS_SCROLL) {
            p.page
        } else {
            p.card
        };
        fill(dc, r, background);
        SelectObject(dc, state.paint.normal as HGDIOBJ);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let surface = if hot {
            preview::mix(p.card, p.text, 0.11)
        } else {
            preview::mix(p.card, p.text, 0.05)
        };
        let mut caption = [0u16; 256];
        match id {
            SETTINGS_SCROLL => {
                let mut info = SCROLLBARINFO {
                    cbSize: mem::size_of::<SCROLLBARINFO>() as u32,
                    ..Default::default()
                };
                if state.scroll_max > 0 && GetScrollBarInfo(hwnd, OBJID_CLIENT, &mut info) != 0 {
                    let width = px(if hot { 7 } else { 4 });
                    let left = (r.right - width) / 2;
                    let thumb = RECT {
                        left,
                        right: left + width,
                        top: info.xyThumbTop,
                        bottom: info.xyThumbBottom,
                    };
                    round(dc, thumb, width, p.muted, p.muted);
                }
            }
            TABS => {
                let selected = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
                for index in 0..SendMessageW(hwnd, TCM_GETITEMCOUNT, 0, 0) {
                    let mut tab = RECT::default();
                    if SendMessageW(
                        hwnd,
                        TCM_GETITEMRECT,
                        index as usize,
                        &mut tab as *mut RECT as LPARAM,
                    ) == 0
                    {
                        continue;
                    }
                    tab.top = px(2);
                    tab.bottom = r.bottom - px(2);
                    let mut item = TCITEMW {
                        mask: TCIF_TEXT,
                        pszText: caption.as_mut_ptr(),
                        cchTextMax: caption.len() as i32,
                        ..Default::default()
                    };
                    if SendMessageW(
                        hwnd,
                        TCM_GETITEMW,
                        index as usize,
                        &mut item as *mut TCITEMW as LPARAM,
                    ) == 0
                    {
                        continue;
                    }
                    let active = index == selected;
                    if active || (hot && PtInRect(&tab, cursor) != 0) {
                        round(dc, tab, px(8), surface, surface);
                    }
                    SetTextColor(dc, if active { p.text } else { p.muted });
                    label(dc, tab, &caption, DT_CENTER);
                    if active {
                        let x = (tab.left + tab.right) / 2;
                        fill(
                            dc,
                            RECT {
                                left: x - px(8),
                                right: x + px(8),
                                top: tab.bottom - px(3),
                                bottom: tab.bottom - px(1),
                            },
                            p.accent,
                        );
                        if focus {
                            focus_ring(dc, tab, px(2));
                        }
                    }
                }
            }
            RADIUS | TRANSPARENCY | ICON_SIZE | HOVER | PRESS => {
                let mut thumb = RECT::default();
                let mut channel = RECT::default();
                SendMessageW(hwnd, TBM_GETTHUMBRECT, 0, &mut thumb as *mut RECT as LPARAM);
                SendMessageW(
                    hwnd,
                    TBM_GETCHANNELRECT,
                    0,
                    &mut channel as *mut RECT as LPARAM,
                );
                let y = (thumb.top + thumb.bottom) / 2;
                let x = (thumb.left + thumb.right) / 2;
                let rail = RECT {
                    top: y - px(2),
                    bottom: y + px(2),
                    ..channel
                };
                round(dc, rail, px(4), p.line, p.line);
                let accent = if enabled { p.accent } else { p.muted };
                round(dc, RECT { right: x, ..rail }, px(4), accent, accent);
                let radius = px(if hot { 8 } else { 7 });
                round(
                    dc,
                    RECT {
                        left: x - radius,
                        top: y - radius,
                        right: x + radius,
                        bottom: y + radius,
                    },
                    radius * 2,
                    accent,
                    accent,
                );
            }
            LAYOUT | ALIGN | ZONE | MEDIA_SIDE | MONITOR | OUTSIDE | SHARED | REDIRECT_PROVIDER
            | APP_LABEL_DELAY => {
                let border = if focus { p.accent } else { p.line };
                round(dc, r, px(8), surface, border);
                GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32);
                label(
                    dc,
                    RECT {
                        left: px(9),
                        right: r.right - px(26),
                        ..r
                    },
                    &caption,
                    DT_LEFT,
                );
                let (x, y) = (r.right - px(13), r.bottom / 2);
                SelectObject(dc, GetStockObject(DC_PEN));
                SetDCPenColor(dc, color);
                MoveToEx(dc, x - px(4), y - px(2), null_mut());
                LineTo(dc, x, y + px(2));
                LineTo(dc, x + px(5), y - px(3));
            }
            SHOW_ZONE | MATCH_MENU_WIDTH | MOTION | SIGN_IN | MEDIA | REDIRECT | OVERRIDE => {
                let checked = SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                let y = r.bottom / 2;
                let mark = RECT {
                    left: px(1),
                    right: px(17),
                    top: y - px(8),
                    bottom: y + px(8),
                };
                let accent = if enabled { p.accent } else { p.muted };
                let (bg, border) = if checked {
                    (accent, accent)
                } else {
                    (surface, p.muted)
                };
                round(dc, mark, px(5), bg, border);
                if checked {
                    SelectObject(dc, GetStockObject(DC_PEN));
                    SetDCPenColor(dc, if p.dark { p.page } else { 0x00ffffff });
                    MoveToEx(dc, px(4), y, null_mut());
                    LineTo(dc, px(8), y + px(4));
                    LineTo(dc, px(14), y - px(4));
                }
                GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32);
                label(dc, RECT { left: px(25), ..r }, &caption, DT_LEFT);
            }
            _ => {
                let primary = id == APPLY && enabled;
                let pressed = SendMessageW(hwnd, BM_GETSTATE, 0, 0) & BST_PUSHED as isize != 0;
                let bg = if primary {
                    p.accent
                } else if !enabled {
                    p.card
                } else {
                    surface
                };
                let bg = if pressed {
                    preview::mix(bg, p.page, 0.2)
                } else {
                    bg
                };
                round(dc, r, px(9), bg, if primary { bg } else { p.line });
                if primary {
                    SetTextColor(dc, if p.dark { p.page } else { 0x00ffffff });
                }
                GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32);
                label(
                    dc,
                    RECT {
                        left: px(5),
                        right: r.right - px(5),
                        ..r
                    },
                    &caption,
                    DT_CENTER,
                );
            }
        }
        if focus && !matches!(id, TABS | SETTINGS_SCROLL) {
            focus_ring(dc, r, px(2));
        }
        RestoreDC(dc, saved);
    }
}

unsafe fn focus_ring(dc: HDC, mut r: RECT, inset: i32) {
    unsafe {
        InflateRect(&mut r, -inset, -inset);
        DrawFocusRect(dc, &r);
    }
}

unsafe fn fill(dc: HDC, r: RECT, color: u32) {
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, &r, GetStockObject(DC_BRUSH) as HBRUSH);
    }
}

unsafe fn round(dc: HDC, r: RECT, size: i32, bg: u32, edge: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    unsafe {
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, bg);
        SetDCPenColor(dc, edge);
        RoundRect(dc, r.left, r.top, r.right, r.bottom, size, size);
    }
}

unsafe fn label(dc: HDC, mut r: RECT, caption: &[u16], align: u32) {
    unsafe {
        DrawTextW(
            dc,
            caption.as_ptr(),
            -1,
            &mut r,
            align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}
