//! Live shell coordination. Merely linking this module never opens a monitor or changes Windows.
mod app_labels;
mod bar_menu;
mod lease;
mod native;
mod rescue;

use crate::{
    join::Join,
    media::{self, Widget, Worker},
    policy::{self, Edge, HoverState, Rect, Slide},
    profiles::{BottomIsland, Geometry},
    settings::Settings,
};
use lease::Snapshot;
use native::{Bar, Scene};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Instant, SystemTime};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub use native::Monitor;

pub fn monitor_choices() -> Vec<Monitor> {
    let _dpi = DpiScope::enter();
    native::monitors()
}

pub(crate) fn activation_preview(
    settings: &Settings,
    monitors: &[Monitor],
    selected: Option<&str>,
) -> Vec<Rect> {
    native::bars()
        .into_iter()
        .filter_map(|bar| {
            let id = monitors
                .iter()
                .find(|m| m.bounds == bar.strip.display)
                .map(|m| m.id.as_str());
            if selected.is_some() && selected != id {
                return None;
            }
            let face = settings
                .island
                .then(|| lease::preview_compact(bar))
                .flatten();
            Some(policy::activation_band(
                bar.strip,
                face,
                settings.reveal(id),
                bar.dpi,
            ))
        })
        .collect()
}

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
    if let Some(island) = primary_island() {
        lines.push(format!("Primary bottom island uses a {} DIP Start panel shifted {} DIPs and a {} DIP tray shift.", island.start_width, island.start_shift, island.tray_shift));
    }
    Ok(lines.join("\n"))
}

fn layout(bar: Bar, face: Option<Rect>) -> Geometry {
    let face = face?;
    if !bar.display.primary || bar.strip.edge != Edge::Bottom {
        return None;
    }
    let work = bar.display.work;
    let available = native::px_to_dip(work.width(), bar.dpi);
    let start = policy::anchored_start(bar.strip, face, 160, bar.dpi);
    let start_width = native::px_to_dip(start.width(), bar.dpi).min(available);
    let centered_left = work.left + (work.width() - start.width()) / 2;
    let start_shift = native::px_to_dip(start.left - centered_left, bar.dpi);
    let tray_shift = (policy::SIDE_INSET_DIP - native::px_to_dip(work.right - face.right, bar.dpi))
        .clamp((384 - available).min(0), 0);
    Some(BottomIsland {
        start_width,
        start_shift,
        tray_shift,
    })
}

