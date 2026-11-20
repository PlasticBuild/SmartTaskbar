//! Live shell coordination. Merely linking this module never opens a monitor or changes Windows.
mod lease;
mod native;
mod rescue;

use crate::{
    join::Join,
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
    join: Join,
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
            join: Join::default(),
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
        let full = Self::thickness(bar);
        if bar.display.work == self.snap.bar.display.work {
            self.snap.displace_work();
        }
        let empty = lease::region_empty(bar);
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
}
impl Model {
    fn sync(&mut self, guard: &mut rescue::Monitor) -> Result<(), String> {
        self.displays = native::displays();
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
        let mut delay = 1200;
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
            let shell_menu = panel.is_some() || self.scene.menus.contains(&bar.display);
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
                m.hover
                    .update(surface, settings.zone, &self.displays, p, covered, menu)
            });
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
            if animating || bridge {
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
        );
    }
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
            next_sync = now + 850;
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
        let mut delay = model.update(&settings, now)?;
        if publish {
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
