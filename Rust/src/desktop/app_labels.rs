use super::bar_menu::Hook;
use super::native::Bar;
use crate::{
    policy::{Edge, Point, Rect},
    settings::Settings,
};
use std::{
    ptr::{null, null_mut},
    rc::Rc,
    sync::mpsc::{self, Receiver, TryRecvError},
};
use windows::Win32::{
    Foundation::HWND as AutomationWindow,
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
    UI::Accessibility::*,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const CLASS: *const u16 = windows_sys::w!("SmartTaskbar.AppName");
const FADE_MS: u64 = 120;

struct Label {
    name: String,
    anchor: Rect,
    hit: Rect,
}

struct Surface {
    labels: Vec<Label>,
    occupied: Rc<[Rect]>,
}

#[derive(Default)]
pub(super) struct Hover {
    input: Option<(usize, Rect)>,
    pending: Option<Receiver<Option<SurfaceData>>>,
    surface: Option<Surface>,
    entered: Option<(usize, u64)>,
    started: u64,
    retry_at: u64,
    window: Popup,
    menu_hook: Hook,
}

impl Hover {
    pub fn update(
        &mut self,
        target: Option<(Bar, Point, Rect)>,
        settings: &Settings,
        now: u64,
    ) -> Option<u32> {
        let Some((bar, point, area)) = target.filter(|(bar, point, _)| unsafe {
            GetAncestor(
                WindowFromPoint(POINT {
                    x: point.x,
                    y: point.y,
                }),
                GA_ROOT,
            ) == bar.hwnd
        }) else {
            self.window.hide();
            self.surface = None;
            self.entered = None;
            self.input = None;
            self.retry_at = 0;
            self.menu_hook.track(None);
            return None;
        };
        let hwnd = bar.hwnd as usize;
        let input = (hwnd, area);
        if self.input != Some(input) {
            self.surface = None;
            self.entered = None;
            self.window.hide();
            self.menu_hook.track(None);
        }
        if let Some(result) = self.pending.as_ref().map(Receiver::try_recv)
            && !matches!(result, Err(TryRecvError::Empty))
        {
            self.pending = None;
            if self.input == Some(input) {
                if let Ok(Some(data)) = result {
                    self.surface = Some(Surface {
                        labels: data.labels,
                        occupied: data.occupied.into(),
                    });
                } else {
                    self.input = None;
                    self.retry_at = now.saturating_add(500);
                }
            }
        }
        if self.pending.is_none() && self.input != Some(input) && now >= self.retry_at {
            self.input = Some(input);
            self.started = now;
            let (tx, rx) = mpsc::sync_channel(1);
            let edge = bar.strip.edge;
            let dpi = bar.dpi;
            if std::thread::Builder::new()
                .name("taskbar-accessibility".into())
                .spawn(move || {
                    let _ = tx.send(probe(hwnd, edge, dpi));
                })
                .is_ok()
            {
                self.pending = Some(rx);
            }
        }
        let Some(surface) = &self.surface else {
            return self.pending.as_ref().map(|_| 32);
        };
        self.menu_hook
            .track(Some((bar.hwnd, area, surface.occupied.clone())));
        if !settings.show_app_labels {
            self.entered = None;
            self.window.hide();
            return Some(45);
        }
        let mouse_down = unsafe {
            GetAsyncKeyState(VK_LBUTTON.into()) < 0 || GetAsyncKeyState(VK_RBUTTON.into()) < 0
        };
        let Some((index, label)) = surface
            .labels
            .iter()
            .enumerate()
            .find(|(_, label)| label.hit.contains(point) && !mouse_down)
        else {
            self.entered = None;
            self.started = now;
            self.window.hide();
            return Some(45);
        };
        if self.entered.is_none_or(|(previous, _)| previous != index) {
            let start = if self.entered.is_some() {
                now
            } else {
                self.started
            };
            self.entered = Some((index, start));
            self.window.hide();
        }
        let remaining = self
            .entered
            .unwrap()
            .1
            .saturating_add(u64::from(settings.app_label_delay_ms))
            .saturating_sub(now);
        if remaining != 0 {
            return Some((remaining as u32).min(45));
        }
        self.window
            .update(bar, label, settings.animate, now)
            .or(Some(45))
    }
}

struct SurfaceData {
    labels: Vec<Label>,
    occupied: Vec<Rect>,
}

fn probe(hwnd: usize, edge: Edge, dpi: u32) -> Option<SurfaceData> {
    // A new accessibility worker otherwise receives virtualized coordinates.
    let _dpi = super::DpiScope::enter();
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return None;
        }
        let data = (|| {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let root = automation
                .ElementFromHandle(AutomationWindow(hwnd as *mut _))
                .ok()?;
            let elements = root
                .FindAll(
                    TreeScope_Descendants,
                    &automation.ControlViewCondition().ok()?,
                )
                .ok()?;
            let mut data = SurfaceData {
                labels: Vec::new(),
                occupied: Vec::new(),
            };
            let count = elements.Length().ok()?;
            if count > 512 {
                return None;
            }
            for i in 0..count {
                let element = elements.GetElement(i).ok()?;
                let role = element.CurrentControlType().ok()?;
                if ![
                    UIA_ButtonControlTypeId,
                    UIA_SplitButtonControlTypeId,
                    UIA_EditControlTypeId,
                    UIA_MenuItemControlTypeId,
                    UIA_ListItemControlTypeId,
                    UIA_TabItemControlTypeId,
                    UIA_HyperlinkControlTypeId,
                    UIA_CheckBoxControlTypeId,
                ]
                .contains(&role)
                    && !element.CurrentIsKeyboardFocusable().ok()?.as_bool()
                {
                    continue;
                }
                let r = element.CurrentBoundingRectangle().ok()?;
                let anchor = Rect::new(r.left, r.top, r.right, r.bottom);
                if anchor.is_empty() {
                    continue;
                }
                let is_app =
                    element.CurrentClassName().ok()? == "Taskbar.TaskListButtonAutomationPeer";
                let is_start = !is_app && element.CurrentAutomationId().ok()? == "StartButton";
                if !is_app && !is_start {
                    data.occupied.push(anchor);
                    continue;
                }
                let hit = if is_app {
                    icon_hit(anchor, edge, dpi)
                } else {
                    anchor
                };
                data.occupied.push(hit);
                let raw = element.CurrentName().ok()?.to_string();
                let name = raw.strip_suffix(" pinned").unwrap_or(&raw);
                let name = name
                    .rsplit_once(" - ")
                    .filter(|(_, status)| {
                        status.ends_with(" running window") || status.ends_with(" running windows")
                    })
                    .map_or(name, |(name, _)| name);
                if !name.trim().is_empty() {
                    data.labels.push(Label {
                        name: name.to_owned(),
                        anchor,
                        hit,
                    });
                }
            }
            data.occupied
                .retain(|r| !spans_multiple_icons(*r, &data.labels));
            (!data.occupied.is_empty()).then_some(data)
        })();
        CoUninitialize();
        data
    }
}

