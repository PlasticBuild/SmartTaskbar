//! Live shell coordination. Merely linking this module never opens a monitor or changes Windows.
mod lease;
mod native;
mod rescue;

use crate::{
    join::Join,
    media::{self, Widget, Worker},
    policy::{self, Edge, HoverState, Rect, Slide},
    settings::Settings,
};
use lease::Snapshot;
use native::{Bar, Scene};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};
use windows_sys::Win32::{
    Foundation::*,
    System::Threading::*,
    UI::{Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

struct DpiScope(DPI_AWARENESS_CONTEXT);
impl DpiScope {
    fn enter() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for DpiScope {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

pub fn inspect() -> Result<String, String> {
    let _dpi = DpiScope::enter();
    let displays = native::displays();
    let bars = native::bars();
    let mut lines = vec![format!(
        "Displays: {}. Explorer taskbars: {}. Tablet posture: {}. Explorer auto-hide: {}.",
        displays.len(),
        bars.len(),
        native::tablet(),
        native::explorer_auto_hide()
    )];
    for bar in bars {
        lines.push(format!(
            "{:?} monitor {:?}, taskbar {:?}, {} DPI, visible island {:?}",
            bar.strip.edge,
            bar.strip.display,
            bar.strip.bounds,
            bar.dpi,
            lease::current_compact(bar)
        ));
    }
    if let Some((width, shift, inset)) = primary_island() {
        lines.push(format!("Primary bottom island uses a {width} DIP Start panel, a {shift} DIP flyout shift, and a {inset} DIP Search inset."));
    }
    Ok(lines.join("\n"))
}

fn layout(bar: Bar, face: Option<Rect>) -> Option<(i32, i32, i32)> {
    let face = face?;
    if !bar.display.primary || bar.strip.edge != Edge::Bottom {
        return None;
    }
    let available = native::px_to_dip(bar.display.work.width(), bar.dpi);
    let width = native::px_to_dip(
        policy::anchored_start(bar.strip, face, 160, bar.dpi).width(),
        bar.dpi,
    )
    .min(available);
    let shift = (policy::SIDE_INSET_DIP
        - native::px_to_dip(bar.display.work.right - face.right, bar.dpi))
    .clamp((384 - available).min(0), 0);
    let reclaimed =
        (bar.display.work.bottom - bar.strip.bounds.top).clamp(0, bar.strip.bounds.height());
    let search_inset = (native::px_to_dip(reclaimed, bar.dpi) - 18).max(0);
    Some((width, shift, search_inset))
}

pub fn primary_island() -> Option<(i32, i32, i32)> {
    let _dpi = DpiScope::enter();
    native::bars()
        .into_iter()
        .find_map(|bar| layout(bar, lease::current_compact(bar)))
}

pub fn recover() -> Result<(), String> {
    rescue::recover()
}
const STOP_NAME: &str = r"Local\SmartTaskbar.Rust.Stop";
const HOTKEY: i32 = 0x0371;

pub fn request_stop() -> Result<(), String> {
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, native::wide(STOP_NAME).as_ptr()) };
    if handle.is_null() {
        return Err("The Rust taskbar engine is not running in this session.".into());
    }
    let ok = unsafe { SetEvent(handle) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        Err("Windows could not signal the running taskbar engine.".into())
    } else {
        Ok(())
    }
}

static DIRTY: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn change(
    _: HWINEVENTHOOK,
    event: u32,
    h: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if !h.is_null()
        && (event == EVENT_SYSTEM_FOREGROUND
            || (object == 0 && child == 0 && unsafe { GetAncestor(h, GA_ROOT) } == h))
    {
        DIRTY.store(true, Ordering::Relaxed)
    }
}

struct Notifications {
    stop: HANDLE,
    hooks: Vec<HWINEVENTHOOK>,
}
impl Notifications {
    fn create() -> Result<Self, String> {
        let stop = unsafe { CreateEventW(null(), 1, 0, native::wide(STOP_NAME).as_ptr()) };
        if stop.is_null() {
            return Err("Could not open the taskbar stop event.".into());
        }
        if unsafe {
            RegisterHotKey(
                null_mut(),
                HOTKEY,
                MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                u32::from(b'T'),
            )
        } == 0
        {
            unsafe { CloseHandle(stop) };
            return Err("Ctrl+Alt+Shift+T is in use. The taskbar engine could not register its restore shortcut.".into());
        }
        let mut hooks = Vec::new();
        for (from, to) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
        ] {
            let hook = unsafe {
                SetWinEventHook(
                    from,
                    to,
                    null_mut(),
                    Some(change),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if !hook.is_null() {
                hooks.push(hook)
            }
        }
        Ok(Self { stop, hooks })
    }
    fn wait(&self, ms: u32) -> Result<bool, String> {
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(1, &self.stop, ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if result == WAIT_FAILED {
            return Err("Windows could not wait for taskbar input.".into());
        }
        if result == WAIT_OBJECT_0 {
            return Ok(false);
        }
        let mut msg = MSG::default();
        // Bound each batch so a noisy compositor cannot starve the next animation step.
        for _ in 0..128 {
            if unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } == 0 {
                break;
            }
            if msg.message == WM_QUIT || (msg.message == WM_HOTKEY && msg.wParam == HOTKEY as usize)
            {
                return Ok(false);
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg)
            };
        }
        Ok(true)
    }
}
impl Drop for Notifications {
    fn drop(&mut self) {
        for h in self.hooks.drain(..) {
            unsafe { UnhookWinEvent(h) };
        }
        unsafe {
            UnregisterHotKey(null_mut(), HOTKEY);
            CloseHandle(self.stop)
        };
    }
}

struct Managed {
    snap: Snapshot,
    hover: HoverState,
    slide: Slide,
    depth: i32,
    raised: bool,
    menu: Option<usize>,
    context: Option<usize>,
    last_face: Option<u64>,
    thumbnail_hover: Option<(Rect, u64)>,
    join: Join,
    media: Widget,
    reflow: Option<media::motion::Reflow>,
}
impl Managed {
    fn attach(bar: Bar, guard: &mut rescue::Monitor) -> Result<Option<Self>, String> {
        // Detaching a previous generation of this shell window may have restored its work area.
        let Some(bar) = native::bar_info(bar.hwnd, bar.pid) else {
            return Ok(None);
        };
        let Ok(snap) = Snapshot::capture(bar, std::process::id()) else {
            return Ok(None);
        };
        if let Err(e) = guard.watch(bar) {
            guard.check()?;
            if native::bar_info(bar.hwnd, bar.pid).is_none() || lease::region_empty(bar) {
                return Ok(None);
            }
            return Err(e);
        }
        if let Err(e) = snap.claim() {
            guard.forget(bar.hwnd as usize)?;
            if native::bar_info(bar.hwnd, bar.pid).is_none() {
                return Ok(None);
            }
            return Err(e);
        }
        snap.displace_work();
        let depth = Self::thickness(bar);
        Ok(Some(Self {
            snap,
            hover: HoverState::default(),
            slide: Slide::default(),
            depth,
            raised: false,
            menu: None,
            context: None,
            last_face: None,
            thumbnail_hover: None,
            join: Join::default(),
            media: Widget::default(),
            reflow: None,
        }))
    }
    fn thickness(bar: Bar) -> i32 {
        if bar.strip.edge.horizontal() {
            bar.strip.bounds.height()
        } else {
            bar.strip.bounds.width()
        }
    }
    fn observe(&mut self, bar: Bar, guard: &mut rescue::Monitor) -> Result<(), String> {
        if self.reflow.as_ref().is_some_and(|motion| motion.clipped) {
            return Ok(());
        }
        let full = Self::thickness(bar);
        if bar.display.work == self.snap.bar.display.work {
            self.snap.displace_work();
        }
        let empty = lease::region_empty(bar);
        // The styler temporarily clears its clip while replacing the layout.
        // Keep the last real compact region until Explorer publishes the new one.
        if self.reflow.is_some() && lease::current_compact(bar).is_none() {
            return Ok(());
        }
        if self.depth == full && empty {
            // A stale shell outline must not leave a supposedly visible desktop taskbar invisible.
            self.depth = 0;
        } else if (self.depth == full || self.depth == 0) && !empty {
            let face = lease::current_compact(bar);
            if face != self.snap.compact {
                guard.refresh(bar.hwnd as usize)?;
                self.snap.refresh_outline()?;
            }
            // Explorer reapplied its resting region during a style switch or monitor change.
            if self.depth == 0 {
                self.depth = full;
            }
        }
        Ok(())
    }
    fn reflow_offset(
        &mut self,
        allow: bool,
        reserve: i32,
        now: u64,
    ) -> Result<Option<policy::Point>, String> {
        let Some(mut motion) = self.reflow.take() else {
            return Ok(None);
        };
        let full = Self::thickness(self.snap.bar);
        let Some(face) = self.snap.compact else {
            if motion.clipped && self.snap.owns() {
                self.snap.apply(full, full)?;
            } else if allow && now.saturating_sub(motion.requested_at) <= 2800 {
                self.reflow = Some(motion);
            }
            return Ok(None);
        };
        if !allow
            || !self.snap.owns()
            || self.depth != full
            || now.saturating_sub(motion.requested_at) > 2800
            || motion
                .started
                .is_some_and(|start| now.saturating_sub(start) >= media::motion::DURATION)
        {
            if motion.clipped && self.snap.owns() {
                self.snap.apply(full, full)?;
            }
            return Ok(None);
        }
        if motion.started.is_none() {
            let moved =
                (motion.from.left - face.left).abs() + (motion.from.top - face.top).abs() >= 4;
            let ready = reserve == 0
                || media::attached_placement(self.snap.bar.strip, face, reserve, self.snap.bar.dpi)
                    .is_some();
            if !moved || !ready {
                self.reflow = Some(motion);
                return Ok(None);
            }
            motion.started = Some(now);
            if !motion.show(policy::Point {
                x: motion.from.left,
                y: motion.from.top,
            }) {
                return Ok(None);
            }
            self.snap.apply(0, full)?;
            motion.clipped = true;
            motion.release_freeze();
        }
        let point = media::motion::position(
            motion.from,
            face,
            now.saturating_sub(motion.started.unwrap_or(now)),
        );
        if !motion.show(point) {
            self.snap.apply(full, full)?;
            return Ok(None);
        }
        self.reflow = Some(motion);
        Ok(Some(policy::Point {
            x: point.x - face.left,
            y: point.y - face.top,
        }))
    }

    fn draw(
        &mut self,
        settings: &Settings,
        visible: bool,
        raise: bool,
        now: u64,
    ) -> Result<bool, String> {
        let bar = self.snap.bar;
        let fraction = self
            .slide
            .update(visible, now, if settings.animate { 165 } else { 0 });
        let full = Self::thickness(bar);
        let depth = (fraction * f64::from(full)).round() as i32;
        if depth != self.depth {
            if let Err(err) = self.snap.apply(depth, full) {
                if !self.snap.owns() {
                    return Ok(false);
                }
                return Err(err);
            }
            self.depth = depth;
        }
        if raise != self.raised {
            self.snap.raise(raise);
            self.raised = raise;
        }
        Ok((visible && depth < full) || (!visible && depth > 0))
    }
}
impl Drop for Managed {
    fn drop(&mut self) {
        self.join.clear();
        self.snap.restore()
    }
}

#[derive(Default)]
struct Model {
    managed: HashMap<usize, Managed>,
    displays: Vec<Rect>,
    scene: Scene,
    names: native::ProcessNames,
    shell: u32,
    media: Option<Worker>,
    media_reserve: media::Reserve,
    reserve_changed_at: u64,
}
impl Model {
    fn sync(&mut self, guard: &mut rescue::Monitor) -> Result<(), String> {
        self.displays = native::displays();
        self.shell = native::shell_pid();
        let bars = if native::tablet() || native::explorer_auto_hide() {
            Vec::new()
        } else {
            native::bars()
        };
        let gone: Vec<_> = self
            .managed
            .iter()
            .filter(|(_, m)| {
                !m.snap.owns()
                    || !bars.iter().any(|b| {
                        b.hwnd == m.snap.bar.hwnd
                            && b.pid == m.snap.bar.pid
                            && b.strip == m.snap.bar.strip
                            && b.dpi == m.snap.bar.dpi
                    })
            })
            .map(|(k, _)| *k)
            .collect();
        for k in gone {
            self.managed.remove(&k);
            guard.forget(k)?;
        }
        for bar in bars {
            let key = bar.hwnd as usize;
            if let Some(m) = self.managed.get_mut(&key) {
                m.observe(bar, guard)?;
            } else if let Some(m) = Managed::attach(bar, guard)? {
                self.managed.insert(key, m);
            }
        }
        Ok(())
    }
    fn update(&mut self, settings: &Settings, now: u64) -> Result<u32, String> {
        let cursor = native::cursor();
        let thumbnail = cursor.and_then(|p| native::hovered_thumbnail(p, self.shell));
        let mut delay = 1200;
        if settings.media_player && !self.managed.is_empty() {
            if self.media.is_none() {
                self.media = Worker::start();
            }
        } else {
            self.media = None;
        }
        let track = self.media.as_ref().and_then(Worker::snapshot);
        let mut reserve = media::Reserve::default();
        if settings.island
            && settings.media_player
            && (track.is_some() || self.managed.values().any(|m| m.media.is_visible()))
        {
            let direction = if settings.media_side == crate::settings::MediaSide::Left {
                1
            } else {
                -1
            };
            for (horizontal, length, target) in [
                (true, media::HORIZONTAL_DIPS, &mut reserve.horizontal),
                (false, media::VERTICAL_DIPS, &mut reserve.vertical),
            ] {
                let mut matching = self
                    .managed
                    .values()
                    .filter(|m| m.snap.bar.strip.edge.horizontal() == horizontal)
                    .peekable();
                let previous = self.media_reserve.along(horizontal);
                if matching.peek().is_some()
                    && (previous == direction * length
                        || matching.all(|m| {
                            let bar = m.snap.bar;
                            let px = (length + 24) * bar.dpi as i32 / 96;
                            m.snap.compact.is_some_and(|face| {
                                let free = if horizontal {
                                    bar.strip.display.width() - face.width()
                                } else {
                                    bar.strip.display.height() - face.height()
                                };
                                free >= px
                            })
                        }))
                {
                    *target = direction * length;
                }
            }
        }
        if reserve != self.media_reserve {
            if settings.animate {
                for managed in self.managed.values_mut() {
                    let bar = managed.snap.bar;
                    let horizontal = bar.strip.edge.horizontal();
                    if reserve.along(horizontal) != self.media_reserve.along(horizontal)
                        && managed.reflow.is_none()
                        && managed.depth == Managed::thickness(bar)
                        && let Some(face) = managed.snap.compact
                    {
                        let radius = i32::from(settings.radius) * bar.dpi as i32 / 48;
                        let expected = media::motion::expected_face(
                            bar.strip,
                            face,
                            reserve.along(horizontal),
                            bar.dpi,
                            settings.alignment,
                        );
                        managed.reflow =
                            media::motion::Reflow::capture(bar.hwnd, face, expected, radius, now);
                    }
                }
            }
            self.reserve_changed_at = now;
            self.media_reserve = reserve;
        }
        for m in self.managed.values_mut() {
            let bar = m.snap.bar.strip;
            let face = if settings.island {
                m.snap.compact
            } else {
                None
            };
            let covered = policy::is_covered(bar, face, &self.scene.windows);
            let panel = self
                .scene
                .panels
                .iter()
                .find(|p| p.monitor == bar.display)
                .copied();
            let thumbnail_held = cursor.is_some_and(|point| {
                grouped_thumbnail_hold(
                    bar.display,
                    &self.scene.thumbnail_groups,
                    point,
                    now,
                    &mut m.thumbnail_hover,
                )
            });
            let shell_menu = panel.is_some()
                || self.scene.menus.contains(&bar.display)
                || thumbnail_held
                || thumbnail == Some(bar.display);
            let under_cursor = cursor.is_some_and(|p| face.unwrap_or(bar.bounds).contains(p));
            if (m.depth > 0 && under_cursor) || shell_menu || m.menu.is_some() {
                m.last_face = Some(now);
            }
            // Tray apps can own their own context menu. Associate it with the bar the user just
            // left and retain that association after the pointer moves up into the popup.
            m.context = related_context(
                bar.display,
                &self.scene.context_menus,
                m.context,
                m.last_face
                    .is_some_and(|last| now.saturating_sub(last) <= 400),
            );
            let menu = shell_menu || m.context.is_some();
            let media_hold = track.is_some() && cursor.is_some_and(|p| m.media.holds(p));
            let surface = policy::BarInput {
                bar,
                island: face,
                dpi: m.snap.bar.dpi,
            };
            let pointer = cursor.map(|cursor| policy::PointerFrame {
                cursor,
                now_ms: now,
            });
            // The lock screen or a secure input desktop can deny GetCursorPos. Release the bar.
            let visible = pointer.is_none_or(|p| {
                m.hover.update(
                    surface,
                    settings.zone,
                    &self.displays,
                    p,
                    covered,
                    menu || media_hold,
                )
            });
            let offset = m.reflow_offset(
                visible && !menu && settings.animate && pointer.is_some(),
                reserve.along(bar.edge.horizontal()),
                now,
            )?;
            let animating = m.draw(
                settings,
                visible,
                cursor.is_some() && (menu || (covered && visible)),
                now,
            )?;
            let active_panel = panel.map(|p| p.hwnd as usize);
            if active_panel.is_some() && active_panel != m.menu {
                m.snap.raise(true)
            }
            m.menu = active_panel;
            let bridge = if let Some(p) = pointer {
                m.join.update(surface, panel, p, settings)
            } else {
                m.join.clear();
                false
            };
            let media_frame = if let Some(worker) = &self.media
                && visible
                && cursor.is_some()
                && m.depth == Managed::thickness(m.snap.bar)
                && let Some(place) = {
                    let reserved = reserve.along(bar.edge.horizontal());
                    if reserved == 0 {
                        media::placement(bar, face, settings.media_side, surface.dpi)
                    } else {
                        face.and_then(|face| {
                            media::attached_placement(bar, face, reserved, surface.dpi)
                        })
                        .or_else(|| {
                            (now.saturating_sub(self.reserve_changed_at) >= 1800)
                                .then(|| {
                                    media::placement(bar, face, settings.media_side, surface.dpi)
                                })
                                .flatten()
                        })
                    }
                }
                && panel.is_none_or(|panel| panel.bounds.intersection(place.bounds).is_none())
            {
                if track.is_some() {
                    delay = delay.min(500);
                }
                let animating = m.media.update(
                    offset.map_or(place, |delta| place.offset(delta)),
                    (worker, track.clone()),
                    surface.dpi,
                    settings,
                    now,
                );
                if let Some(point) = cursor
                    && track.is_some()
                    && m.media.hover_details(bar, point, settings.animate, now)
                {
                    delay = delay.min(35);
                }
                animating
            } else {
                m.media.clear(now);
                false
            };
            if animating || bridge || media_frame || offset.is_some() {
                delay = 16
            } else if covered {
                delay = delay.min(32)
            }
        }
        Ok(delay)
    }
    fn publish(&self, settings: &Settings) {
        let geometry = if settings.island {
            self.managed.values().find_map(|m| {
                let mut bar = m.snap.bar;
                bar.display = native::display_for(bar.hwnd)?;
                layout(bar, m.snap.compact)
            })
        } else {
            None
        };
        crate::integrations::layout_changed(
            geometry,
            !self.scene.menus.is_empty()
                || !self.scene.panels.is_empty()
                || self.managed.values().any(|m| m.context.is_some()),
            self.media_reserve,
        );
    }
}

fn grouped_thumbnail_hold(
    display: Rect,
    groups: &[(Rect, Rect)],
    pointer: policy::Point,
    now: u64,
    hold: &mut Option<(Rect, u64)>,
) -> bool {
    if let Some((_, bounds)) = groups
        .iter()
        .find(|(owner, bounds)| *owner == display && bounds.contains(pointer))
    {
        *hold = Some((*bounds, now.saturating_add(550)));
        return true;
    }
    if hold.is_some_and(|(bounds, deadline)| now <= deadline && bounds.contains(pointer)) {
        return true;
    }
    *hold = None;
    false
}

fn related_context(
    display: Rect,
    popups: &[(usize, Rect)],
    active: Option<usize>,
    recent: bool,
) -> Option<usize> {
    if let Some(h) = active
        && popups.contains(&(h, display))
    {
        return Some(h);
    }
    if recent {
        popups
            .iter()
            .find(|(_, home)| *home == display)
            .map(|(h, _)| *h)
    } else {
        None
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn run(initial: &Settings, settings_path: &Path) -> Result<(), String> {
    let _dpi = DpiScope::enter();
    let events = Notifications::create()?;
    let mut guard = rescue::Monitor::start()?;
    // Drop order matters: Managed objects return the shell to its original state before the
    // recovery pipe closes. The child also handles abrupt termination or a process crash.
    let mut model = Model::default();
    let start = Instant::now();
    let mut settings = initial.clone();
    let mut stamp = modified(settings_path);
    let (mut next_sync, mut next_scene, mut next_config) = (0, 0, 0);
    loop {
        let now = start.elapsed().as_millis() as u64;
        let mut publish = false;
        if now >= next_config {
            if crate::csharp_running() {
                return Err(
                    "The C# taskbar engine started. The Rust engine has released the desktop."
                        .into(),
                );
            }
            guard.check()?;
            let current = modified(settings_path);
            if current != stamp
                && let Ok(new) = Settings::load(settings_path)
            {
                settings = new;
                stamp = current;
                publish = true;
                next_sync = now;
            }
            next_config = now + 900;
        }
        if now >= next_sync {
            model.sync(&mut guard)?;
            next_sync = now
                + if model.managed.values().any(|m| m.reflow.is_some()) {
                    27
                } else {
                    850
                };
            publish = true;
        }
        if now >= next_scene
            || (DIRTY.load(Ordering::Relaxed) && next_scene.saturating_sub(now) <= 1160)
        {
            DIRTY.store(false, Ordering::Relaxed);
            model.scene = if model.managed.is_empty() {
                model.names = native::ProcessNames::default();
                Scene::default()
            } else {
                native::scene(&mut model.names)
            };
            next_scene = now + 1200;
            publish = true;
        }
        let before = model.media_reserve;
        let mut delay = model.update(&settings, now)?;
        if before != model.media_reserve && settings.animate {
            next_sync = now;
        }
        if publish || before != model.media_reserve {
            model.publish(&settings)
        }
        for due in [next_sync, next_scene, next_config] {
            delay = delay.min(due.saturating_sub(now).min(u32::MAX as u64) as u32);
        }
        if DIRTY.load(Ordering::Relaxed) {
            delay = delay.min(40)
        }
        if !events.wait(delay)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_owned_tray_menus_remain_held_after_the_pointer_leaves() {
        let a = Rect::new(0, -800, 1200, 0);
        let b = Rect::new(0, 0, 1200, 800);
        let menus = [(19, a)];
        assert_eq!(related_context(a, &menus, None, true), Some(19));
        assert_eq!(related_context(a, &menus, Some(19), false), Some(19));
        assert_eq!(related_context(b, &menus, None, true), None);
        assert_eq!(related_context(a, &[(20, a)], Some(19), false), None);
    }
    fn bar(width: i32, dpi: u32, workbottom: i32) -> Bar {
        let display = Rect::new(-width, 0, 0, 1800);
        Bar {
            hwnd: null_mut(),
            pid: 1,
            strip: policy::NativeBar {
                display,
                bounds: Rect::new(-width, 1704, 0, 1800),
                edge: Edge::Bottom,
            },
            dpi,
            display: native::Display {
                bounds: display,
                work: Rect::new(-width, 0, 0, workbottom),
                primary: true,
            },
        }
    }
    #[test]
    fn search_reserves_only_the_reclaimed_area_beyond_its_own_gutter() {
        let face = Some(Rect::new(-1550, 1712, -350, 1796));
        assert_eq!(layout(bar(2880, 192, 1800), face), Some((600, -163, 30)));
        assert_eq!(layout(bar(2880, 192, 1704), face), Some((600, -163, 0)));
    }
    #[test]
    fn start_fits_a_physically_narrow_screen_and_the_right_anchor_never_shifts_outward() {
        let face = Some(Rect::new(-350, 1712, -2, 1796));
        assert_eq!(layout(bar(720, 192, 1800), face), Some((360, 0, 30)));
    }
}
#[test]
fn moving_between_preview_cells_survives_a_popup_handoff() {
    let monitor = Rect::new(0, 0, 1800, 1000);
    let other = Rect::new(0, -1000, 1800, 0);
    let thumbnails = [(monitor, Rect::new(400, 650, 1000, 850))];
    let mut hold = None;
    assert!(grouped_thumbnail_hold(
        monitor,
        &thumbnails,
        policy::Point { x: 450, y: 720 },
        1,
        &mut hold
    ));
    assert!(grouped_thumbnail_hold(
        monitor,
        &[],
        policy::Point { x: 825, y: 720 },
        280,
        &mut hold
    ));
    assert!(!grouped_thumbnail_hold(
        monitor,
        &[],
        policy::Point { x: 825, y: 720 },
        600,
        &mut hold
    ));
    assert!(!grouped_thumbnail_hold(
        other,
        &thumbnails,
        policy::Point { x: 450, y: 720 },
        601,
        &mut hold
    ));
}