pub fn primary_island() -> Geometry {
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
static MEDIA_CHANGED: AtomicBool = AtomicBool::new(false);
static TOPOLOGY_CHANGED: AtomicBool = AtomicBool::new(false);
static SHELL_LAYOUT_CHANGED: AtomicBool = AtomicBool::new(false);
static TASKBAR_STYLE_CHANGED: AtomicBool = AtomicBool::new(false);
static ISLAND_STYLER_ACTIVE: AtomicBool = AtomicBool::new(false);
static TASKBAR_STYLER_ACTIVE: AtomicBool = AtomicBool::new(false);
static CONFIG_CHANGED: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static OBSERVER_THREAD: AtomicU32 = AtomicU32::new(0);
const STYLE_CHANGED: u32 = WM_APP + 0x62;

pub fn style_changed() {
    let thread = OBSERVER_THREAD.load(Ordering::Relaxed);
    if thread != 0 {
        unsafe { PostThreadMessageW(thread, STYLE_CHANGED, 0, 0) };
    }
}

pub fn taskbar_style_applied(island: bool, refresh_outline: bool) {
    ISLAND_STYLER_ACTIVE.store(island, Ordering::Relaxed);
    TASKBAR_STYLER_ACTIVE.store(true, Ordering::Relaxed);
    if refresh_outline {
        TASKBAR_STYLE_CHANGED.store(true, Ordering::Relaxed);
        style_changed();
    }
}

pub fn taskbar_style_restored() {
    ISLAND_STYLER_ACTIVE.store(false, Ordering::Relaxed);
    TASKBAR_STYLER_ACTIVE.store(false, Ordering::Relaxed);
}

unsafe extern "system" fn system_change(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if unsafe { bar_menu::paint_message(msg, lp) } {
        return 1;
    }
    let setting = if msg == WM_SETTINGCHANGE && lp != 0 {
        // Compare only the names we handle. The broadcast text belongs to Windows.
        ["ConvertibleSlateMode", "SystemDockMode", "TraySettings"]
            .iter()
            .any(|name| {
                native::wide(name)
                    .iter()
                    .enumerate()
                    .all(|(i, c)| unsafe { *(lp as *const u16).add(i) == *c })
            })
    } else {
        false
    };
    if matches!(msg, WM_DISPLAYCHANGE | WM_DEVICECHANGE | WM_POWERBROADCAST)
        || setting
        || msg == TASKBAR_CREATED.load(Ordering::Relaxed) && msg != 0
    {
        TOPOLOGY_CHANGED.store(true, Ordering::Relaxed);
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn notification_window() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = native::wide("SmartTaskbar.Rust.Notifications");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(system_change),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return null_mut();
        }
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(native::wide("TaskbarCreated").as_ptr()),
            Ordering::Relaxed,
        );
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
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
            || (object == 0
                && child == 0
                && (event != EVENT_OBJECT_LOCATIONCHANGE || unsafe { IsWindowVisible(h) } != 0)
                && unsafe { GetAncestor(h, GA_ROOT) } == h))
    {
        DIRTY.store(true, Ordering::Relaxed)
    }
}

