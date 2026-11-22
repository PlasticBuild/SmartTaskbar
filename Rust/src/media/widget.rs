use super::{Action, Art, Timeline, Track, Worker};
use crate::{
    policy::{Edge, NativeBar, Point, Rect, Slide},
    settings::{MediaSide, Settings},
};
use std::{
    ptr::{null, null_mut},
    sync::{Arc, OnceLock},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{
        Controls::WM_MOUSELEAVE,
        Input::KeyboardAndMouse::{
            GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
        },
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub bounds: Rect,
    vertical: bool,
    from_end: bool,
    attached: bool,
}

pub fn attached_placement(
    bar: NativeBar,
    face: Rect,
    reserved: i32,
    dpi: u32,
) -> Option<Placement> {
    let vertical = !bar.edge.horizontal();
    let from_end = reserved > 0;
    let length = reserved.abs() * dpi as i32 / 96;
    let inset = (3 * dpi as i32 / 96).max(1);
    let bounds = if vertical {
        let y = if from_end {
            face.top - length
        } else {
            face.bottom
        };
        Rect::new(face.left + inset, y, face.right - inset, y + length)
    } else {
        let x = if from_end {
            face.left - length
        } else {
            face.right
        };
        Rect::new(x, face.top + inset, x + length, face.bottom - inset)
    };
    let center_delta = if vertical {
        bounds.top.min(face.top) + bounds.bottom.max(face.bottom)
            - bar.display.top
            - bar.display.bottom
    } else {
        bounds.left.min(face.left) + bounds.right.max(face.right)
            - bar.display.left
            - bar.display.right
    };
    // Let Explorer's matching click-through region settle before revealing the controls.
    (length > 0
        && bounds.intersection(bar.display) == Some(bounds)
        && center_delta.abs() <= inset * 10)
        .then_some(Placement {
            bounds,
            vertical,
            from_end,
            attached: true,
        })
}

impl Placement {
    pub fn offset(mut self, offset: Point) -> Self {
        let r = self.bounds;
        self.bounds = Rect::new(
            r.left + offset.x,
            r.top + offset.y,
            r.right + offset.x,
            r.bottom + offset.y,
        );
        self
    }
    fn length(self) -> i32 {
        if self.vertical {
            self.bounds.height()
        } else {
            self.bounds.width()
        }
    }
}

pub fn placement(
    bar: NativeBar,
    island: Option<Rect>,
    side: MediaSide,
    dpi: u32,
) -> Option<Placement> {
    let low = side == MediaSide::Left;
    let px = |value: i32| value * dpi.max(24) as i32 / 96;
    let gap = px(5);
    let display = bar.display;
    let native = display.intersection(bar.bounds)?;
    let face = island.and_then(|r| r.intersection(native));
    if !bar.edge.horizontal() {
        let width = native.width().min(px(56));
        let height = px(190).min(display.height() - gap * 2);
        if width < px(32) || height < px(135) || width + gap * 2 > display.width() {
            return None;
        }
        let fitted = face.and_then(|face| {
            let space = if low {
                face.top - display.top
            } else {
                display.bottom - face.bottom
            };
            let inline = height.min(space - gap * 2);
            if inline < px(135) {
                return None;
            }
            let x = native.left + (native.width() - width) / 2;
            let y = if low {
                face.top - gap - inline
            } else {
                face.bottom + gap
            };
            Some(Rect::new(x, y, x + width, y + inline))
        });
        let bounds = fitted.or_else(|| {
            let anchor = face.unwrap_or(native);
            let x = if bar.edge == Edge::Left {
                native.right + gap
            } else {
                native.left - width - gap
            };
            let y = (if low {
                anchor.top
            } else {
                anchor.bottom - height
            })
            .clamp(display.top + gap, display.bottom - gap - height);
            let bounds = Rect::new(x, y, x + width, y + height);
            (bounds.intersection(display) == Some(bounds)).then_some(bounds)
        })?;
        return Some(Placement {
            bounds,
            vertical: true,
            from_end: low,
            attached: false,
        });
    }
    let width = px(270).min(display.width() - gap * 2);
    if width < px(172) {
        return None;
    }
    let depth = px(46);
    if let Some(face) = face
        && native.height() >= px(35)
    {
        let slot = if low {
            face.left - display.left
        } else {
            display.right - face.right
        };
        let inline = width.min(slot - 2 * gap);
        if inline >= px(172) {
            let x = if low {
                face.left - gap - inline
            } else {
                face.right + gap
            };
            let height = depth.min(native.height());
            let y = native.top + (native.height() - height) / 2;
            return Some(Placement {
                bounds: Rect::new(x, y, x + inline, y + height),
                vertical: false,
                from_end: low,
                attached: false,
            });
        }
    }
    // When no inline space is free, attach on the inward side so no native target is covered.
    let anchor = face.unwrap_or(native);
    let (x, y) = (
        if low {
            anchor.left + gap
        } else {
            anchor.right - gap - width
        },
        if bar.edge == Edge::Bottom {
            native.top - gap - depth
        } else {
            native.bottom + gap
        },
    );
    if display.height() < depth + gap * 2 {
        return None;
    }
    let x = x.clamp(display.left + gap, display.right - gap - width);
    let y = y.clamp(display.top + gap, display.bottom - gap - depth);
    let bounds = Rect::new(x, y, x + width, y + depth);
    bounds.intersection(native).is_none().then_some(Placement {
        bounds,
        vertical: false,
        from_end: low,
        attached: false,
    })
}

#[derive(Clone, Copy)]
struct Layout {
    label: Rect,
    buttons: [Rect; 3],
    scrub: Rect,
    vertical: bool,
}

impl Layout {
    fn details(w: i32, h: i32, dpi: u32) -> Self {
        let px = |n: i32| n * dpi as i32 / 96;
        let button = px(44).min(w / 5);
        let x = (w - button * 3) / 2;
        let top = px(51);
        Self {
            label: Rect::new(px(16), px(12), w - px(16), top),
            buttons: std::array::from_fn(|i| {
                let x = x + button * i as i32;
                Rect::new(x, top, x + button, top + button)
            }),
            scrub: Rect::new(px(16), top + button + px(6), w - px(16), h - px(25)),
            vertical: false,
        }
    }

    fn new(w: i32, h: i32, dpi: u32) -> Self {
        let px = |n: i32| n * dpi as i32 / 96;
        let gap = px(6);
        if h > w {
            let button_h = px(29).min((h - gap * 4) / 4);
            let start = gap;
            let buttons = std::array::from_fn(|i| {
                let y = start + i as i32 * button_h;
                Rect::new(gap, y, w - gap, y + button_h)
            });
            return Self {
                label: Rect::default(),
                buttons,
                scrub: Rect::new(
                    w / 2 - px(9),
                    buttons[2].bottom + gap,
                    w / 2 + px(9),
                    h - gap,
                ),
                vertical: true,
            };
        }
        let button = px(24);
        let control_x = w - gap - 3 * button;
        let controls = std::array::from_fn(|i| {
            let x = control_x + i as i32 * button;
            Rect::new(x, px(1), x + button, h - px(11))
        });
        Self {
            label: Rect::new(gap * 2, px(3), control_x - gap, h - px(12)),
            buttons: controls,
            scrub: Rect::new(gap * 2, h - px(13), w - gap - px(1), h - px(1)),
            vertical: false,
        }
    }

    fn scrub_fraction(self, point: Point) -> f64 {
        let (offset, length) = if self.vertical {
            (self.scrub.bottom - point.y, self.scrub.height())
        } else {
            (point.x - self.scrub.left, self.scrub.width())
        };
        (f64::from(offset) / f64::from(length.max(1))).clamp(0.0, 1.0)
    }
}

type Controller = std::rc::Rc<dyn Fn(u64, Action)>;
struct Drag {
    generation: u64,
    timeline: Timeline,
    fraction: f64,
}
struct Ui {
    track: Option<Arc<Track>>,
    control: Controller,
    dpi: u32,
    light: bool,
    drag: Option<Drag>,
    hover: Option<usize>,
    expanded: bool,
}

impl Ui {
    fn new(control: Controller) -> Self {
        Self {
            track: None,
            control,
            dpi: 96,
            light: false,
            drag: None,
            hover: None,
            expanded: false,
        }
    }

    fn layout(&self, width: i32, height: i32) -> Layout {
        if self.expanded {
            Layout::details(width, height, self.dpi)
        } else {
            Layout::new(width, height, self.dpi)
        }
    }
}

struct Details {
    hwnd: HWND,
    ui: Box<Ui>,
    bounds: Rect,
    shown_at: u64,
    alpha: u8,
    draw_at: u64,
}

impl Drop for Details {
    fn drop(&mut self) {
        unsafe {
            if GetCapture() == self.hwnd {
                ReleaseCapture();
            }
            DestroyWindow(self.hwnd);
        }
    }
}

pub struct Widget {
    hwnd: HWND,
    ui: Option<Box<Ui>>,
    slide: Slide,
    frame: Option<(Placement, i32, u8, i32)>,
    draw_at: u64,
    details: Option<Details>,
    hover_at: Option<u64>,
    details_until: u64,
}

impl Default for Widget {
    fn default() -> Self {
        let mut slide = Slide::default();
        slide.update(false, 0, 0);
        Self {
            hwnd: null_mut(),
            ui: None,
            slide,
            frame: None,
            draw_at: 0,
            details: None,
            hover_at: None,
            details_until: 0,
        }
    }
}

impl Widget {
    pub fn is_visible(&self) -> bool {
        self.frame.is_some()
    }

    pub fn holds(&self, pointer: Point) -> bool {
        if self
            .details
            .as_ref()
            .is_some_and(|detail| detail.bounds.contains(pointer))
        {
            return true;
        }
        self.frame.is_some_and(|(place, span, _, _)| {
            span == place.length()
                && place.bounds.contains(pointer)
                && unsafe {
                    GetAncestor(
                        WindowFromPhysicalPoint(POINT {
                            x: pointer.x,
                            y: pointer.y,
                        }),
                        GA_ROOT,
                    ) == self.hwnd
                }
        })
    }

    pub fn clear(&mut self, now: u64) {
        self.close_details();
        if !self.hwnd.is_null() {
            if unsafe { GetCapture() == self.hwnd } {
                unsafe { ReleaseCapture() };
            }
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = null_mut();
            self.ui = None;
        }
        self.frame = None;
        self.slide.update(false, now, 0);
    }

    fn close_details(&mut self) {
        self.details = None;
        self.hover_at = None;
        self.details_until = 0;
    }

    pub fn hover_details(
        &mut self,
        bar: NativeBar,
        pointer: Point,
        animate: bool,
        now: u64,
    ) -> bool {
        let Some((compact, span, _, _)) = self.frame else {
            self.close_details();
            return false;
        };
        let Some(ui) = &self.ui else { return false };
        let Some(track) = ui.track.clone() else {
            return false;
        };
        let over_player =
            span == compact.length() && compact.bounds.contains(pointer) && self.holds(pointer);
        if over_player {
            self.hover_at.get_or_insert(now);
        } else {
            self.hover_at = None;
        }
        if self.hover_at.is_none() && self.details.is_none() {
            return false;
        }
        let Some(bounds) = detail_bounds(bar, compact.bounds, ui.dpi) else {
            self.close_details();
            return false;
        };
        if let Some(detail) = &mut self.details {
            if detail.bounds != bounds {
                self.close_details();
                return false;
            }
            let in_panel = bounds.contains(pointer) || unsafe { GetCapture() == detail.hwnd };
            if over_player || in_panel {
                self.details_until = now.saturating_add(280);
            }
            if now > self.details_until {
                self.close_details();
                return false;
            }
            if detail
                .ui
                .drag
                .as_ref()
                .is_some_and(|drag| drag.generation != track.generation)
            {
                detail.ui.drag = None;
            }
            let changed = detail
                .ui
                .track
                .as_ref()
                .is_none_or(|previous| !Arc::ptr_eq(previous, &track))
                || detail.ui.light != ui.light;
            let advances = track.playing && track.timeline.is_some() && detail.ui.drag.is_none();
            detail.ui.track = Some(track);
            detail.ui.light = ui.light;
            let alpha = if animate {
                (now.saturating_sub(detail.shown_at) * 255 / 145).min(255) as u8
            } else {
                255
            };
            if alpha != detail.alpha {
                unsafe {
                    SetLayeredWindowAttributes(detail.hwnd, 0, alpha, LWA_ALPHA);
                }
                detail.alpha = alpha;
            }
            // Pointer handling and fades can run faster than the seek display needs to paint.
            if changed || (advances && now >= detail.draw_at) {
                detail.draw_at = now + 100;
                unsafe { InvalidateRect(detail.hwnd, null(), 0) };
            }
            return true;
        }
        let Some(since) = self.hover_at else {
            return false;
        };
        if now.saturating_sub(since) < 300 {
            return true;
        }
        let mut expanded = Box::new(Ui::new(ui.control.clone()));
        expanded.expanded = true;
        expanded.dpi = ui.dpi;
        expanded.light = ui.light;
        expanded.track = Some(track);
        let hwnd = create(&mut expanded);
        if hwnd.is_null() {
            return false;
        }
        let alpha = if animate { 1 } else { 255 };
        let radius = ui.dpi as i32 * 24 / 96;
        let details = Details {
            hwnd,
            ui: expanded,
            bounds,
            shown_at: now,
            alpha,
            draw_at: now + 100,
        };
        if !place(
            hwnd,
            Placement {
                bounds,
                vertical: false,
                from_end: false,
                attached: false,
            },
            bounds.width(),
            alpha,
            radius,
        ) {
            return false;
        }
        self.details_until = now.saturating_add(280);
        self.details = Some(details);
        true
    }

    pub fn update(
        &mut self,
        target: Placement,
        content: (&Worker, Option<Arc<Track>>),
        dpi: u32,
        settings: &Settings,
        now: u64,
    ) -> bool {
        let (worker, track) = content;
        if self.hwnd.is_null() {
            if track.is_none() {
                return false;
            }
            let mut ui = Box::new(Ui::new(std::rc::Rc::new(worker.controller())));
            self.hwnd = create(&mut ui);
            if self.hwnd.is_null() {
                return false;
            }
            self.ui = Some(ui);
        }
        let opening = track.is_some();
        if !opening {
            self.close_details();
        }
        let fraction = self.slide.update(
            opening,
            now,
            if settings.animate {
                super::motion::DURATION
            } else {
                0
            },
        );
        let length = target.length();
        let span = (f64::from(length) * fraction).round() as i32;
        if span == 0 {
            if !opening {
                self.clear(now);
            }
            return opening;
        }
        let ui = self.ui.as_mut().expect("the window owns its player state");
        let dpi = dpi.max(96);
        let changed = track
            .as_ref()
            .zip(ui.track.as_ref())
            .is_some_and(|(a, b)| !Arc::ptr_eq(a, b))
            || (track.is_some() && ui.track.is_none())
            || dpi != ui.dpi;
        if let Some(track) = track {
            if ui
                .drag
                .as_ref()
                .is_some_and(|d| d.generation != track.generation)
            {
                ui.drag = None;
            }
            ui.track = Some(track);
        }
        ui.dpi = dpi;
        if changed || self.frame.is_none() || now >= self.draw_at {
            ui.light = light_theme();
            self.draw_at = now + 500;
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }
        let alpha = (120.0 + 135.0 * fraction).round() as u8;
        let radius = (i32::from(settings.radius).clamp(4, 16) * dpi as i32 / 96) * 2;
        let frame = (target, span, alpha, radius);
        if self.frame != Some(frame) {
            if !place(self.hwnd, target, span, alpha, radius) {
                self.clear(now);
                return false;
            }
            self.frame = Some(frame);
        }
        if opening {
            fraction < 1.0
        } else {
            fraction > 0.0
        }
    }
}

impl Drop for Widget {
    fn drop(&mut self) {
        self.clear(0);
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn detail_bounds(bar: NativeBar, player: Rect, dpi: u32) -> Option<Rect> {
    let d = bar.display;
    let px = |n: i32| n * dpi as i32 / 96;
    let gap = px(7);
    let width = px(312).min(d.width() - gap * 2);
    let height = px(144);
    if width < px(215) || d.height() < height + gap * 2 {
        return None;
    }
    let centered_x =
        (player.left + (player.width() - width) / 2).clamp(d.left + gap, d.right - width - gap);
    let centered_y =
        (player.top + (player.height() - height) / 2).clamp(d.top + gap, d.bottom - height - gap);
    let (x, y) = match bar.edge {
        Edge::Left => (player.right + gap, centered_y),
        Edge::Right => (player.left - gap - width, centered_y),
        Edge::Top => (centered_x, player.bottom + gap),
        Edge::Bottom => (centered_x, player.top - gap - height),
    };
    let bounds = Rect::new(x, y, x + width, y + height);
    (bounds.intersection(d) == Some(bounds)).then_some(bounds)
}

fn create(ui: &mut Ui) -> HWND {
    static CLASS: OnceLock<u16> = OnceLock::new();
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = *CLASS.get_or_init(|| {
        let name = wide("SmartTaskbar.Rust.Media");
        unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: name.as_ptr(),
                ..Default::default()
            })
        }
    });
    if class == 0 {
        return null_mut();
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            class as usize as *const u16,
            wide(if ui.expanded {
                "Taskbar media details"
            } else {
                "Taskbar media player"
            })
            .as_ptr(),
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
    };
    if !hwnd.is_null() {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, ui as *mut Ui as isize) };
    }
    hwnd
}

