//! A short, input-transparent piece of the island's surface between a docked
//! shell panel and its button. The shell owns its menu geometry and input.
use crate::{
    policy::{
        BarInput, Edge, NativeBar, Point, PointerFrame, Rect, START_MAX_DIP, Slide, anchored_start,
        joined_start_face,
    },
    settings::Settings,
};
use std::{
    ptr::{null, null_mut},
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::WindowsAndMessaging::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuKind {
    Start,
    Search,
    QuickSettings,
    Notifications,
    Overflow,
    JumpList,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Panel {
    pub hwnd: HWND,
    pub monitor: Rect,
    pub bounds: Rect,
    pub kind: MenuKind,
}

fn px(dip: i32, dpi: u32) -> i32 {
    ((i64::from(dip) * i64::from(if dpi == 0 { 96 } else { dpi }) + 48) / 96)
        .min(i64::from(i32::MAX)) as i32
}

fn gap(bar: NativeBar, island: Rect, panel: Panel, dpi: u32) -> Option<Rect> {
    if panel.monitor != bar.display {
        return None;
    }
    let face = island.intersection(bar.bounds)?;
    let host = panel.bounds.intersection(bar.display)?;
    let max_gap = px(48, dpi);
    let gutter = px(12, dpi);
    let rect = match bar.edge {
        Edge::Bottom => {
            // The visible face occupies only part of the transparent Start/Search
            // host. Notification and Quick Settings have known, right-aligned faces.
            let (left, right, inset) = match panel.kind {
                MenuKind::Start => {
                    let start = anchored_start(bar, face, host.height(), dpi);
                    (
                        start.left.max(host.left),
                        start.right.min(host.right),
                        gutter,
                    )
                }
                MenuKind::Search => (face.left.max(host.left), face.right.min(host.right), gutter),
                MenuKind::Notifications | MenuKind::QuickSettings => {
                    let width = px(
                        if panel.kind == MenuKind::Notifications {
                            332
                        } else {
                            344
                        },
                        dpi,
                    );
                    let right = host.right.saturating_sub(gutter);
                    (right.saturating_sub(width).max(host.left), right, gutter)
                }
                MenuKind::Overflow | MenuKind::JumpList => (host.left, host.right, 0),
            };
            Rect::new(
                face.left.max(left),
                host.bottom.saturating_sub(inset),
                face.right.min(right),
                face.top,
            )
        }
        // For side/top placements, trust only a physically adjacent bounded popup.
        // Never extrapolate a painted face from a full-screen shell host.
        Edge::Top => Rect::new(
            face.left.max(host.left),
            face.bottom,
            face.right.min(host.right),
            host.top,
        ),
        Edge::Left => Rect::new(
            face.right,
            face.top.max(host.top),
            host.left,
            face.bottom.min(host.bottom),
        ),
        Edge::Right => Rect::new(
            host.right,
            face.top.max(host.top),
            face.left,
            face.bottom.min(host.bottom),
        ),
    };
    let (length, depth) = if bar.edge.horizontal() {
        (rect.width(), i64::from(rect.bottom) - i64::from(rect.top))
    } else {
        (rect.height(), i64::from(rect.right) - i64::from(rect.left))
    };
    if depth < 0 || depth > i64::from(max_gap) || length < px(8, dpi) {
        return None;
    }
    if depth == 0 {
        // An already touching Start still needs its lateral endcaps at the width limit.
        if panel.kind != MenuKind::Start {
            return None;
        }
    } else if rect.intersection(bar.display) != Some(rect) {
        return None;
    }
    Some(rect)
}

fn reveal(gap: Rect, edge: Edge, anchor: Point, progress: f64) -> Option<Rect> {
    if progress <= 0.0 || gap.is_empty() {
        return None;
    }
    if progress >= 1.0 {
        return Some(gap);
    }
    let horizontal = edge.horizontal();
    let (lo, hi, pointer) = if horizontal {
        (gap.left, gap.right, anchor.x)
    } else {
        (gap.top, gap.bottom, anchor.y)
    };
    let anchor = pointer.clamp(lo, hi);
    let scale = |n: i32| (f64::from(n) * progress).round() as i32;
    let a = anchor - scale(anchor - lo);
    let b = anchor + scale(hi - anchor);
    let depth = scale(if horizontal {
        gap.height()
    } else {
        gap.width()
    });
    let frame = match edge {
        Edge::Bottom => Rect::new(a, gap.bottom - depth, b, gap.bottom),
        Edge::Top => Rect::new(a, gap.top, b, gap.top + depth),
        Edge::Left => Rect::new(gap.left, a, gap.left + depth, b),
        Edge::Right => Rect::new(gap.right - depth, a, gap.right, b),
    };
    (!frame.is_empty()).then_some(frame)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wing {
    bounds: Rect,
    low: bool,
    tapered: bool,
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.left.min(b.left),
        a.top.min(b.top),
        a.right.max(b.right),
        a.bottom.max(b.bottom),
    )
}

fn wings(bar: NativeBar, face: Rect, panel: Panel, gap: Rect, dpi: u32) -> [Option<Wing>; 2] {
    if panel.kind != MenuKind::Start {
        return [None; 2];
    }
    let Some(host) = panel.bounds.intersection(bar.display) else {
        return [None; 2];
    };
    let horizontal = bar.edge.horizontal();
    let contact = if bar.edge == Edge::Bottom {
        let native = anchored_start(bar, face, host.height(), dpi);
        // The panel's transparent wrapper can be screen-wide. Clip its projected
        // painted width to the real native window before drawing either extension.
        Rect::new(
            native.left.max(host.left),
            native.top,
            native.right.min(host.right),
            native.bottom,
        )
    } else {
        host
    };
    let goal = joined_start_face(bar.edge, face, contact);
    let axis = |r: Rect| {
        if horizontal {
            (r.left, r.right)
        } else {
            (r.top, r.bottom)
        }
    };
    let (old0, old1) = axis(face);
    let (new0, new1) = axis(goal);
    if new1 <= old0 || old1 <= new0 || new1.saturating_sub(new0) > px(START_MAX_DIP + 24, dpi) {
        return [None; 2];
    }
    let envelope = union(face, gap);
    std::array::from_fn(|i| {
        let low = i == 0;
        let (a, b) = if low { (old0, new0) } else { (old1, new1) };
        if a == b {
            return None;
        }
        // A wider Start adds painted endcaps beside the native bar. A narrower
        // Start tapers across the gap while all existing native icons remain visible.
        let tapered = if low { b > a } else { b < a };
        let band = if tapered { gap } else { envelope };
        let bounds = if horizontal {
            Rect::new(a.min(b), band.top, a.max(b), band.bottom)
        } else {
            Rect::new(band.left, a.min(b), band.right, a.max(b))
        };
        Some(Wing {
            bounds: bounds.intersection(bar.display)?,
            low,
            tapered,
        })
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Frame {
    bridge: Option<Rect>,
    wings: [Option<Wing>; 2],
    edge: Edge,
    radius: i32,
}

impl Frame {
    fn bounds(self) -> Option<Rect> {
        self.wings
            .iter()
            .flatten()
            .map(|w| w.bounds)
            .chain(self.bridge)
            .reduce(union)
    }

    fn region(self, origin: Rect) -> HRGN {
        let local = |r: Rect| {
            Rect::new(
                r.left - origin.left,
                r.top - origin.top,
                r.right - origin.left,
                r.bottom - origin.top,
            )
        };
        let bridge = self.bridge.map(local).unwrap_or_default();
        let region = unsafe { CreateRectRgn(bridge.left, bridge.top, bridge.right, bridge.bottom) };
        if region.is_null() {
            return region;
        }
        for wing in self.wings.into_iter().flatten() {
            let r = local(wing.bounds);
            let horizontal = self.edge.horizontal();
            let (length, depth) = if horizontal {
                (r.width(), r.height())
            } else {
                (r.height(), r.width())
            };
            let pt = |u: i32, v: i32| {
                let u = if wing.low { length - u } else { u };
                let v = if matches!(self.edge, Edge::Top | Edge::Left) {
                    depth - v
                } else {
                    v
                };
                if horizontal {
                    POINT {
                        x: r.left + u,
                        y: r.top + v,
                    }
                } else {
                    POINT {
                        x: r.left + v,
                        y: r.top + u,
                    }
                }
            };
            let radius = self.radius.min(length).min(depth);
            let shoulder = [pt(0, 0), pt(length, depth), pt(0, depth)];
            let corner = radius / 3;
            let endcap = [
                pt(0, 0),
                pt(length, 0),
                pt(length, depth - radius),
                pt(length - corner, depth - corner),
                pt(length - radius, depth),
                pt(0, depth),
            ];
            let points: &[POINT] = if wing.tapered { &shoulder } else { &endcap };
            let part = unsafe { CreatePolygonRgn(points.as_ptr(), points.len() as i32, WINDING) };
            if part.is_null() {
                unsafe {
                    DeleteObject(region);
                }
                return null_mut();
            }
            let ok = unsafe { CombineRgn(region, region, part, RGN_OR) };
            unsafe {
                DeleteObject(part);
            }
            if ok == ERROR {
                unsafe {
                    DeleteObject(region);
                }
                return null_mut();
            }
        }
        region
    }
}

fn animated_frame(t: Target, progress: f64) -> Frame {
    let edge = t.edge;
    let wings = t.wings.map(|wing| {
        wing.and_then(|mut wing| {
            let mut r = wing.bounds;
            let size = if edge.horizontal() {
                r.width()
            } else {
                r.height()
            };
            let size = (f64::from(size) * progress).round() as i32;
            if size < 1 {
                return None;
            }
            match (edge.horizontal(), wing.low) {
                (true, true) => r.left = r.right - size,
                (true, false) => r.right = r.left + size,
                (false, true) => r.top = r.bottom - size,
                (false, false) => r.bottom = r.top + size,
            }
            wing.bounds = r;
            Some(wing)
        })
    });
    Frame {
        bridge: reveal(t.gap, edge, t.anchor, progress),
        wings,
        edge,
        radius: t.radius,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn material(island: Rect, gap: Rect, edge: Edge, dpi: u32) -> COLORREF {
    // Sample a single taskbar background pixel, after proving the point belongs
    // to Explorer's taskbar. This keeps the join tinted by the same wallpaper.
    let inset = px(2, dpi).max(1);
    let x = match edge {
        Edge::Left => island.right.saturating_sub(inset),
        Edge::Right => island.left.saturating_add(inset),
        _ => gap.left + gap.width() / 2,
    }
    .clamp(island.left, island.right - 1);
    let y = match edge {
        Edge::Top => island.bottom.saturating_sub(inset),
        Edge::Bottom => island.top.saturating_add(inset),
        _ => gap.top + gap.height() / 2,
    }
    .clamp(island.top, island.bottom - 1);
    let hit = unsafe { GetAncestor(WindowFromPhysicalPoint(POINT { x, y }), GA_ROOT) };
    let mut name = [0u16; 48];
    let length =
        unsafe { GetClassNameW(hit, name.as_mut_ptr(), name.len() as i32) }.max(0) as usize;
    if matches!(
        String::from_utf16_lossy(&name[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        let dc = unsafe { GetDC(null_mut()) };
        if !dc.is_null() {
            let color = unsafe { GetPixel(dc, x, y) };
            unsafe { ReleaseDC(null_mut(), dc) };
            if color != CLR_INVALID {
                return color;
            }
        }
    }
    let mut value = 1u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    if value == 0 { 0x252525 } else { 0xe8e8e8 }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            if !dc.is_null() {
                let brush = GetStockObject(DC_BRUSH);
                SetDCBrushColor(dc, GetWindowLongPtrW(hwnd, GWLP_USERDATA) as COLORREF);
                FillRect(dc, &paint.rcPaint, brush);
            }
            EndPaint(hwnd, &paint);
            0
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn create_overlay() -> HWND {
    static CLASS: OnceLock<u16> = OnceLock::new();
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = *CLASS.get_or_init(|| {
        let name = wide("SmartTaskbar.Rust.IslandJoin");
        let def = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&def) }
    });
    if class == 0 {
        return null_mut();
    }
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class as usize as *const u16,
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    }
}

#[derive(Clone, Copy)]
struct Target {
    panel: Panel,
    face: Rect,
    gap: Rect,
    wings: [Option<Wing>; 2],
    radius: i32,
    anchor: Point,
    edge: Edge,
    dpi: u32,
    transparency: u8,
    color: COLORREF,
}

pub struct Join {
    overlay: HWND,
    target: Option<Target>,
    slide: Slide,
    last: Option<(Frame, COLORREF, u8)>,
}

impl Default for Join {
    fn default() -> Self {
        let mut slide = Slide::default();
        slide.update(false, 0, 0);
        Self {
            overlay: null_mut(),
            target: None,
            slide,
            last: None,
        }
    }
}

impl Join {
    pub fn clear(&mut self) {
        if self.last.is_some() && !self.overlay.is_null() {
            unsafe { ShowWindow(self.overlay, SW_HIDE) };
        }
        self.target = None;
        self.last = None;
        self.slide.update(false, 0, 0);
    }

    pub fn update(
        &mut self,
        surface: BarInput,
        panel: Option<Panel>,
        pointer: PointerFrame,
        settings: &Settings,
    ) -> bool {
        let BarInput { bar, island, dpi } = surface;
        let PointerFrame {
            cursor,
            now_ms: now,
        } = pointer;
        let Some(face) = island.and_then(|r| r.intersection(bar.bounds)) else {
            self.clear();
            return false;
        };
        if !settings.island
            || self
                .target
                .is_some_and(|t| t.panel.monitor != bar.display || t.edge != bar.edge)
        {
            self.clear();
            if !settings.island {
                return false;
            }
        }
        let mut open = false;
        if let Some(panel) = panel {
            if !settings.match_start_search_width
                && matches!(panel.kind, MenuKind::Start | MenuKind::Search)
            {
                self.clear();
                return false;
            }
            let Some(gap) = gap(bar, face, panel, dpi) else {
                self.clear();
                return false;
            };
            if panel.hwnd.is_null() || unsafe { IsWindowVisible(panel.hwnd) } == 0 {
                self.clear();
                return false;
            }
            let previous = self
                .target
                .filter(|t| t.panel.hwnd == panel.hwnd && t.panel.kind == panel.kind);
            if previous.is_none() {
                self.clear()
            }
            let anchor = previous.map(|t| t.anchor).unwrap_or_else(|| {
                if face.contains(cursor) {
                    return cursor;
                }
                Point {
                    x: gap.left + gap.width() / 2,
                    y: gap.top + gap.height() / 2,
                }
            });
            let wings = wings(bar, face, panel, gap, dpi);
            if gap.is_empty() && wings.iter().all(Option::is_none) {
                self.clear();
                return false;
            }
            let color = previous
                .filter(|t| {
                    t.face == face && t.dpi == dpi && t.transparency == settings.transparency
                })
                .map_or_else(|| material(face, gap, bar.edge, dpi), |t| t.color);
            self.target = Some(Target {
                panel,
                face,
                gap,
                wings,
                radius: px(i32::from(settings.radius), dpi),
                anchor,
                edge: bar.edge,
                dpi,
                transparency: settings.transparency,
                color,
            });
            open = true;
        }
        let Some(target) = self.target else {
            return false;
        };
        let duration = if settings.animate { 145 } else { 0 };
        let progress = self.slide.update(open, now, duration);
        let running = if open { progress < 1.0 } else { progress > 0.0 };
        let frame = animated_frame(target, progress);
        let Some(bounds) = frame.bounds() else {
            if self.last.take().is_some() && !self.overlay.is_null() {
                unsafe { ShowWindow(self.overlay, SW_HIDE) };
            }
            if !open && !running {
                self.clear()
            }
            return running;
        };
        if self.overlay.is_null() {
            self.overlay = create_overlay()
        }
        if self.overlay.is_null() {
            return false;
        }
        let alpha = (255.0 * progress).round() as u8;
        if self.last == Some((frame, target.color, alpha)) {
            return running;
        }
        unsafe {
            SetWindowLongPtrW(self.overlay, GWLP_USERDATA, target.color as isize);
            SetLayeredWindowAttributes(self.overlay, 0, alpha, LWA_ALPHA);
            // Put only our new window behind the native panel. Closing can continue in its
            // current z-order after Explorer hides or destroys the original wrapper.
            let flags = SWP_NOACTIVATE
                | SWP_NOOWNERZORDER
                | SWP_SHOWWINDOW
                | if open { 0 } else { SWP_NOZORDER };
            // The HWND's bounding box can contain the native bar. Apply the exact
            // union first so the overlay can never paint across its real controls.
            let mask = frame.region(bounds);
            if mask.is_null() {
                self.clear();
                return false;
            }
            if SetWindowRgn(self.overlay, mask, 0) == 0 {
                DeleteObject(mask);
                self.clear();
                return false;
            }
            if SetWindowPos(
                self.overlay,
                if open { target.panel.hwnd } else { null_mut() },
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
                flags,
            ) == 0
            {
                self.clear();
                return false;
            }
            InvalidateRect(self.overlay, null::<RECT>(), 0);
            UpdateWindow(self.overlay);
        }
        self.last = Some((frame, target.color, alpha));
        running
    }
}

impl Drop for Join {
    fn drop(&mut self) {
        if !self.overlay.is_null() {
            unsafe { DestroyWindow(self.overlay) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{policy::island_rect, settings::Alignment};

    const SCREEN: Rect = Rect::new(-1920, -1080, 0, 0);
    fn native(edge: Edge) -> NativeBar {
        let bounds = match edge {
            Edge::Top => Rect::new(-1920, -1080, 0, -1032),
            Edge::Bottom => Rect::new(-1920, -48, 0, 0),
            Edge::Left => Rect::new(-1920, -1080, -1872, 0),
            Edge::Right => Rect::new(-48, -1080, 0, 0),
        };
        NativeBar {
            display: SCREEN,
            bounds,
            edge,
        }
    }
    fn panel(bounds: Rect, kind: MenuKind) -> Panel {
        Panel {
            hwnd: null_mut(),
            monitor: SCREEN,
            bounds,
            kind,
        }
    }

    #[test]
    fn start_joins_its_painted_width_inside_a_screen_wide_wrapper() {
        let bar = native(Edge::Bottom);
        let face = island_rect(bar, 564, Alignment::Center, 96);
        let wrapper = panel(
            Rect::new(SCREEN.left, -790, SCREEN.right, -50),
            MenuKind::Start,
        );
        let bridge = gap(bar, face, wrapper, 96).unwrap();
        assert_eq!((bridge.left, bridge.right), (face.left, face.right));
        assert_eq!(bridge.bottom, face.top);
        assert_eq!(bridge.top, -62);
        assert!(bridge.intersection(face).is_none());
        let wrong = Panel {
            monitor: Rect::new(0, -1080, 1920, 0),
            ..wrapper
        };
        assert!(gap(bar, face, wrong, 96).is_none());
        assert!(
            gap(
                bar,
                face,
                panel(Rect::new(-1700, -790, -120, -310), MenuKind::Search),
                96
            )
            .is_none()
        );
        assert!(gap(bar, face, panel(SCREEN, MenuKind::Start), 96).is_none());
    }

    #[test]
    fn right_tray_panels_and_smaller_overflow_only_bridge_the_contact() {
        let bar = native(Edge::Bottom);
        let face = island_rect(bar, 564, Alignment::Center, 96);
        for (kind, width) in [
            (MenuKind::QuickSettings, 344),
            (MenuKind::Notifications, 332),
        ] {
            let right = face.right + 12;
            let bridge = gap(
                bar,
                face,
                panel(Rect::new(right - 450, -760, right, -50), kind),
                96,
            )
            .unwrap();
            assert_eq!(bridge.width(), width);
            assert_eq!(bridge.right, face.right);
        }
        for kind in [MenuKind::Overflow, MenuKind::JumpList] {
            let host = Rect::new(face.right - 100, -300, face.right + 130, -66);
            let seam = gap(bar, face, panel(host, kind), 96).unwrap();
            assert_eq!(seam.width(), 100);
            assert_eq!(seam.top, host.bottom);
        }
    }

    #[test]
    fn side_and_top_joins_require_a_real_adjacent_shell_panel() {
        for edge in [Edge::Top, Edge::Left, Edge::Right] {
            let bar = native(edge);
            let face = island_rect(bar, 500, Alignment::Center, 96);
            let bounds = match edge {
                Edge::Top => Rect::new(face.left, face.bottom + 10, face.right, face.bottom + 600),
                Edge::Left => Rect::new(face.right + 10, face.top, face.right + 600, face.bottom),
                Edge::Right => Rect::new(face.left - 600, face.top, face.left - 10, face.bottom),
                Edge::Bottom => unreachable!(),
            };
            let seam = gap(bar, face, panel(bounds, MenuKind::Start), 96).unwrap();
            assert_eq!(
                if edge.horizontal() {
                    seam.height()
                } else {
                    seam.width()
                },
                10
            );
            assert!(seam.intersection(face).is_none() && seam.intersection(bounds).is_none());
            assert!(gap(bar, face, panel(SCREEN, MenuKind::Start), 96).is_none());
        }
    }

    #[test]
    fn seam_animates_from_invoking_point_stays_in_gap_and_reverses_continuously() {
        let seam = Rect::new(-500, -80, -100, -60);
        let cursor = Point { x: -470, y: -65 };
        let mut motion = Slide::default();
        motion.update(false, 0, 0);
        assert!(reveal(seam, Edge::Bottom, cursor, motion.update(true, 10, 100)).is_none());
        let half = reveal(seam, Edge::Bottom, cursor, motion.update(true, 60, 100)).unwrap();
        assert_eq!(half, Rect::new(-485, -70, -285, -60));
        assert_eq!(
            reveal(seam, Edge::Bottom, cursor, motion.update(false, 60, 100)),
            Some(half)
        );
        assert!(reveal(seam, Edge::Bottom, cursor, motion.update(false, 110, 100)).is_none());
        assert_eq!(
            reveal(seam, Edge::Bottom, cursor, motion.update(true, 120, 0)),
            Some(seam)
        );
        for edge in [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom] {
            for n in 0..=100 {
                if let Some(r) = reveal(seam, edge, cursor, f64::from(n) / 100.0) {
                    assert_eq!(r.intersection(seam), Some(r));
                }
            }
        }
    }

    #[test]
    fn minimum_start_extends_only_beside_native_buttons_even_without_a_gap() {
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            let bar = native(edge);
            let face = island_rect(bar, 80, Alignment::Center, 96);
            let expected = anchored_start(bar, face, 400, 96);
            let host = match edge {
                Edge::Bottom => Rect::new(SCREEN.left, -700, SCREEN.right, face.top + 12),
                _ => expected,
            };
            let popup = panel(host, MenuKind::Start);
            let contact = gap(bar, face, popup, 96).unwrap();
            assert!(contact.is_empty());
            let arms = wings(bar, face, popup, contact, 96);
            assert!(arms.iter().all(|w| w.is_some_and(|w| !w.tapered && w.bounds.intersection(face).is_none())));
            let painted = Frame {
                bridge: None,
                wings: arms,
                edge,
                radius: 8,
            };
            let envelope = painted.bounds().unwrap();
            assert_eq!(envelope, joined_start_face(edge, face, expected));
            let mask = painted.region(envelope);
            assert!(!mask.is_null());
            // Exercise GDI's exact region off-screen: no desktop HWND or screen access.
            unsafe {
                for x in (face.left..face.right).step_by(3) {
                    for y in (face.top..face.bottom).step_by(3) {
                        assert_eq!(PtInRegion(mask, x - envelope.left, y - envelope.top), 0);
                    }
                }
                for wing in arms.into_iter().flatten() {
                    let x = wing.bounds.left + wing.bounds.width() / 2 - envelope.left;
                    let y = wing.bounds.top + wing.bounds.height() / 2 - envelope.top;
                    assert_ne!(PtInRegion(mask, x, y), 0);
                }
                DeleteObject(mask);
            }
        }
    }

    #[test]
    fn maximum_start_tapers_toward_the_menu_without_clipping_pinned_buttons() {
        let bar = native(Edge::Bottom);
        let face = island_rect(bar, 1200, Alignment::Center, 96);
        let popup = panel(
            Rect::new(SCREEN.left, -750, SCREEN.right, face.top - 12),
            MenuKind::Start,
        );
        let contact = gap(bar, face, popup, 96).unwrap();
        let arms = wings(bar, face, popup, contact, 96);
        assert!(
            arms.iter()
                .all(|w| w.is_some_and(|w| w.tapered && w.bounds.intersection(face).is_none()))
        );
        let target = Target {
            panel: popup,
            face,
            gap: contact,
            wings: arms,
            radius: 4,
            anchor: Point {
                x: face.left,
                y: face.top,
            },
            edge: bar.edge,
            dpi: 96,
            transparency: 65,
            color: 0x202020,
        };
        let half = animated_frame(target, 0.5);
        let full = animated_frame(target, 1.0);
        assert!(half.wings[0].unwrap().bounds.width() < full.wings[0].unwrap().bounds.width());
        let bounds = full.bounds().unwrap();
        let mask = full.region(bounds);
        assert!(!mask.is_null());
        let mid = arms[0].unwrap().bounds;
        let x = mid.left + mid.width() / 2 - bounds.left;
        unsafe {
            assert_eq!(PtInRegion(mask, x, 1), 0);
            assert_ne!(PtInRegion(mask, x, bounds.height() - 2), 0);
            DeleteObject(mask);
        }
    }
}
