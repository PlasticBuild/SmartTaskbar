use super::native::{self, Bar};
use crate::policy::{Edge, Rect};
use std::ptr::null_mut;
use std::sync::atomic::Ordering;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

const OWNER: *const u16 = windows_sys::w!("SmartTaskbarRust.31a9.RegionOwner");
const CHANGES: *const u16 = windows_sys::w!("SmartTaskbarRust.31a9.Changes");
const ISLAND_SPAN: *const u16 = windows_sys::w!("SmartTaskbarRust.31a9.IslandSpan");
const OUTLINE: usize = 1;
const WORK: usize = 2;
const Z_ORDER: usize = 4;
const STYLER_OUTLINE: usize = 8;

pub(super) struct Region(HRGN);
impl Drop for Region {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}
impl Region {
    fn rect(r: Rect) -> Result<Self, String> {
        let h = unsafe { CreateRectRgn(r.left, r.top, r.right, r.bottom) };
        if h.is_null() {
            Err("Windows could not allocate the taskbar outline.".into())
        } else {
            Ok(Self(h))
        }
    }
    fn duplicate(&self) -> Result<Self, String> {
        let new = Self::rect(Rect::default())?;
        if unsafe { CombineRgn(new.0, self.0, null_mut(), RGN_COPY) } == ERROR {
            return Err("Could not copy the original taskbar outline.".into());
        }
        Ok(new)
    }
    fn bounds(&self) -> Option<Rect> {
        let mut r = RECT::default();
        let kind = unsafe { GetRgnBox(self.0, &mut r) };
        (kind > NULLREGION).then(|| native::rect(r))
    }
    fn same_shape(&self, other: &Self) -> bool {
        unsafe { EqualRgn(self.0, other.0) != 0 }
    }
    fn read(h: HWND) -> Result<Option<Self>, String> {
        let r = Self::rect(Rect::default())?;
        Ok((unsafe { GetWindowRgn(h, r.0) } != ERROR).then_some(r))
    }
    fn apply(h: HWND, r: Option<Self>) -> Result<(), String> {
        let raw = r.as_ref().map_or(null_mut(), |r| r.0);
        if unsafe { SetWindowRgn(h, raw, 1) } == 0 {
            return Err("Explorer rejected the taskbar outline update.".into());
        }
        // Win32 takes ownership of a successful SetWindowRgn argument.
        if let Some(r) = r {
            std::mem::forget(r)
        }
        Ok(())
    }
}

