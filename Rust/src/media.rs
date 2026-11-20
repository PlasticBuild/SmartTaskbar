//! The media worker owns WinRT subscriptions. The desktop thread only sees snapshots.
mod session;
mod widget;

pub use session::Worker;
pub use widget::{Widget, attached_placement, placement};

pub const HORIZONTAL_DIPS: i32 = 220;
pub const VERTICAL_DIPS: i32 = 144;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reserve {
    pub horizontal: i32,
    pub vertical: i32,
}

impl Reserve {
    pub fn along(self, horizontal: bool) -> i32 {
        if horizontal {
            self.horizontal
        } else {
            self.vertical
        }
    }
}

use std::{sync::Arc, time::Instant};

#[derive(Clone, Copy)]
pub enum Action {
    Previous,
    PlayPause,
    Next,
    Seek(i64),
}

pub struct Art {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Default)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub art: Option<Art>,
}

#[derive(Clone, Copy)]
pub struct Timeline {
    pub first: i64,
    pub last: i64,
    position: i64,
    sampled: Instant,
    rate: f64,
}

impl Timeline {
    fn position(self, now: Instant) -> i64 {
        let elapsed = now.saturating_duration_since(self.sampled).as_secs_f64();
        self.position
            .saturating_add((elapsed * self.rate * 10_000_000.0) as i64)
            .clamp(self.first, self.last)
    }

    fn fraction(self, now: Instant) -> f64 {
        (self.position(now) - self.first) as f64 / (self.last - self.first) as f64
    }

    fn at(self, fraction: f64) -> i64 {
        self.first + ((self.last - self.first) as f64 * fraction.clamp(0.0, 1.0)).round() as i64
    }
}

pub struct Track {
    pub generation: u64,
    pub details: Arc<Metadata>,
    pub playing: bool,
    pub previous: bool,
    pub toggle: bool,
    pub next: bool,
    pub can_seek: bool,
    pub timeline: Option<Timeline>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn live_position_advances_and_clamps_to_the_seek_range() {
        let sampled = Instant::now();
        let timeline = Timeline {
            first: 10_000_000,
            last: 40_000_000,
            position: 25_000_000,
            sampled,
            rate: 1.5,
        };
        assert_eq!(
            timeline.position(sampled + Duration::from_millis(500)),
            32_500_000
        );
        assert_eq!(
            timeline.position(sampled + Duration::from_secs(5)),
            40_000_000
        );
        assert_eq!(timeline.at(-0.5), 10_000_000);
        assert_eq!(timeline.at(0.5), 25_000_000);
    }
}