fn place(hwnd: HWND, placement: Placement, span: i32, alpha: u8, radius: i32) -> bool {
    let r = placement.bounds;
    unsafe {
        let rounded = CreateRoundRectRgn(0, 0, r.width() + 1, r.height() + 1, radius, radius);
        if rounded.is_null() {
            return false;
        }
        if placement.attached {
            let seam = match (placement.vertical, placement.from_end) {
                (true, true) => CreateRectRgn(0, r.height() / 2, r.width() + 1, r.height() + 1),
                (true, false) => CreateRectRgn(0, 0, r.width() + 1, r.height() / 2),
                (false, true) => CreateRectRgn(r.width() / 2, 0, r.width() + 1, r.height() + 1),
                (false, false) => CreateRectRgn(0, 0, r.width() / 2, r.height() + 1),
            };
            if !seam.is_null() {
                CombineRgn(rounded, rounded, seam, RGN_OR);
                DeleteObject(seam);
            }
        }
        if span < placement.length() {
            let offset = if placement.from_end {
                placement.length() - span
            } else {
                0
            };
            let mask = if placement.vertical {
                CreateRectRgn(0, offset, r.width() + 1, offset + span)
            } else {
                CreateRectRgn(offset, 0, offset + span, r.height() + 1)
            };
            if mask.is_null() {
                DeleteObject(rounded);
                return false;
            }
            let code = CombineRgn(rounded, rounded, mask, RGN_AND);
            DeleteObject(mask);
            if code == ERROR {
                DeleteObject(rounded);
                return false;
            }
        }
        if SetWindowRgn(hwnd, rounded, 1) == 0 {
            DeleteObject(rounded);
            return false;
        }
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            r.left,
            r.top,
            r.width(),
            r.height(),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        ) != 0
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Ui;
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if !state.is_null() {
        let pos = Point {
            x: lp as i16 as i32,
            y: (lp >> 16) as i16 as i32,
        };
        match msg {
            WM_PAINT => {
                paint(hwnd, unsafe { &*state });
                return 0;
            }
            WM_LBUTTONDOWN => {
                let ui = unsafe { &mut *state };
                let layout = client_layout(hwnd, ui);
                if let Some(track) = &ui.track {
                    if layout.scrub.contains(pos)
                        && track.can_seek
                        && let Some(timeline) = track.timeline
                    {
                        ui.drag = Some(Drag {
                            generation: track.generation,
                            timeline,
                            fraction: layout.scrub_fraction(pos),
                        });
                        unsafe {
                            SetCapture(hwnd);
                            InvalidateRect(hwnd, null(), 0);
                        }
                    } else {
                        for ((rect, enabled), action) in layout
                            .buttons
                            .into_iter()
                            .zip([track.previous, track.toggle, track.next])
                            .zip([Action::Previous, Action::PlayPause, Action::Next])
                        {
                            if enabled && rect.contains(pos) {
                                (ui.control)(track.generation, action);
                            }
                        }
                    }
                }
                return 0;
            }
            WM_MOUSEMOVE => {
                let ui = unsafe { &mut *state };
                let layout = client_layout(hwnd, ui);
                let hover = layout.buttons.iter().position(|r| r.contains(pos));
                if hover != ui.hover {
                    ui.hover = hover;
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        TrackMouseEvent(&mut tracking);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                if let Some(drag) = &mut ui.drag {
                    drag.fraction = layout.scrub_fraction(pos);
                    unsafe { InvalidateRect(hwnd, null(), 0) };
                }
                return 0;
            }
            WM_MOUSELEAVE => {
                unsafe {
                    (*state).hover = None;
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }
            WM_LBUTTONUP => {
                let ui = unsafe { &mut *state };
                if let Some(drag) = ui.drag.take() {
                    let fraction = client_layout(hwnd, ui).scrub_fraction(pos);
                    (ui.control)(drag.generation, Action::Seek(drag.timeline.at(fraction)));
                }
                if unsafe { GetCapture() == hwnd } {
                    unsafe { ReleaseCapture() };
                }
                unsafe { InvalidateRect(hwnd, null(), 0) };
                return 0;
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                unsafe {
                    (*state).drag = None;
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn client_layout(hwnd: HWND, ui: &Ui) -> Layout {
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r) };
    ui.layout(r.right, r.bottom)
}

fn light_theme() -> bool {
    let mut value = 1u32;
    let mut size = 4;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        );
    }
    value != 0
}

fn fill(dc: HDC, r: Rect, color: COLORREF) {
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(
            dc,
            &RECT {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            },
            GetStockObject(DC_BRUSH),
        );
    }
}

fn text(dc: HDC, label: &str, r: Rect, color: COLORREF, flags: u32) {
    let mut bounds = RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    unsafe {
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            wide(label).as_ptr(),
            -1,
            &mut bounds,
            flags | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

fn artwork(dc: HDC, r: Rect, art: &Art) {
    let w = f64::from(art.width);
    let h = f64::from(art.height);
    let scale = (f64::from(r.width()) / w).max(f64::from(r.height()) / h);
    let width = (w * scale).round() as i32;
    let height = (h * scale).round() as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: art.width as i32,
            biHeight: -(art.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        SetStretchBltMode(dc, HALFTONE);
        StretchDIBits(
            dc,
            r.left + (r.width() - width) / 2,
            r.top + (r.height() - height) / 2,
            width,
            height,
            0,
            0,
            art.width as i32,
            art.height as i32,
            art.pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

fn transport(
    dc: HDC,
    rect: Rect,
    button: usize,
    playing: bool,
    color: COLORREF,
    surface: COLORREF,
    highlighted: bool,
) {
    let x = rect.left + rect.width() / 2;
    let y = rect.top + rect.height() / 2;
    let radius = (rect.width().min(rect.height()) / 2 - 2).max(4);
    let unit = (radius / 3).max(1);
    unsafe {
        let pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        if highlighted || button == 1 {
            SetDCBrushColor(dc, if button == 1 { color } else { surface });
            Ellipse(dc, x - radius, y - radius, x + radius, y + radius);
        }
        let ink = if button == 1 { surface } else { color };
        SetDCBrushColor(dc, ink);
        if button == 1 && playing {
            fill(
                dc,
                Rect::new(x - unit - 1, y - unit - 1, x - 1, y + unit + 1),
                ink,
            );
            fill(
                dc,
                Rect::new(x + 1, y - unit - 1, x + unit + 1, y + unit + 1),
                ink,
            );
        } else {
            let dir = if button == 0 { -1 } else { 1 };
            let shift = if button == 1 { 1 } else { 0 };
            let triangle = [
                POINT {
                    x: x + shift - dir * unit,
                    y: y - unit - 1,
                },
                POINT {
                    x: x + shift + dir * (unit + 1),
                    y,
                },
                POINT {
                    x: x + shift - dir * unit,
                    y: y + unit + 1,
                },
            ];
            Polygon(dc, triangle.as_ptr(), 3);
            if button != 1 {
                let line = x + dir * (unit + 2);
                fill(
                    dc,
                    Rect::new(line - 1, y - unit - 1, line + 1, y + unit + 1),
                    ink,
                );
            }
        }
        SelectObject(dc, brush);
        SelectObject(dc, pen);
    }
}

fn paint(hwnd: HWND, ui: &Ui) {
    unsafe {
        let mut painting = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut painting);
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r);
        let layer = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, r.right, r.bottom);
        if !layer.is_null() && !bitmap.is_null() {
            let old = SelectObject(layer, bitmap);
            paint_surface(layer, r.right, r.bottom, ui);
            BitBlt(dc, 0, 0, r.right, r.bottom, layer, 0, 0, SRCCOPY);
            SelectObject(layer, old);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !layer.is_null() {
            DeleteDC(layer);
        }
        EndPaint(hwnd, &painting);
    }
}

fn paint_surface(dc: HDC, w: i32, h: i32, ui: &Ui) {
    let has_art = ui
        .track
        .as_ref()
        .is_some_and(|track| track.details.art.is_some());
    let (base, backdrop, main, secondary, accent) = if ui.light && !has_art {
        (0xf7f5f2, 0xe3e0dc, 0x2a2320, 0x675a52, 0x995d20)
    } else {
        (0x282323, 0x443934, 0xf8f2ec, 0xc1b3ad, 0xf5bb81)
    };
    fill(dc, Rect::new(0, 0, w, h), base);
    let Some(track) = &ui.track else { return };
    let layout = ui.layout(w, h);
    if let Some(art) = &track.details.art {
        artwork(dc, Rect::new(0, 0, w, h), art);
    }
    let font = |height: i32| unsafe {
        CreateFontW(
            -height * ui.dpi as i32 / 96,
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
            wide("Segoe UI").as_ptr(),
        )
    };
    let regular = font(if ui.expanded { 15 } else { 12 });
    unsafe { SetBkMode(dc, TRANSPARENT as i32) };
    let previous = unsafe { SelectObject(dc, regular) };
    let title = if track.details.title.is_empty() {
        "Playing media"
    } else {
        &track.details.title
    };
    if !layout.vertical
        && (track.details.artist.is_empty() || layout.label.height() < 25 * ui.dpi as i32 / 96)
    {
        text(dc, title, layout.label, main, DT_VCENTER);
    } else if !layout.vertical {
        let mut first = layout.label;
        first.bottom = first.top + first.height() / 2 + 1;
        let mut second = layout.label;
        second.top = first.bottom;
        text(dc, title, first, main, DT_VCENTER);
        text(dc, &track.details.artist, second, secondary, DT_VCENTER);
    }
    for (index, (rect, enabled)) in layout
        .buttons
        .into_iter()
        .zip([track.previous, track.toggle, track.next])
        .enumerate()
    {
        transport(
            dc,
            rect,
            index,
            track.playing,
            if enabled { main } else { secondary },
            base,
            enabled && ui.hover == Some(index),
        );
    }
    let weight = (2 * ui.dpi as i32 / 96).max(1);
    let center = Point {
        x: layout.scrub.left + layout.scrub.width() / 2,
        y: layout.scrub.top + layout.scrub.height() / 2,
    };
    let rail = if layout.vertical {
        Rect::new(
            center.x,
            layout.scrub.top,
            center.x + weight,
            layout.scrub.bottom,
        )
    } else {
        Rect::new(
            layout.scrub.left,
            center.y,
            layout.scrub.right,
            center.y + weight,
        )
    };
    fill(dc, rail, backdrop);
    if let Some(timeline) = track.timeline {
        let fraction = ui
            .drag
            .as_ref()
            .map_or_else(|| timeline.fraction(Instant::now()), |d| d.fraction);
        let mut played = rail;
        let knob = if layout.vertical {
            let y = rail.bottom - (fraction * f64::from(rail.height())).round() as i32;
            played.top = y;
            Rect::new(
                center.x - weight,
                y - weight,
                center.x + 2 * weight,
                y + weight + 1,
            )
        } else {
            let x = rail.left + (fraction * f64::from(rail.width())).round() as i32;
            played.right = x;
            Rect::new(
                x - weight,
                center.y - weight,
                x + weight + 1,
                center.y + 2 * weight,
            )
        };
        fill(dc, played, if track.can_seek { accent } else { secondary });
        if track.can_seek {
            fill(dc, knob, accent);
        }
        if ui.expanded {
            let px = |value| value * ui.dpi as i32 / 96;
            let elapsed = (timeline.at(fraction) - timeline.first) / 10_000_000;
            let total = (timeline.last - timeline.first) / 10_000_000;
            let small = font(11);
            unsafe { SelectObject(dc, small) };
            for (seconds, align) in [(elapsed, DT_LEFT), (total, DT_RIGHT)] {
                text(
                    dc,
                    &format!("{}:{:02}", seconds / 60, seconds % 60),
                    Rect::new(px(16), layout.scrub.bottom + px(2), w - px(16), h - px(5)),
                    secondary,
                    align | DT_VCENTER,
                );
            }
            unsafe {
                SelectObject(dc, regular);
                DeleteObject(small);
            }
        }
    }
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(regular);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn details_open_inward_and_leave_space_for_the_larger_transport_and_seek_bar() {
        let display = Rect::new(-1600, -1000, 0, 0);
        let strip = Rect::new(-48, -1000, 0, 0);
        let bar = NativeBar {
            edge: Edge::Right,
            display,
            bounds: strip,
        };
        let compact = Rect::new(-42, -440, -6, -296);
        let panel = detail_bounds(bar, compact, 96).unwrap();
        assert!(panel.right < compact.left);
        assert_eq!(panel.intersection(display), Some(panel));
        let layout = Layout::details(panel.width(), panel.height(), 96);
        assert_eq!(layout.buttons[1].height(), 44);
        assert!(layout.scrub.top > layout.buttons[1].bottom);
        assert!(layout.scrub.width() > 200);
    }
    use super::*;

    #[test]
    fn the_player_paints_artwork_and_progress_into_a_windows_bitmap() {
        let mut ui = Ui::new(std::rc::Rc::new(|_, _| {}));
        ui.track = Some(Arc::new(Track {
            generation: 1,
            details: Arc::new(super::super::Metadata {
                art: Some(Art {
                    width: 1,
                    height: 1,
                    pixels: vec![0x21, 0x53, 0xb4, 255],
                }),
                ..Default::default()
            }),
            playing: false,
            previous: true,
            toggle: true,
            next: true,
            can_seek: true,
            timeline: Some(Timeline {
                first: 0,
                last: 100,
                position: 40,
                sampled: Instant::now(),
                rate: 0.0,
            }),
        }));
        let (artwork, played, remaining) = unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 270,
                    biHeight: -46,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let original = SelectObject(dc, bitmap);
            paint_surface(dc, 270, 46, &ui);
            let layout = Layout::new(270, 46, 96);
            let y = layout.scrub.top + layout.scrub.height() / 2;
            let pixels = (
                GetPixel(dc, 10, 12),
                GetPixel(dc, layout.scrub.left + 2, y),
                GetPixel(dc, layout.scrub.right - 2, y),
            );
            SelectObject(dc, original);
            DeleteObject(bitmap);
            DeleteDC(dc);
            pixels
        };
        assert_eq!(artwork, 0x2153b4);
        assert_eq!(played, 0xf5bb81);
        assert_eq!(remaining, 0x443934);
    }

    #[test]
    fn inline_placement_tracks_both_sides_of_a_signed_monitor() {
        let display = Rect::new(-1400, -1080, 0, 0);
        let bounds = Rect::new(-1400, -48, 0, 0);
        let bar = NativeBar {
            edge: Edge::Bottom,
            display,
            bounds,
        };
        let island = Rect::new(-1030, -48, -370, 0);
        let left = placement(bar, Some(island), MediaSide::Left, 96)
            .unwrap()
            .bounds;
        let right = placement(bar, Some(island), MediaSide::Right, 96)
            .unwrap()
            .bounds;
        assert!(left.right < island.left && right.left > island.right);
        assert_eq!(left.top, right.top);
        assert_eq!(left.intersection(display), Some(left));
        assert_eq!(right.intersection(display), Some(right));
    }

    #[test]
    fn occupied_horizontal_space_falls_back_without_covering_native_controls() {
        let display = Rect::new(0, 0, 640, 480);
        let bounds = Rect::new(0, 0, 640, 48);
        let bar = NativeBar {
            edge: Edge::Top,
            display,
            bounds,
        };
        let near_right = Rect::new(80, 0, 630, 48);
        let fallback = placement(bar, Some(near_right), MediaSide::Right, 96)
            .unwrap()
            .bounds;
        assert!(fallback.top >= bounds.bottom);
        assert!(fallback.intersection(bounds).is_none());
        assert_eq!(fallback.intersection(display), Some(fallback));
    }

    #[test]
    fn vertical_bars_keep_the_player_on_the_same_monitor_and_inward_of_the_rail() {
        let display = Rect::new(20, 0, 1020, 780);
        let bounds = Rect::new(960, 0, 1020, 780);
        let bar = NativeBar {
            edge: Edge::Right,
            display,
            bounds,
        };
        let island = Rect::new(960, 150, 1020, 610);
        let left = placement(bar, Some(island), MediaSide::Left, 120)
            .unwrap()
            .bounds;
        let right = placement(bar, Some(island), MediaSide::Right, 120)
            .unwrap()
            .bounds;
        assert!(left.top < right.top);
        assert!(right.height() > right.width());
        assert!(right.right < bounds.left);
        assert_eq!(right.intersection(display), Some(right));
    }

    #[test]
    fn a_vertical_island_uses_free_rail_space_for_its_player() {
        let display = Rect::new(0, 0, 2880, 1800);
        let bounds = Rect::new(2784, 0, 2880, 1800);
        let bar = NativeBar {
            edge: Edge::Right,
            display,
            bounds,
        };
        let island = Rect::new(2784, 378, 2880, 1422);
        let above = placement(bar, Some(island), MediaSide::Left, 192).unwrap();
        let below = placement(bar, Some(island), MediaSide::Right, 192).unwrap();
        assert_eq!(above.bounds.left, bounds.left);
        assert!(above.bounds.bottom < island.top);
        assert_eq!(below.bounds.right, bounds.right);
        assert!(below.bounds.top > island.bottom);
        let layout = Layout::new(above.bounds.width(), above.bounds.height(), 192);
        assert_eq!(
            layout.scrub_fraction(Point {
                x: layout.scrub.left,
                y: layout.scrub.top
            }),
            1.0
        );
        assert_eq!(
            layout.scrub_fraction(Point {
                x: layout.scrub.left,
                y: layout.scrub.bottom
            }),
            0.0
        );
    }
}
#[test]
fn an_attached_player_shares_the_edge_of_a_centered_group() {
    for (edge, strip, face, reserve) in [
        (
            Edge::Right,
            Rect::new(1872, 0, 1920, 1000),
            Rect::new(1872, 178, 1920, 678),
            -144,
        ),
        (
            Edge::Bottom,
            Rect::new(0, 952, 1920, 1000),
            Rect::new(670, 952, 1470, 1000),
            220,
        ),
    ] {
        let bar = NativeBar {
            edge,
            display: Rect::new(0, 0, 1920, 1000),
            bounds: strip,
        };
        let player = attached_placement(bar, face, reserve, 96).unwrap();
        assert!(player.attached);
        if edge.horizontal() {
            assert_eq!(player.bounds.right, face.left);
            assert_eq!(player.bounds.left + face.right, 1920);
        } else {
            assert_eq!(player.bounds.top, face.bottom);
            assert_eq!(face.top + player.bounds.bottom, 1000);
        }
    }
}
