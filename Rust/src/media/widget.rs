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
        Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub bounds: Rect,
    from_right: bool,
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
    let width = px(270).min(display.width() - gap * 2);
    if width < px(172) {
        return None;
    }
    let depth = px(46);
    if let Some(face) = island.and_then(|r| r.intersection(native))
        && bar.edge.horizontal()
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
                from_right: low,
            });
        }
    }
    // When no inline space is free, attach on the inward side so no native target is covered.
    let anchor = island
        .and_then(|r| r.intersection(native))
        .unwrap_or(native);
    let (x, y) = match bar.edge {
        Edge::Top | Edge::Bottom => (
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
        ),
        Edge::Left | Edge::Right => (
            if bar.edge == Edge::Left {
                native.right + gap
            } else {
                native.left - width - gap
            },
            if low {
                anchor.top
            } else {
                anchor.bottom - depth
            },
        ),
    };
    if display.height() < depth + gap * 2 {
        return None;
    }
    let x = x.clamp(display.left + gap, display.right - gap - width);
    let y = y.clamp(display.top + gap, display.bottom - gap - depth);
    let bounds = Rect::new(x, y, x + width, y + depth);
    bounds.intersection(native).is_none().then_some(Placement {
        bounds,
        from_right: low,
    })
}

#[derive(Clone, Copy)]
struct Layout {
    art: Rect,
    label: Rect,
    buttons: [Rect; 3],
    scrub: Rect,
}

impl Layout {
    fn new(w: i32, h: i32, dpi: u32) -> Self {
        let px = |n: i32| n * dpi as i32 / 96;
        let gap = px(6);
        let size = h - gap * 2;
        let button = px(24);
        let control_x = w - gap - 3 * button;
        let controls = std::array::from_fn(|i| {
            let x = control_x + i as i32 * button;
            Rect::new(x, px(1), x + button, h - px(11))
        });
        Self {
            art: Rect::new(gap, gap, gap + size, h - gap),
            label: Rect::new(size + 2 * gap, px(3), control_x - gap, h - px(12)),
            buttons: controls,
            scrub: Rect::new(size + 2 * gap, h - px(13), w - gap - px(1), h - px(1)),
        }
    }

    fn scrub_fraction(self, x: i32) -> f64 {
        (f64::from(x - self.scrub.left) / f64::from(self.scrub.width().max(1))).clamp(0.0, 1.0)
    }
}

type Controller = Box<dyn Fn(u64, Action)>;
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
}

impl Ui {
    fn new(control: Controller) -> Self {
        Self {
            track: None,
            control,
            dpi: 96,
            light: false,
            drag: None,
        }
    }
}

pub struct Widget {
    hwnd: HWND,
    ui: Option<Box<Ui>>,
    slide: Slide,
    frame: Option<(Placement, i32, u8, i32)>,
    draw_at: u64,
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
        }
    }
}

