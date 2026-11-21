use super::native::{self, Bar};
use crate::policy::{Edge, Rect};
use std::ptr::null_mut;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

const OWNER: *const u16 = windows_sys::w!("SmartTaskbarRust.31a9.RegionOwner");
const CHANGES: *const u16 = windows_sys::w!("SmartTaskbarRust.31a9.Changes");
const OUTLINE: usize = 1;
const WORK: usize = 2;
const Z_ORDER: usize = 4;

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
        // A changed monitor or shell layout owns its new dimensions.
        if current.is_some_and(|b| b.strip == self.bar.strip) {
            if changed & OUTLINE != 0 {
                let copy = self.baseline_copy().ok().flatten();
                if Region::apply(self.bar.hwnd, copy).is_err() {
                    let _ = Region::apply(self.bar.hwnd, None);
                }
            }
        } else if changed & OUTLINE != 0 {
            let _ = Region::apply(self.bar.hwnd, None);
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
        Ok(())
    }
    pub fn release_stale_compact_outline(&self) -> Result<bool, String> {
        // Windhawk rebuilds the outline once the window has no foreign clip. Only
        // remove the old styled shape if it is still the one on the visible bar.
        if self.compact.is_some()
            && self.owns()
            && let Some(old) = &self.baseline
            && let Some(current) = Region::read(self.bar.hwnd)?
            && unsafe { EqualRgn(old.0, current.0) } != 0
        {
            Region::apply(self.bar.hwnd, None)?;
            self.mark(OUTLINE, false);
            return Ok(true);
        }
        Ok(false)
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
    pub fn apply(&self, depth: i32, full: i32) -> Result<(), String> {
        if !self.owns() {
            return Err("Explorer replaced the taskbar while its outline was changing.".into());
        }
        let strip = self.bar.strip.bounds;
        if depth == full {
            Region::apply(self.bar.hwnd, self.baseline_copy()?)?;
            self.mark(OUTLINE, false);
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
        if !self.mark(OUTLINE, true) {
            return Err("Could not record taskbar outline recovery.".into());
        }
        Region::apply(self.bar.hwnd, Some(clip))
    }
}

pub(super) fn current_compact(bar: Bar) -> Option<Rect> {
    compact_bounds(bar, Region::read(bar.hwnd).ok()??.bounds()?)
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
}