// Each record is captured before the first mutation, in both the engine and its crash monitor.
pub(super) struct Snapshot {
    pub bar: Bar,
    baseline: Option<Region>,
    temporary_clip: Option<Region>,
    pub compact: Option<Rect>,
    work_target: Option<Rect>,
    was_topmost: bool,
    owner: u32,
}
impl Snapshot {
    pub fn capture(bar: Bar, owner: u32) -> Result<Self, String> {
        let baseline = Region::read(bar.hwnd)?;
        if baseline.as_ref().is_some_and(|r| r.bounds().is_none()) {
            return Err("Waiting for Explorer to finish showing its taskbar.".into());
        }
        let compact = baseline
            .as_ref()
            .and_then(Region::bounds)
            .and_then(|r| compact_bounds(bar, r));
        let work_target = native::expanded_work(bar.strip, bar.display.work);
        let was_topmost =
            unsafe { GetWindowLongW(bar.hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST != 0;
        Ok(Self {
            bar,
            baseline,
            temporary_clip: None,
            compact,
            work_target,
            was_topmost,
            owner,
        })
    }
    pub fn displace_work(&self) -> bool {
        if let Some(target) = self.work_target
            && native::display_for(self.bar.hwnd).is_some_and(|d| d.work == self.bar.display.work)
            && self.owns()
            && self.mark(WORK, true)
        {
            return self.change_work(self.bar.display.work, target);
        }
        false
    }
    fn change_work(&self, before: Rect, after: Rect) -> bool {
        if !native::set_work(after)
            || !native::display_for(self.bar.hwnd).is_some_and(|d| d.work == after)
        {
            return false;
        }
        native::reflow_maximized(self.bar.display.bounds, before, after);
        true
    }
    pub fn owns(&self) -> bool {
        native::is_bar(self.bar.hwnd, self.bar.pid)
            && unsafe { GetPropW(self.bar.hwnd, OWNER) } as usize == self.owner as usize
    }
    fn mark(&self, part: usize, changed: bool) -> bool {
        let old = unsafe { GetPropW(self.bar.hwnd, CHANGES) } as usize;
        let value = if changed { old | part } else { old & !part };
        if value == old {
            return true;
        }
        if value == 0 {
            unsafe { RemovePropW(self.bar.hwnd, CHANGES) };
            return true;
        }
        unsafe { SetPropW(self.bar.hwnd, CHANGES, value as HANDLE) != 0 }
    }
    pub fn claim(&self) -> Result<(), String> {
        let existing = unsafe { GetPropW(self.bar.hwnd, OWNER) } as usize;
        if existing != 0 && existing != self.owner as usize {
            return Err("Another Rust taskbar controller owns this taskbar.".into());
        }
        if unsafe { SetPropW(self.bar.hwnd, OWNER, self.owner as usize as HANDLE) } == 0 {
            return Err("Explorer refused a recoverable taskbar session.".into());
        }
        self.publish_island_span();
        Ok(())
    }
    fn baseline_copy(&self) -> Result<Option<Region>, String> {
        self.baseline.as_ref().map(Region::duplicate).transpose()
    }
    pub fn restore(&self) {
        if !self.owns() {
            return;
        }
        let changed = unsafe { GetPropW(self.bar.hwnd, CHANGES) } as usize;
        let current = native::bar_info(self.bar.hwnd, self.bar.pid);
        if changed & OUTLINE != 0
            && self.temporary_clip.as_ref().is_none_or(|clip| {
                Region::read(self.bar.hwnd).map_or(true, |current| {
                    current.is_some_and(|region| clip.same_shape(&region))
                })
            })
        {
            // Windhawk rebuilds styled islands from the current shell layout.
            let original = if changed & STYLER_OUTLINE == 0
                && current.is_some_and(|b| b.strip == self.bar.strip)
            {
                self.baseline_copy().ok().flatten()
            } else {
                None
            };
            if Region::apply(self.bar.hwnd, original).is_err() {
                let _ = Region::apply(self.bar.hwnd, None);
            }
        }
        if changed & WORK != 0
            && let Some(current) = current
            && current.display.bounds == self.bar.display.bounds
            && self.work_target == Some(current.display.work)
            && !native::tablet()
            && !native::explorer_auto_hide()
        {
            let target = if current.strip == self.bar.strip {
                Some(self.bar.display.work)
            } else {
                native::reserve_after_move(current.strip, current.display.work)
            };
            if let Some(target) = target {
                self.change_work(current.display.work, target);
            }
        }
        if changed & Z_ORDER != 0 && !self.was_topmost {
            self.raise(false);
        }
        unsafe {
            RemovePropW(self.bar.hwnd, ISLAND_SPAN);
            RemovePropW(self.bar.hwnd, CHANGES);
            RemovePropW(self.bar.hwnd, OWNER)
        };
    }
    pub fn refresh_outline(&mut self) -> Result<(), String> {
        let baseline = Region::read(self.bar.hwnd)?;
        if baseline.as_ref().is_some_and(|r| r.bounds().is_none()) {
            return Err("The new shell outline is empty.".into());
        }
        self.compact = baseline
            .as_ref()
            .and_then(Region::bounds)
            .and_then(|r| compact_bounds(self.bar, r));
        self.baseline = baseline;
        self.publish_island_span();
        Ok(())
    }
    fn publish_island_span(&self) {
        // Settings runs in a separate process and cannot read the island from a fully clipped bar.
        let strip = self.bar.strip;
        let span = self.compact.and_then(|face| {
            let (start, end) = if strip.edge.horizontal() {
                (
                    face.left - strip.bounds.left,
                    face.right - strip.bounds.left,
                )
            } else {
                (face.top - strip.bounds.top, face.bottom - strip.bounds.top)
            };
            Some(
                (usize::from(u16::try_from(end).ok()?) << 16)
                    | usize::from(u16::try_from(start).ok()?),
            )
        });
        unsafe {
            if let Some(span) = span {
                SetPropW(self.bar.hwnd, ISLAND_SPAN, span as HANDLE);
            } else {
                RemovePropW(self.bar.hwnd, ISLAND_SPAN);
            }
        }
    }
    pub fn release_stale_compact_outline(&self) -> Result<bool, String> {
        // Windhawk rebuilds the outline once the window has no foreign clip. Only
        // remove the old styled shape if it is still the one on the visible bar.
        if self.compact.is_some()
            && self.owns()
            && let Some(old) = &self.baseline
            && let Some(current) = Region::read(self.bar.hwnd)?
            && old.same_shape(&current)
        {
            Region::apply(self.bar.hwnd, None)?;
            self.mark(OUTLINE | STYLER_OUTLINE, false);
            return Ok(true);
        }
        Ok(false)
    }
    pub fn center_unplaced_outline(&self) -> Result<(), String> {
        let bar = self.bar;
        if !bar.strip.edge.horizontal() || !self.owns() {
            return Ok(());
        }
        let Some(region) = Region::read(bar.hwnd)? else {
            return Ok(());
        };
        let Some(local) = region.bounds() else {
            return Ok(());
        };
        let offset = centered_outline_offset(local, bar.strip.bounds.width(), bar.dpi);
        if offset != 0 && compact_bounds(bar, local).is_some() {
            if unsafe { OffsetRgn(region.0, offset, 0) } == ERROR {
                return Err("Could not center the floating taskbar outline.".into());
            }
            if self.mark(OUTLINE, true) && self.mark(STYLER_OUTLINE, true) {
                Region::apply(bar.hwnd, Some(region))?;
            }
        }
        Ok(())
    }
    pub fn raise(&self, on: bool) {
        if !self.owns() {
            return;
        }
        if on && !self.was_topmost && !self.mark(Z_ORDER, true) {
            return;
        }
        let order = if on {
            HWND_TOPMOST
        } else if self.was_topmost {
            return;
        } else {
            HWND_NOTOPMOST
        };
        let ok = unsafe {
            SetWindowPos(
                self.bar.hwnd,
                order,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
        if ok != 0 && !on {
            self.mark(Z_ORDER, false);
        }
    }
    pub fn apply(&mut self, depth: i32, full: i32) -> Result<(), String> {
        if !self.owns() {
            return Err("Explorer replaced the taskbar while its outline was changing.".into());
        }
        let strip = self.bar.strip.bounds;
        let previous = unsafe { GetPropW(self.bar.hwnd, CHANGES) } as usize;
        let use_styler =
            super::ISLAND_STYLER_ACTIVE.load(Ordering::Relaxed) || previous & STYLER_OUTLINE != 0;
        if depth == full {
            let current = Region::read(self.bar.hwnd)?;
            if current.as_ref().is_some_and(|region| {
                region.bounds().is_none()
                    || self
                        .temporary_clip
                        .as_ref()
                        .is_some_and(|clip| clip.same_shape(region))
            }) {
                // Clearing our temporary clip triggers Windhawk's window-position handler
                // to rebuild the island around the current task buttons and system tray.
                let visible = if use_styler {
                    None
                } else {
                    self.baseline_copy()?
                };
                Region::apply(self.bar.hwnd, visible)?;
            }
            self.temporary_clip = None;
            self.mark(OUTLINE | STYLER_OUTLINE, false);
            super::SHELL_LAYOUT_CHANGED.store(true, Ordering::Relaxed);
            return Ok(());
        }
        let clip = Region::rect(edge_clip(
            self.bar.strip.edge,
            strip.width(),
            strip.height(),
            depth,
        ))?;
        if let Some(original) = self.baseline.as_ref()
            && unsafe { CombineRgn(clip.0, clip.0, original.0, RGN_AND) } == ERROR
        {
            return Err("Could not preserve the native taskbar shape.".into());
        }
        let temporary_clip = clip.duplicate()?;
        if !self.mark(OUTLINE, true) || !self.mark(STYLER_OUTLINE, use_styler) {
            return Err("Could not record taskbar outline recovery.".into());
        }
        Region::apply(self.bar.hwnd, Some(clip))?;
        self.temporary_clip = Some(temporary_clip);
        Ok(())
    }
}

fn centered_outline_offset(local: Rect, strip_width: i32, dpi: u32) -> i32 {
    // On affected Explorer layouts the visual tree centers the island, but the
    // styler returns the same compact region in coordinates starting at zero.
    // Leave positioned regions and nearly full-width islands alone.
    let offset = (strip_width - local.width()) / 2;
    if local.left == 0 && native::px_to_dip(offset, dpi) >= 12 {
        offset
    } else {
        0
    }
}

pub(super) fn current_compact(bar: Bar) -> Option<Rect> {
    compact_bounds(bar, Region::read(bar.hwnd).ok()??.bounds()?)
}
pub(super) fn preview_compact(bar: Bar) -> Option<Rect> {
    let span = unsafe { GetPropW(bar.hwnd, ISLAND_SPAN) } as usize;
    let start = (span & 0xffff) as i32;
    let end = (span >> 16) as i32;
    let r = bar.strip.bounds;
    let cached = if bar.strip.edge.horizontal() {
        Rect::new(r.left + start, r.top, r.left + end, r.bottom)
    } else {
        Rect::new(r.left, r.top + start, r.right, r.top + end)
    };
    current_compact(bar).or_else(|| (r.intersection(cached) == Some(cached)).then_some(cached))
}
pub(super) fn region_empty(bar: Bar) -> bool {
    Region::read(bar.hwnd)
        .ok()
        .flatten()
        .is_some_and(|r| r.bounds().is_none())
}

fn compact_bounds(bar: Bar, local: Rect) -> Option<Rect> {
    let r = bar.strip.bounds;
    let face = Rect::new(
        r.left + local.left,
        r.top + local.top,
        r.left + local.right,
        r.top + local.bottom,
    )
    .intersection(r)?;
    let (span, outer, cross, thick) = if bar.strip.edge.horizontal() {
        (face.width(), r.width(), face.height(), r.height())
    } else {
        (face.height(), r.height(), face.width(), r.width())
    };
    (span >= thick && span + 4 < outer && cross * 2 >= thick).then_some(face)
}

fn edge_clip(edge: Edge, w: i32, h: i32, depth: i32) -> Rect {
    match edge {
        Edge::Left => Rect::new(0, 0, depth, h),
        Edge::Right => Rect::new(w - depth, 0, w, h),
        Edge::Top => Rect::new(0, 0, w, depth),
        Edge::Bottom => Rect::new(0, h - depth, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipping_never_spills_onto_the_adjacent_monitor() {
        assert_eq!(
            edge_clip(Edge::Bottom, 1920, 48, 13),
            Rect::new(0, 35, 1920, 48)
        );
        assert_eq!(
            edge_clip(Edge::Top, 1920, 48, 13),
            Rect::new(0, 0, 1920, 13)
        );
        assert!(edge_clip(Edge::Right, 48, 1080, 0).is_empty());
    }

    #[test]
    fn rebases_only_a_leading_edge_outline_with_room_to_center() {
        assert_eq!(
            centered_outline_offset(Rect::new(0, 0, 1339, 96), 2880, 192),
            770
        );
        assert_eq!(
            centered_outline_offset(Rect::new(902, 0, 1977, 96), 2880, 192),
            0
        );
        assert_eq!(
            centered_outline_offset(Rect::new(0, 0, 2860, 96), 2880, 192),
            0
        );
    }
}