struct Notifications {
    stop: HANDLE,
    config: HANDLE,
    hwnd: HWND,
    hooks: Vec<HWINEVENTHOOK>,
}
impl Notifications {
    fn create(path: &Path) -> Result<Self, String> {
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
        use std::os::windows::ffi::OsStrExt;
        let config = path.parent().map_or(INVALID_HANDLE_VALUE, |dir| {
            let dir = dir.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
            unsafe {
                FindFirstChangeNotificationW(
                    dir.as_ptr(),
                    0,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                )
            }
        });
        OBSERVER_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
        Ok(Self {
            stop,
            config,
            hwnd: notification_window(),
            hooks,
        })
    }
    fn wait(&mut self, ms: u32) -> Result<bool, String> {
        let handles = [self.stop, self.config];
        let count = if self.config == INVALID_HANDLE_VALUE {
            1
        } else {
            2
        };
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(
                count,
                handles.as_ptr(),
                ms,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if result == WAIT_FAILED {
            return Err("Windows could not wait for taskbar input.".into());
        }
        if result == WAIT_OBJECT_0 {
            return Ok(false);
        }
        if count == 2 && result == WAIT_OBJECT_0 + 1 {
            CONFIG_CHANGED.store(true, Ordering::Relaxed);
            if unsafe { FindNextChangeNotification(self.config) } == 0 {
                unsafe { FindCloseChangeNotification(self.config) };
                self.config = INVALID_HANDLE_VALUE;
            }
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
            if msg.hwnd.is_null() && msg.message == media::SESSION_CHANGED {
                MEDIA_CHANGED.store(true, Ordering::Relaxed);
                continue;
            }
            if msg.hwnd.is_null() && msg.message == STYLE_CHANGED {
                // Give Explorer one layout pass before sampling the new outline.
                SHELL_LAYOUT_CHANGED.store(true, Ordering::Relaxed);
                continue;
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
        OBSERVER_THREAD.store(0, Ordering::Relaxed);
        for h in self.hooks.drain(..) {
            unsafe { UnhookWinEvent(h) };
        }
        unsafe {
            if self.config != INVALID_HANDLE_VALUE {
                FindCloseChangeNotification(self.config);
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            UnregisterHotKey(null_mut(), HOTKEY);
            CloseHandle(self.stop)
        };
    }
}

struct Managed {
    snap: Snapshot,
    hover: HoverState,
    overlap: Option<(Option<Rect>, bool)>,
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
    outline_needs_review: bool,
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
            overlap: None,
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
            outline_needs_review: false,
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
                self.outline_needs_review = false;
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
        let mut outline_released = false;
        if depth == full
            && self.outline_needs_review
            && !self.reflow.as_ref().is_some_and(|motion| motion.clipped)
        {
            self.outline_needs_review = false;
            outline_released = self.snap.release_stale_compact_outline()?;
            if outline_released {
                SHELL_LAYOUT_CHANGED.store(true, Ordering::Relaxed);
            }
        }
        Ok(outline_released || (visible && depth < full) || (!visible && depth > 0))
    }
}
impl Drop for Managed {
    fn drop(&mut self) {
        self.join.clear();
        self.snap.restore()
    }
}

fn measured_media_room(
    bars: impl Iterator<Item = (i32, Option<i32>, u32)>,
    media_length: i32,
) -> bool {
    let mut measured = bars
        .filter_map(|(display, face, dpi)| {
            face.map(|face| display - face >= (media_length + 24) * dpi as i32 / 96)
        })
        .peekable();
    // A hidden or not-yet-laid-out bar supplies no measurement. Every measured
    // bar must fit because Explorer shares these styles across displays.
    measured.peek().is_some() && measured.all(|fits| fits)
}

#[derive(Default)]
struct Model {
    managed: HashMap<usize, Managed>,
    displays: Vec<Rect>,
    monitors: Vec<Monitor>,
    scene: Scene,
    scene_windows: Vec<HWND>,
    names: native::ProcessNames,
    shell: u32,
    media: Option<Worker>,
    media_reserve: media::Reserve,
    reserve_changed_at: u64,
    cursor: Option<policy::Point>,
    pointer_idle_ms: Option<u32>,
    app_label: app_labels::Hover,
    native_touch: bool,
}
impl Model {
    fn sync(&mut self, guard: &mut rescue::Monitor, topology_changed: bool) -> Result<(), String> {
        if topology_changed || self.monitors.is_empty() {
            self.monitors = native::monitors();
            self.displays = self.monitors.iter().map(|m| m.bounds).collect();
        }
        self.shell = native::shell_pid();
        self.native_touch = native::tablet();
        let bars = if self.native_touch || native::explorer_auto_hide() {
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
    fn update(
        &mut self,
        settings: &Settings,
        now: u64,
        cursor: Option<policy::Point>,
    ) -> Result<u32, String> {
        let pointer_moved = self.cursor != cursor;
        self.cursor = cursor;
        let mut thumbnail = None;
        let mut delay = 2400;
        let mut stationary_can_wait = true;
        let labels = TASKBAR_STYLER_ACTIVE.load(Ordering::Relaxed) && !self.managed.is_empty();
        let mut label_target = None;
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
                let matching = self
                    .managed
                    .values()
                    .filter(|m| m.snap.bar.strip.edge.horizontal() == horizontal);
                let previous = self.media_reserve.along(horizontal);
                if (previous == direction * length && matching.clone().next().is_some())
                    || measured_media_room(
                        matching.map(|m| {
                            let span = |rect: Rect| {
                                if horizontal {
                                    rect.width()
                                } else {
                                    rect.height()
                                }
                            };
                            (
                                span(m.snap.bar.strip.display),
                                m.snap.compact.map(span),
                                m.snap.bar.dpi,
                            )
                        }),
                        length,
                    )
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
                        && self.scene.foreground_fullscreen != Some(bar.strip.display)
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
            let fullscreen = self.scene.foreground_fullscreen == Some(bar.display);
            let reveal = settings.reveal(
                self.monitors
                    .iter()
                    .find(|monitor| monitor.bounds == bar.display)
                    .map(|monitor| monitor.id.as_str()),
            );
            let face = if settings.island {
                m.snap.compact
            } else {
                None
            };
            let cover = if fullscreen {
                policy::Cover::ForegroundFullscreen
            } else {
                let covered = match m.overlap {
                    Some((cached_face, covered)) if cached_face == face => covered,
                    _ => {
                        let covered = policy::is_covered(bar, face, &self.scene.windows);
                        m.overlap = Some((face, covered));
                        covered
                    }
                };
                if covered {
                    policy::Cover::Overlap
                } else {
                    policy::Cover::Clear
                }
            };
            let panel = self
                .scene
                .panels
                .iter()
                .find(|p| !fullscreen && p.monitor == bar.display)
                .copied();
            let menu = if fullscreen {
                m.thumbnail_hover = None;
                m.last_face = None;
                m.context = None;
                false
            } else {
                // Hit-test thumbnails only when the bar or a thumbnail can already be in use.
                let hovered_thumbnail = if panel.is_none()
                    && (m.depth > 0 || m.thumbnail_hover.is_some())
                    && (cover == policy::Cover::Overlap || !self.scene.thumbnail_groups.is_empty())
                {
                    *thumbnail.get_or_insert_with(|| {
                        cursor.and_then(|p| native::hovered_thumbnail(p, self.shell))
                    })
                } else {
                    None
                };
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
                    || hovered_thumbnail == Some(bar.display);
                let under_cursor = cursor.is_some_and(|p| face.unwrap_or(bar.bounds).contains(p));
                if (m.depth > 0 && under_cursor) || shell_menu || m.menu.is_some() {
                    m.last_face = Some(now);
                }
                // Keep tray app context menus tied to the bar the pointer just left.
                m.context = related_context(
                    bar.display,
                    &self.scene.context_menus,
                    m.context,
                    m.last_face
                        .is_some_and(|last| now.saturating_sub(last) <= 400),
                );
                shell_menu || m.context.is_some()
            };
            let media_hold =
                !fullscreen && track.is_some() && cursor.is_some_and(|p| m.media.holds(p));
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
            let visible = pointer.map_or(!fullscreen, |p| {
                m.hover.update(
                    surface,
                    reveal,
                    &self.displays,
                    p,
                    cover,
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
                cursor.is_some() && (menu || (cover == policy::Cover::Overlap && visible)),
                now,
            )?;
            let active_panel = panel.map(|p| p.hwnd as usize);
            if active_panel.is_some() && active_panel != m.menu {
                m.snap.raise(true)
            }
            m.menu = active_panel;
            if labels
                && visible
                && let Some(point) = cursor
                && face.unwrap_or(bar.bounds).contains(point)
            {
                label_target = Some((m.snap.bar, point, face.unwrap_or(bar.bounds)));
            }
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
                delay = 16;
                stationary_can_wait = false;
            } else if let Some(point) = cursor {
                if cover == policy::Cover::Overlap {
                    // Read the cursor at animation cadence near the edge. Most of
                    // a maximized session spends its time well away from the bar.
                    let interval =
                        pointer_interval(bar, point, surface.dpi, pointer_moved, visible);
                    delay = delay.min(interval);
                    stationary_can_wait &= interval != 32;
                } else if m.media.is_visible() || !self.scene.context_menus.is_empty() {
                    delay = delay.min(85);
                }
            }
            stationary_can_wait &= !menu && !m.media.is_visible() && m.reflow.is_none();
        }
        if labels {
            delay = delay.min(125);
        }
        if let Some(tick) = self.app_label.update(label_target, settings, now) {
            delay = delay.min(tick);
            stationary_can_wait = false;
        }
        self.pointer_idle_ms = stationary_can_wait.then_some(if delay == 60 { 140 } else { delay });
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
        crate::integrations::layout_changed(crate::integrations::Layout {
            geometry,
            flyout_open: !self.scene.menus.is_empty()
                || !self.scene.panels.is_empty()
                || self.managed.values().any(|m| m.context.is_some()),
            reserve: self.media_reserve,
            native_touch: self.native_touch,
        });
    }
}

fn pointer_interval(
    bar: policy::NativeBar,
    cursor: policy::Point,
    dpi: u32,
    moved: bool,
    showing: bool,
) -> u32 {
    let bounds = bar.bounds;
    let margin = (dpi as i32 * 48 / 96).max(48);
    let near = Rect::new(
        bounds.left.saturating_sub(margin),
        bounds.top.saturating_sub(margin),
        bounds.right.saturating_add(margin),
        bounds.bottom.saturating_add(margin),
    );
    if showing || near.contains(cursor) {
        32
    } else if moved {
        60
    } else {
        140
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
    let mut events = Notifications::create(settings_path)?;
    let mut guard = rescue::Monitor::start()?;
    // Drop order matters: Managed objects return the shell to its original state before the
    // recovery pipe closes. The child also handles abrupt termination or a process crash.
    let mut model = Model::default();
    let start = Instant::now();
    let mut settings = initial.clone();
    let mut stamp = modified(settings_path);
    let scene_fallback = if events.hooks.len() == 3 { 3200 } else { 1800 };
    const SCENE_DEBOUNCE: u64 = 40;
    let mut next_sync = 0;
    let mut next_topology = 0;
    let mut next_scene = 0u64;
    let mut next_config = 0;
    let mut next_health = 0;
    let mut next_update = 0;
    loop {
        bar_menu::show_pending(events.hwnd, settings_path);
        let now = start.elapsed().as_millis() as u64;
        let mut publish = false;
        if now >= next_health {
            if crate::csharp_running() {
                return Err(
                    "The C# taskbar engine started. The Rust engine has released the desktop."
                        .into(),
                );
            }
            guard.check()?;
            next_health = now + 4500;
        }
        let config_changed = CONFIG_CHANGED.swap(false, Ordering::Relaxed);
        if config_changed || now >= next_config {
            if crate::is_paused(settings_path) {
                break;
            }
            let current = modified(settings_path);
            if current != stamp
                && let Ok(new) = Settings::load(settings_path)
            {
                settings = new;
                crate::integrations::settings_changed(&settings);
                stamp = current;
                publish = true;
                next_sync = now;
            }
            next_config = now
                + if events.config == INVALID_HANDLE_VALUE {
                    1500
                } else {
                    8000
                };
        }
        let topology_event = TOPOLOGY_CHANGED.swap(false, Ordering::Relaxed);
        let topology_changed = topology_event || now >= next_topology;
        if topology_changed {
            next_topology = now + if events.hwnd.is_null() { 1250 } else { 9500 };
            next_sync = now;
            next_scene = now;
        }
        let taskbar_style_changed = TASKBAR_STYLE_CHANGED.swap(false, Ordering::Relaxed);
        if taskbar_style_changed {
            for managed in model.managed.values_mut() {
                managed.outline_needs_review = true;
            }
        }
        if SHELL_LAYOUT_CHANGED.swap(false, Ordering::Relaxed) || taskbar_style_changed {
            next_sync = next_sync.min(now + 27);
        }
        if now >= next_sync {
            model.sync(&mut guard, topology_changed)?;
            next_sync = now
                + if model.managed.values().any(|m| m.reflow.is_some()) {
                    27
                } else if model.managed.is_empty() {
                    900
                } else {
                    4200
                };
            publish = true;
        }
        let dirty_due = next_scene.saturating_sub(scene_fallback - SCENE_DEBOUNCE);
        if now >= next_scene || (DIRTY.load(Ordering::Relaxed) && now >= dirty_due) {
            DIRTY.store(false, Ordering::Relaxed);
            if model.managed.is_empty() {
                model.names = native::ProcessNames::default();
                model.scene.clear();
            } else {
                native::refresh_scene(&mut model.names, &mut model.scene, &mut model.scene_windows);
                for managed in model.managed.values_mut() {
                    managed.overlap = None;
                }
            }
            next_scene = now + scene_fallback;
            publish = true;
        }
        let before = model.media_reserve;
        let media_changed = MEDIA_CHANGED.swap(false, Ordering::Relaxed);
        if publish || now >= next_update || media_changed {
            let cursor = native::cursor();
            let delay = if !publish
                && !media_changed
                && cursor == model.cursor
                && let Some(idle_ms) = model.pointer_idle_ms
            {
                idle_ms
            } else {
                model.update(&settings, now, cursor)?
            };
            next_update = now + u64::from(delay);
        }
        if before != model.media_reserve && settings.animate {
            next_sync = now;
        }
        if publish || before != model.media_reserve {
            model.publish(&settings)
        }
        let mut due = next_sync
            .min(next_topology)
            .min(next_scene)
            .min(next_config)
            .min(next_health)
            .min(next_update);
        if DIRTY.load(Ordering::Relaxed) {
            due = due.min(next_scene.saturating_sub(scene_fallback - SCENE_DEBOUNCE));
        }
        let elapsed = start.elapsed().as_millis() as u64;
        if !events.wait(due.saturating_sub(elapsed) as u32)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_listener_accepts_shell_and_posture_notifications() {
        let hwnd = notification_window();
        assert!(!hwnd.is_null());
        let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
        assert_ne!(taskbar_created, 0);
        unsafe { SendMessageW(hwnd, taskbar_created, 0, 0) };
        assert!(TOPOLOGY_CHANGED.swap(false, Ordering::Relaxed));
        let slate = native::wide("ConvertibleSlateMode");
        unsafe { SendMessageW(hwnd, WM_SETTINGCHANGE, 0, slate.as_ptr() as isize) };
        assert!(TOPOLOGY_CHANGED.swap(false, Ordering::Relaxed));
        unsafe { DestroyWindow(hwnd) };
    }

    #[test]
    fn pointer_sampling_slows_only_away_from_a_covered_bar() {
        let display = Rect::new(0, -900, 1600, 0);
        let strip = Rect::new(0, -64, 1600, 0);
        let bar = policy::NativeBar::from_bounds(display, strip).unwrap();
        let edge = policy::Point { x: 800, y: -5 };
        let away = policy::Point { x: 800, y: -400 };
        assert_eq!(pointer_interval(bar, edge, 144, false, false), 32);
        assert_eq!(pointer_interval(bar, away, 144, true, false), 60);
        assert_eq!(pointer_interval(bar, away, 144, false, false), 140);
        assert_eq!(pointer_interval(bar, away, 144, false, true), 32);
    }

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
    fn start_fits_a_physically_narrow_screen_and_the_right_anchor_never_shifts_outward() {
        let face = Some(Rect::new(-350, 1712, -2, 1796));
        let island = layout(bar(720, 192, 1800), face).unwrap();
        assert_eq!(island.start_width, 360);
        assert_eq!(island.start_shift, 0);
        assert_eq!(island.tray_shift, 0);
    }

    #[test]
    fn start_follows_the_taskbar_when_the_media_player_shifts_the_island() {
        let face = Some(Rect::new(-2110, 1712, -1210, 1796));
        let island = layout(bar(2880, 192, 1800), face).unwrap();
        assert_eq!(island.start_width, 450);
        assert_eq!(island.start_shift, -110);
    }

    #[test]
    fn an_unmeasured_secondary_bar_does_not_block_media_on_the_primary_island() {
        let fit = (2880, Some(1135), 192);
        let hidden = (2880, None, 192);
        let narrow = (1280, Some(1100), 192);
        let length = media::HORIZONTAL_DIPS;
        assert!(measured_media_room([fit, hidden].into_iter(), length));
        assert!(!measured_media_room([hidden].into_iter(), length));
        assert!(!measured_media_room(
            [fit, narrow, hidden].into_iter(),
            length
        ));
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
