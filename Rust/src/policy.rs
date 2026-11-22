//! Desktop-independent taskbar policy. Rectangles are half-open, in physical virtual-screen pixels.
use crate::settings::{Alignment, Reveal, Zone};

pub const START_MIN_DIP: i32 = 420;
pub const START_MAX_DIP: i32 = 760;
pub const SIDE_INSET_DIP: i32 = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(self) -> i32 {
        self.right.saturating_sub(self.left).max(0)
    }
    pub fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top).max(0)
    }
    pub fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    pub fn contains(self, point: Point) -> bool {
        self.left <= point.x && point.x < self.right && self.top <= point.y && point.y < self.bottom
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let result = Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!result.is_empty()).then_some(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

impl Edge {
    pub fn horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    fn span(self, rect: Rect) -> (i32, i32) {
        if self.horizontal() {
            (rect.left, rect.right)
        } else {
            (rect.top, rect.bottom)
        }
    }

    fn with_span(self, rect: Rect, start: i32, end: i32) -> Rect {
        if self.horizontal() {
            Rect::new(start, rect.top, end, rect.bottom)
        } else {
            Rect::new(rect.left, start, rect.right, end)
        }
    }

    fn outside(self, display: Rect, p: Point) -> Point {
        match self {
            Self::Left => Point {
                x: display.left.saturating_sub(1),
                ..p
            },
            Self::Top => Point {
                y: display.top.saturating_sub(1),
                ..p
            },
            Self::Right => Point {
                x: display.right,
                ..p
            },
            Self::Bottom => Point {
                y: display.bottom,
                ..p
            },
        }
    }

    fn approaching(self, previous: Point, point: Point) -> bool {
        match self {
            Self::Left => point.x < previous.x,
            Self::Top => point.y < previous.y,
            Self::Right => point.x > previous.x,
            Self::Bottom => point.y > previous.y,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeBar {
    pub display: Rect,
    /// Explorer's unhidden strip on this display (may span the entire edge).
    pub bounds: Rect,
    pub edge: Edge,
}

impl NativeBar {
    pub fn from_bounds(display: Rect, bounds: Rect) -> Option<Self> {
        let bounds = display.intersection(bounds)?;
        let edge = if bounds.width() > bounds.height() {
            if bounds.top.abs_diff(display.top) < bounds.bottom.abs_diff(display.bottom) {
                Edge::Top
            } else {
                Edge::Bottom
            }
        } else if bounds.left.abs_diff(display.left) < bounds.right.abs_diff(display.right) {
            Edge::Left
        } else {
            Edge::Right
        };
        Some(Self {
            display,
            bounds,
            edge,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSample {
    pub bounds: Rect,
    /// Monitor owning this window, including a maximized window that ends at the work area.
    pub home: Rect,
    pub maximized: bool,
}

/// Supply visible application windows, excluding the desktop, the shell's own flyouts and cloaked windows.
/// A window remains a cover on a different monitor when focus moves away from it.
pub fn is_covered(bar: NativeBar, island: Option<Rect>, windows: &[WindowSample]) -> bool {
    let Some(strip) = bar.display.intersection(bar.bounds) else {
        return false;
    };
    let face = island
        .and_then(|face| face.intersection(strip))
        .unwrap_or(strip);
    windows.iter().any(|window| {
        window.bounds.intersection(face).is_some()
            || (window.maximized
                && window.home == bar.display
                && window.bounds.intersection(bar.display).is_some())
    })
}

fn pixels(dip: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((i64::from(dip) * i64::from(dpi) + 48) / 96).min(i64::from(i32::MAX)) as i32
}

pub fn activation_band(bar: NativeBar, island: Option<Rect>, zone: Zone, dpi: u32) -> Rect {
    let (start, end) = bar.edge.span(bar.display);
    let (start, end) = match zone {
        Zone::Entire => (start, end),
        Zone::Center => {
            let quarter = end.saturating_sub(start) / 4;
            (start + quarter, end - quarter)
        }
        Zone::Island => bar.edge.span(
            island
                .and_then(|rect| rect.intersection(bar.bounds))
                .unwrap_or(bar.bounds),
        ),
    };
    let mut band = bar.edge.with_span(bar.display, start, end);
    let depth = pixels(6, dpi).max(1);
    match bar.edge {
        Edge::Left => band.right = band.left.saturating_add(depth).min(band.right),
        Edge::Right => band.left = band.right.saturating_sub(depth).max(band.left),
        Edge::Top => band.bottom = band.top.saturating_add(depth).min(band.bottom),
        Edge::Bottom => band.top = band.bottom.saturating_sub(depth).max(band.top),
    }
    band
}

#[derive(Default)]
pub struct HoverState {
    previous: Option<Point>,
    entered_at: Option<u64>,
    peek_until: Option<u64>,
    display_edge: Option<(Rect, Edge)>,
    reveal: Option<Reveal>,
}

#[derive(Clone, Copy)]
pub struct BarInput {
    pub bar: NativeBar,
    pub island: Option<Rect>,
    pub dpi: u32,
}

#[derive(Clone, Copy)]
pub struct PointerFrame {
    pub cursor: Point,
    pub now_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Clear,
    Overlap,
    ForegroundFullscreen,
}

impl HoverState {
    /// One instance per taskbar. The hot band always lies on its own side of a monitor seam.
    pub fn update(
        &mut self,
        surface: BarInput,
        reveal: Reveal,
        displays: &[Rect],
        frame: PointerFrame,
        cover: Cover,
        menu_open: bool,
    ) -> bool {
        let BarInput { bar, island, dpi } = surface;
        let PointerFrame { cursor, now_ms } = frame;
        if self.display_edge != Some((bar.display, bar.edge)) || self.reveal != Some(reveal) {
            *self = Self {
                display_edge: Some((bar.display, bar.edge)),
                reveal: Some(reveal),
                ..Self::default()
            };
        }
        let previous = self.previous.replace(cursor);
        if cover != Cover::Overlap {
            self.entered_at = None;
            self.peek_until = None;
            return cover == Cover::Clear;
        }
        let grace = now_ms.saturating_add(320);
        if menu_open {
            self.entered_at = None;
            self.peek_until = Some(grace);
            return true;
        }

        let in_band = activation_band(bar, island, reveal.zone, dpi).contains(cursor);
        let face = island
            .and_then(|rect| rect.intersection(bar.bounds))
            .unwrap_or(bar.bounds);
        if let Some(until) = self.peek_until {
            if face.contains(cursor) || in_band {
                self.peek_until = Some(grace);
                return true;
            }
            if now_ms < until {
                return true;
            }
            self.peek_until = None;
        }

        if !in_band {
            self.entered_at = None;
            return false;
        }
        let across = bar.edge.outside(bar.display, cursor);
        let shared = displays
            .iter()
            .any(|other| *other != bar.display && other.contains(across));
        if let Some(previous) = previous {
            // A crossing or a reversal near a seam starts a fresh gesture on that screen.
            if shared && (!bar.display.contains(previous) || bar.edge.approaching(cursor, previous))
            {
                self.entered_at = None;
            }
        }
        if self.entered_at.is_none()
            && (!shared
                || previous
                    .is_some_and(|p| bar.display.contains(p) && bar.edge.approaching(p, cursor)))
        {
            self.entered_at = Some(now_ms);
        }
        let dwell = u64::from(if shared {
            reveal.shared_ms
        } else {
            reveal.outside_ms
        });
        if self
            .entered_at
            .is_some_and(|entered| now_ms.saturating_sub(entered) >= dwell)
        {
            self.entered_at = None;
            self.peek_until = Some(grace);
            return true;
        }
        false
    }
}

/// Interrupted transitions resume from the currently drawn position.
#[derive(Debug)]
pub struct Slide {
    from: f64,
    to: f64,
    started: u64,
    duration: u64,
}

impl Default for Slide {
    fn default() -> Self {
        Self {
            from: 1.0,
            to: 1.0,
            started: 0,
            duration: 0,
        }
    }
}

impl Slide {
    fn at(&self, now_ms: u64) -> f64 {
        if self.duration == 0 {
            return self.to;
        }
        let t = (now_ms.saturating_sub(self.started) as f64 / self.duration as f64).min(1.0);
        self.from + (self.to - self.from) * t * t * (3.0 - 2.0 * t)
    }

    pub fn update(&mut self, visible: bool, now_ms: u64, duration_ms: u64) -> f64 {
        let target = if visible { 1.0 } else { 0.0 };
        let current = self.at(now_ms);
        if target != self.to || duration_ms == 0 {
            self.from = current;
            self.to = target;
            self.started = now_ms;
            self.duration = ((target - current).abs() * duration_ms as f64).round() as u64;
        }
        self.at(now_ms)
    }
}

pub fn slide_offset(edge: Edge, thickness: i32, fraction: f64) -> Point {
    let offset = (f64::from(thickness.max(0)) * (1.0 - fraction.clamp(0.0, 1.0))).round() as i32;
    match edge {
        Edge::Left => Point { x: -offset, y: 0 },
        Edge::Right => Point { x: offset, y: 0 },
        Edge::Top => Point { x: 0, y: -offset },
        Edge::Bottom => Point { x: 0, y: offset },
    }
}

pub fn island_rect(bar: NativeBar, content_length: i32, align: Alignment, dpi: u32) -> Rect {
    let Some(strip) = bar.display.intersection(bar.bounds) else {
        return Rect::default();
    };
    let (start, end) = bar.edge.span(strip);
    let length = end.saturating_sub(start);
    let padding = pixels(8, dpi).saturating_mul(2);
    let needed = content_length.max(1).saturating_add(padding).min(length);
    let slack = length - needed;
    let side_gap = pixels(SIDE_INSET_DIP, dpi).min(slack / 2);
    let offset = match align {
        Alignment::Left => side_gap,
        Alignment::Right => slack - side_gap,
        Alignment::Center => slack / 2,
    };
    let face = bar
        .edge
        .with_span(strip, start + offset, start + offset + needed);
    let thickness = if bar.edge.horizontal() {
        strip.height()
    } else {
        strip.width()
    };
    let gap = pixels(2, dpi).min(thickness.saturating_sub(1) / 2);
    if bar.edge.horizontal() {
        Rect::new(face.left, face.top + gap, face.right, face.bottom - gap)
    } else {
        Rect::new(face.left + gap, face.top, face.right - gap, face.bottom)
    }
}

/// Start shares the island's long axis within its readable width limits. Its near edge touches the island.
/// At a side dock the same construction is rotated so the panel opens into the display.
pub fn anchored_start(bar: NativeBar, island: Rect, preferred_depth: i32, dpi: u32) -> Rect {
    let Some(island) = island.intersection(bar.display) else {
        return Rect::default();
    };
    let (low, high) = bar.edge.span(bar.display);
    let (near, far) = bar.edge.span(island);
    let span = far.saturating_sub(near);
    let size = span
        .clamp(pixels(START_MIN_DIP, dpi), pixels(START_MAX_DIP, dpi))
        .min(high.saturating_sub(low));
    let before = i64::from(near) - i64::from(low);
    let after = i64::from(high) - i64::from(far);
    let side_gap = i64::from(pixels(SIDE_INSET_DIP, dpi));
    let preferred = if before <= side_gap && before < after {
        i64::from(near)
    } else if after <= side_gap && after < before {
        i64::from(far) - i64::from(size)
    } else {
        i64::from(near) + (i64::from(span) - i64::from(size)).div_euclid(2)
    };
    let start = preferred.clamp(i64::from(low), i64::from(high) - i64::from(size)) as i32;
    let depth = preferred_depth.max(pixels(160, dpi));
    let d = bar.display;
    let panel = match bar.edge {
        Edge::Left => Rect::new(
            island.right,
            d.top,
            island.right.saturating_add(depth).min(d.right),
            d.bottom,
        ),
        Edge::Right => Rect::new(
            island.left.saturating_sub(depth).max(d.left),
            d.top,
            island.left,
            d.bottom,
        ),
        Edge::Top => Rect::new(
            d.left,
            island.bottom,
            d.right,
            island.bottom.saturating_add(depth).min(d.bottom),
        ),
        Edge::Bottom => Rect::new(
            d.left,
            island.top.saturating_sub(depth).max(d.top),
            d.right,
            island.top,
        ),
    };
    bar.edge.with_span(panel, start, start + size)
}

/// Match the bar outline to an attached Start panel, even when the panel reaches its min/max.
pub fn joined_start_face(edge: Edge, resting: Rect, panel: Rect) -> Rect {
    if panel.is_empty() {
        return resting;
    }
    let (start, end) = edge.span(panel);
    edge.with_span(resting, start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPPER: Rect = Rect::new(-600, -1080, 1320, 0);
    const LOWER: Rect = Rect::new(0, 0, 1920, 1080);

    fn dock(display: Rect, edge: Edge) -> NativeBar {
        let bounds = match edge {
            Edge::Top => Rect::new(display.left, display.top, display.right, display.top + 48),
            Edge::Bottom => Rect::new(
                display.left,
                display.bottom - 48,
                display.right,
                display.bottom,
            ),
            Edge::Left => Rect::new(display.left, display.top, display.left + 48, display.bottom),
            Edge::Right => Rect::new(
                display.right - 48,
                display.top,
                display.right,
                display.bottom,
            ),
        };
        NativeBar {
            display,
            bounds,
            edge,
        }
    }

    #[test]
    fn signed_half_open_rects_keep_monitor_seams_distinct() {
        assert_eq!(UPPER.intersection(LOWER), None);
        assert!(UPPER.contains(Point { x: 1, y: -1 }));
        assert!(!UPPER.contains(Point { x: 1, y: 0 }));
        assert!(LOWER.contains(Point { x: 1, y: 0 }));
        assert_eq!(Rect::new(4, 0, -4, 0).width(), 0);
        assert_eq!(Rect::new(i32::MIN, 0, i32::MAX, 1).width(), i32::MAX);
        for edge in [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom] {
            let bar = dock(UPPER, edge);
            assert_eq!(NativeBar::from_bounds(UPPER, bar.bounds), Some(bar));
        }
        assert_eq!(NativeBar::from_bounds(UPPER, LOWER), None);
    }

    #[test]
    fn fullscreen_stays_hidden_when_focus_moves_to_other_display() {
        let upper = dock(UPPER, Edge::Bottom);
        let lower = dock(LOWER, Edge::Bottom);
        let covered = WindowSample {
            bounds: UPPER,
            home: UPPER,
            maximized: false,
        };
        let other = WindowSample {
            bounds: Rect::new(100, 100, 880, 600),
            home: LOWER,
            maximized: false,
        };
        for windows in [[covered, other], [other, covered]] {
            assert!(is_covered(upper, None, &windows));
            assert!(!is_covered(lower, None, &windows));
        }
        assert!(!is_covered(upper, None, &[]));
    }

    #[test]
    fn maximized_window_at_work_area_hides_only_its_own_bar() {
        let upper = dock(UPPER, Edge::Right);
        let window = WindowSample {
            bounds: Rect::new(UPPER.left, UPPER.top, upper.bounds.left, UPPER.bottom),
            home: UPPER,
            maximized: true,
        };
        assert!(is_covered(upper, None, &[window]));
        assert!(!is_covered(dock(LOWER, Edge::Right), None, &[window]));
        assert!(!is_covered(
            upper,
            None,
            &[WindowSample {
                maximized: false,
                ..window
            }]
        ));
    }

    #[test]
    fn island_covers_only_its_occupied_face() {
        let bar = dock(UPPER, Edge::Bottom);
        let face = island_rect(bar, 400, Alignment::Center, 144);
        let sample = |bounds| WindowSample {
            bounds,
            maximized: false,
            home: UPPER,
        };
        let margin = Rect::new(UPPER.left, face.top, face.left, face.bottom);
        assert!(is_covered(bar, None, &[sample(margin)]));
        assert!(!is_covered(bar, Some(face), &[sample(margin)]));
        assert!(is_covered(
            bar,
            Some(face),
            &[sample(Rect::new(
                face.left - 1,
                face.top,
                face.left + 1,
                face.bottom
            ))]
        ));
    }

    #[test]
    fn activation_zones_follow_orientation_and_dpi() {
        for edge in [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom] {
            let bar = dock(UPPER, edge);
            let face = island_rect(bar, 220, Alignment::Right, 192);
            let entire = activation_band(bar, Some(face), Zone::Entire, 192);
            let half = activation_band(bar, Some(face), Zone::Center, 192);
            let island = activation_band(bar, Some(face), Zone::Island, 192);
            assert_eq!(UPPER.intersection(entire), Some(entire));
            assert_eq!(edge.span(island), edge.span(face));
            let (a, b) = edge.span(entire);
            let (c, d) = edge.span(half);
            assert_eq!(d - c, (b - a) / 2);
            assert_eq!(
                if edge.horizontal() {
                    entire.height()
                } else {
                    entire.width()
                },
                12
            );
        }
        assert_eq!(
            activation_band(dock(LOWER, Edge::Bottom), None, Zone::Entire, 0).height(),
            6
        );
    }

    #[test]
    fn shared_edge_requires_approach_from_the_requesting_display() {
        let bar = dock(UPPER, Edge::Bottom);
        let mut hover = HoverState::default();
        let input = BarInput {
            bar,
            island: None,
            dpi: 96,
        };
        let mut visible = |y, now_ms| {
            hover.update(
                input,
                Reveal::default(),
                &[UPPER, LOWER],
                PointerFrame {
                    cursor: Point { x: 12, y },
                    now_ms,
                },
                Cover::Overlap,
                false,
            )
        };
        assert!(!visible(20, 0));
        assert!(!visible(-2, 20));
        assert!(!visible(-2, 420));
        assert!(!visible(-36, 450));
        assert!(!visible(-2, 480));
        assert!(!visible(-2, 699));
        assert!(visible(-2, 700));
        assert!(visible(-42, 1600)); // over the revealed bar
        assert!(visible(-60, 1700)); // brief grace leaving the face
        assert!(!visible(-60, 2100));
    }

    #[test]
    fn crossing_a_shared_edge_cancels_dwell_but_exposed_segment_remains_usable() {
        let bar = dock(UPPER, Edge::Bottom);
        let mut hover = HoverState::default();
        let input = BarInput {
            bar,
            island: None,
            dpi: 96,
        };
        let mut visible = |x, y, now_ms| {
            hover.update(
                input,
                Reveal::default(),
                &[UPPER, LOWER],
                PointerFrame {
                    cursor: Point { x, y },
                    now_ms,
                },
                Cover::Overlap,
                false,
            )
        };
        assert!(!visible(5, -30, 0));
        assert!(!visible(5, -2, 5));
        assert!(!visible(5, 2, 80));
        assert!(!visible(5, -2, 100));
        assert!(!visible(5, -2, 500));
        assert!(!visible(-400, -2, 510));
        assert!(visible(-400, -2, 630));
    }

    #[test]
    fn expose_only_after_the_selected_delay_for_that_part_of_the_edge() {
        let input = BarInput {
            bar: dock(UPPER, Edge::Bottom),
            island: None,
            dpi: 96,
        };
        let reveal = Reveal {
            outside_ms: 0,
            shared_ms: 400,
            ..Reveal::default()
        };
        let check = |state: &mut HoverState, x, y, now_ms| {
            state.update(
                input,
                reveal,
                &[UPPER, LOWER],
                PointerFrame {
                    cursor: Point { x, y },
                    now_ms,
                },
                Cover::Overlap,
                false,
            )
        };
        let mut shared = HoverState::default();
        assert!(!check(&mut shared, 12, -40, 0));
        assert!(!check(&mut shared, 12, -2, 20));
        assert!(!check(&mut shared, 12, -2, 419));
        assert!(check(&mut shared, 12, -2, 420));
        assert!(check(&mut HoverState::default(), -400, -2, 0));
    }

    #[test]
    fn menu_pins_its_display_and_desktop_returns_to_rest_without_dwell() {
        let bar = dock(LOWER, Edge::Right);
        let p = Point { x: 100, y: 100 };
        let mut hover = HoverState::default();
        let input = BarInput {
            bar,
            island: None,
            dpi: 96,
        };
        let mut visible = |now_ms, cover, menu| {
            hover.update(
                input,
                Reveal {
                    zone: Zone::Island,
                    ..Reveal::default()
                },
                &[LOWER],
                PointerFrame { cursor: p, now_ms },
                cover,
                menu,
            )
        };
        assert!(visible(0, Cover::Overlap, true));
        assert!(visible(1000, Cover::Overlap, true));
        assert!(visible(1100, Cover::Overlap, false));
        assert!(!visible(1500, Cover::Overlap, false));
        assert!(visible(1501, Cover::Clear, false));
    }

    #[test]
    fn foreground_fullscreen_blocks_edge_reveal_and_clears_pending_hover() {
        let mut hover = HoverState::default();
        let input = BarInput {
            bar: dock(LOWER, Edge::Bottom),
            island: None,
            dpi: 96,
        };
        let reveal = Reveal {
            outside_ms: 120,
            ..Reveal::default()
        };
        let mut visible = |now_ms, cover, menu| {
            hover.update(
                input,
                reveal,
                &[UPPER, LOWER],
                PointerFrame {
                    cursor: Point {
                        x: LOWER.left + LOWER.width() / 2,
                        y: LOWER.bottom - 1,
                    },
                    now_ms,
                },
                cover,
                menu,
            )
        };
        assert!(!visible(0, Cover::Overlap, false));
        assert!(visible(120, Cover::Overlap, false));
        assert!(!visible(121, Cover::ForegroundFullscreen, true));
        assert!(!visible(1000, Cover::ForegroundFullscreen, false));
        assert!(!visible(1001, Cover::Overlap, false));
        assert!(visible(1121, Cover::Overlap, false));
    }

    #[test]
    fn animation_reversal_is_continuous_and_zero_duration_is_immediate() {
        let mut slide = Slide::default();
        assert_eq!(slide.update(false, 100, 200), 1.0);
        assert_eq!(slide.update(false, 200, 200), 0.5);
        assert_eq!(slide.update(true, 200, 200), 0.5);
        assert!(slide.update(true, 250, 200) > 0.5);
        assert_eq!(slide.update(true, 300, 200), 1.0);
        assert_eq!(slide.update(false, 310, 0), 0.0);
        assert_eq!(slide.update(true, 310, 0), 1.0);
        assert_eq!(slide_offset(Edge::Top, 48, 0.5), Point { x: 0, y: -24 });
        assert_eq!(slide_offset(Edge::Right, 48, 0.0), Point { x: 48, y: 0 });
    }

    #[test]
    fn start_attaches_at_final_size_and_never_spills_onto_another_screen() {
        for edge in [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom] {
            for align in [Alignment::Left, Alignment::Center, Alignment::Right] {
                let bar = dock(UPPER, edge);
                let face = island_rect(bar, 530, align, 96);
                let panel = anchored_start(bar, face, 610, 96);
                assert_eq!(edge.span(panel), edge.span(face));
                assert_eq!(panel.intersection(UPPER), Some(panel));
                assert_eq!(panel.intersection(face), None);
                match edge {
                    Edge::Top => assert_eq!(panel.top, face.bottom),
                    Edge::Bottom => assert_eq!(panel.bottom, face.top),
                    Edge::Left => assert_eq!(panel.left, face.right),
                    Edge::Right => assert_eq!(panel.right, face.left),
                }
            }
        }
        let bar = dock(UPPER, Edge::Bottom);
        let wide = island_rect(bar, 10000, Alignment::Center, 96);
        assert_eq!(wide.width(), UPPER.width());
        assert_eq!(anchored_start(bar, wide, 9000, 96).width(), START_MAX_DIP);
        let narrow = island_rect(bar, 40, Alignment::Left, 96);
        let menu = anchored_start(bar, narrow, 400, 96);
        assert_eq!(menu.width(), START_MIN_DIP);
        assert_eq!(menu.left, narrow.left);
        let joined = joined_start_face(Edge::Bottom, narrow, menu);
        assert_eq!(joined.width(), menu.width());
        assert_eq!(joined.top, menu.bottom);
        assert_eq!(joined.height(), narrow.height());
        let tiny = dock(Rect::new(0, -300, 300, 0), Edge::Bottom);
        let panel = anchored_start(tiny, tiny.bounds, 4000, 192);
        assert_eq!(panel.width(), 300);
        assert_eq!(panel.top, -300);
    }
}