fn spans_multiple_icons(area: Rect, labels: &[Label]) -> bool {
    labels
        .iter()
        .filter(|label| area.intersection(label.hit) == Some(label.hit))
        .nth(1)
        .is_some()
}

fn icon_hit(mut bounds: Rect, edge: Edge, dpi: u32) -> Rect {
    let pixels = |dip| dip * dpi as i32 / 96;
    // Preserve labeled app buttons as well as the full hit areas of tray controls.
    let span = if edge.horizontal() {
        bounds.width()
    } else {
        bounds.height()
    };
    if span > pixels(58) || span < pixels(36) {
        return bounds;
    }
    let gap = pixels(5);
    if edge.horizontal() {
        bounds.left += gap;
        bounds.right -= gap;
    } else {
        bounds.top += gap;
        bounds.bottom -= gap;
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a live Explorer taskbar with pinned apps"]
    fn reads_app_labels_from_the_live_taskbar() {
        let _dpi = super::super::DpiScope::enter();
        let bars = super::super::native::bars();
        assert!(!bars.is_empty());
        for bar in bars {
            let surface = probe(bar.hwnd as usize, bar.strip.edge, bar.dpi)
                .expect("taskbar accessibility surface");
            let hwnd = bar.hwnd as usize;
            let edge = bar.strip.edge;
            let dpi = bar.dpi;
            let worker = std::thread::spawn(move || probe(hwnd, edge, dpi))
                .join()
                .unwrap()
                .unwrap();
            assert_eq!(
                surface
                    .labels
                    .iter()
                    .map(|label| label.anchor)
                    .collect::<Vec<_>>(),
                worker
                    .labels
                    .iter()
                    .map(|label| label.anchor)
                    .collect::<Vec<_>>(),
                "background queries must use the same physical coordinates as the controller"
            );
            eprintln!(
                "{} labels, {} occupied controls",
                surface.labels.len(),
                surface.occupied.len()
            );
            assert!(!surface.labels.is_empty(), "the taskbar has no app labels");
        }
    }

    #[test]
    fn discards_icon_row_container_from_the_menu_hit_map() {
        let label = |left| {
            let anchor = Rect::new(left, 0, left + 44, 48);
            Label {
                name: String::new(),
                anchor,
                hit: icon_hit(anchor, Edge::Bottom, 96),
            }
        };
        let labels = [label(0), label(44)];
        assert!(spans_multiple_icons(Rect::new(0, 0, 88, 48), &labels));
        assert!(!spans_multiple_icons(labels[0].anchor, &labels));
        assert!(!spans_multiple_icons(Rect::new(88, 0, 152, 48), &labels));
    }

    #[test]
    fn preserves_icon_centers_and_creates_gaps_in_both_orientations() {
        let horizontal = Rect::new(0, -96, 88, 0);
        assert_eq!(
            icon_hit(horizontal, Edge::Bottom, 192),
            Rect::new(10, -96, 78, 0)
        );
        let vertical = Rect::new(-96, 0, 0, 88);
        assert_eq!(
            icon_hit(vertical, Edge::Left, 192),
            Rect::new(-96, 10, 0, 78)
        );
        let labeled = Rect::new(0, -96, 190, 0);
        assert_eq!(icon_hit(labeled, Edge::Top, 192), labeled);
    }
}

#[derive(Default)]
struct Popup {
    hwnd: HWND,
    shown_at: u64,
    alpha: u8,
}

impl Popup {
    fn hide(&mut self) {
        if !self.hwnd.is_null() {
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = null_mut();
        }
    }

    fn update(&mut self, bar: Bar, label: &Label, animate: bool, now: u64) -> Option<u32> {
        unsafe {
            if self.hwnd.is_null() {
                self.hwnd = create(bar, label)?;
                self.shown_at = now;
                self.alpha = 0;
            }
            let duration = now.saturating_sub(self.shown_at).min(FADE_MS);
            let alpha = if animate {
                (1 + duration * 254 / FADE_MS) as u8
            } else {
                255
            };
            if alpha != self.alpha {
                SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);
                if self.alpha == 0 {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.alpha = alpha;
            }
            (alpha != 255).then_some(16)
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.hide();
    }
}

unsafe fn create(bar: Bar, label: &Label) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            hInstance: instance,
            lpszClassName: CLASS,
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return None;
        }
        let caption = super::native::wide(&label.name);
        let px = |dip| dip * bar.dpi as i32 / 96;
        let dc = GetDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let font = font(bar.dpi);
        let previous = SelectObject(dc, font);
        let mut size = SIZE::default();
        GetTextExtentPoint32W(
            dc,
            caption.as_ptr(),
            caption.len().saturating_sub(1) as i32,
            &mut size,
        );
        SelectObject(dc, previous);
        DeleteObject(font);
        ReleaseDC(null_mut(), dc);
        let display = bar.strip.display;
        let width = (size.cx + px(20)).min(px(340)).min(display.width());
        let height = px(30).min(display.height());
        let gap = px(6);
        let anchor = label.anchor;
        let x = match bar.strip.edge {
            Edge::Left => bar.strip.bounds.right + gap,
            Edge::Right => bar.strip.bounds.left - width - gap,
            _ => anchor.left + (anchor.width() - width) / 2,
        };
        let y = match bar.strip.edge {
            Edge::Top => bar.strip.bounds.bottom + gap,
            Edge::Bottom => bar.strip.bounds.top - height - gap,
            _ => anchor.top + (anchor.height() - height) / 2,
        };
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            CLASS,
            caption.as_ptr(),
            WS_POPUP,
            x.clamp(display.left, display.right - width),
            y.clamp(display.top, display.bottom - height),
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        let radius = px(12);
        let round = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
        if !round.is_null() && SetWindowRgn(hwnd, round, 0) == 0 {
            DeleteObject(round);
        }
        Some(hwnd)
    }
}

pub(super) unsafe fn font(dpi: u32) -> HFONT {
    unsafe {
        CreateFontW(
            -13 * dpi as i32 / 96,
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
            0,
            windows_sys::w!("Segoe UI"),
        )
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                if !dc.is_null() {
                    let mut r = RECT::default();
                    GetClientRect(hwnd, &mut r);
                    let (background, foreground) = crate::appearance::app_label_colors();
                    SetDCBrushColor(dc, background);
                    FillRect(dc, &r, GetStockObject(DC_BRUSH) as HBRUSH);
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, foreground);
                    let font = font(GetDpiForWindow(hwnd).max(96));
                    let previous = SelectObject(dc, font);
                    let mut caption = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
                    GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32);
                    InflateRect(&mut r, -6, 0);
                    DrawTextW(
                        dc,
                        caption.as_ptr(),
                        -1,
                        &mut r,
                        DT_CENTER | DT_END_ELLIPSIS | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                    );
                    SelectObject(dc, previous);
                    DeleteObject(font);
                    EndPaint(hwnd, &paint);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