impl Widget {
    pub fn holds(&self, pointer: Point) -> bool {
        self.frame.is_some_and(|(place, span, _, _)| {
            span == place.bounds.width()
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
            let mut ui = Box::new(Ui::new(Box::new(worker.controller())));
            self.hwnd = create(&mut ui);
            if self.hwnd.is_null() {
                return false;
            }
            self.ui = Some(ui);
        }
        let opening = track.is_some();
        let fraction = self
            .slide
            .update(opening, now, if settings.animate { 185 } else { 0 });
        let width = target.bounds.width();
        let span = (f64::from(width) * fraction).round() as i32;
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
            wide("Taskbar media player").as_ptr(),
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
        if span < r.width() {
            let left = if placement.from_right {
                r.width() - span
            } else {
                0
            };
            let mask = CreateRectRgn(left, 0, left + span, r.height() + 1);
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
                let layout = client_layout(hwnd, ui.dpi);
                if let Some(track) = &ui.track {
                    if layout.scrub.contains(pos)
                        && track.can_seek
                        && let Some(timeline) = track.timeline
                    {
                        ui.drag = Some(Drag {
                            generation: track.generation,
                            timeline,
                            fraction: layout.scrub_fraction(pos.x),
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
                if let Some(drag) = &mut ui.drag {
                    drag.fraction = client_layout(hwnd, ui.dpi).scrub_fraction(pos.x);
                    unsafe { InvalidateRect(hwnd, null(), 0) };
                }
                return 0;
            }
            WM_LBUTTONUP => {
                let ui = unsafe { &mut *state };
                if let Some(drag) = ui.drag.take() {
                    let fraction = client_layout(hwnd, ui.dpi).scrub_fraction(pos.x);
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

fn client_layout(hwnd: HWND, dpi: u32) -> Layout {
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r) };
    Layout::new(r.right, r.bottom, dpi)
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
    let scale = (f64::from(r.width()) / w).min(f64::from(r.height()) / h);
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
    let (base, backdrop, main, secondary, accent) = if ui.light {
        (0xf7f5f2, 0xe3e0dc, 0x2a2320, 0x675a52, 0x995d20)
    } else {
        (0x282323, 0x443934, 0xf8f2ec, 0xc1b3ad, 0xf5bb81)
    };
    fill(dc, Rect::new(0, 0, w, h), base);
    let Some(track) = &ui.track else { return };
    let layout = Layout::new(w, h, ui.dpi);
    fill(dc, layout.art, backdrop);
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
    let regular = font(12);
    let icons = font(17);
    unsafe { SetBkMode(dc, TRANSPARENT as i32) };
    let previous = unsafe { SelectObject(dc, regular) };
    if let Some(art) = &track.details.art {
        artwork(dc, layout.art, art);
    } else {
        unsafe { SelectObject(dc, icons) };
        text(dc, "♪", layout.art, accent, DT_CENTER | DT_VCENTER);
        unsafe { SelectObject(dc, regular) };
    }
    let title = if track.details.title.is_empty() {
        "Playing media"
    } else {
        &track.details.title
    };
    if track.details.artist.is_empty() || layout.label.height() < 25 * ui.dpi as i32 / 96 {
        text(dc, title, layout.label, main, DT_VCENTER);
    } else {
        let mut first = layout.label;
        first.bottom = first.top + first.height() / 2 + 1;
        let mut second = layout.label;
        second.top = first.bottom;
        text(dc, title, first, main, DT_VCENTER);
        text(dc, &track.details.artist, second, secondary, DT_VCENTER);
    }
    unsafe { SelectObject(dc, icons) };
    for ((rect, enabled), glyph) in layout
        .buttons
        .into_iter()
        .zip([track.previous, track.toggle, track.next])
        .zip(["⏮", if track.playing { "Ⅱ" } else { "▶" }, "⏭"])
    {
        text(
            dc,
            glyph,
            rect,
            if enabled { main } else { secondary },
            DT_CENTER | DT_VCENTER,
        );
    }
    let y = layout.scrub.top + layout.scrub.height() / 2;
    let weight = (2 * ui.dpi as i32 / 96).max(1);
    let rail = Rect::new(layout.scrub.left, y, layout.scrub.right, y + weight);
    fill(dc, rail, backdrop);
    if let Some(timeline) = track.timeline {
        let fraction = ui
            .drag
            .as_ref()
            .map_or_else(|| timeline.fraction(Instant::now()), |d| d.fraction);
        let x = rail.left + (fraction * f64::from(rail.width())).round() as i32;
        fill(
            dc,
            Rect::new(rail.left, rail.top, x, rail.bottom),
            if track.can_seek { accent } else { secondary },
        );
        if track.can_seek {
            fill(
                dc,
                Rect::new(x - weight, y - weight, x + weight + 1, y + 2 * weight),
                accent,
            );
        }
    }
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(regular);
        DeleteObject(icons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_paints_artwork_and_progress_into_a_windows_bitmap() {
        let mut ui = Ui::new(Box::new(|_, _| {}));
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
        assert!(right.right < bounds.left);
        assert_eq!(right.intersection(display), Some(right));
    }
}
