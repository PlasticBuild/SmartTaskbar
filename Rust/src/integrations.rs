use crate::{
    profiles,
    settings::{Settings, atomic_write},
};
use std::os::windows::ffi::OsStrExt;
use std::{
    env,
    ffi::OsStr,
    path::Path,
    sync::{Mutex, OnceLock, mpsc},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::*,
};

mod search;
mod shell;
pub use shell::{configure_packaged_style, stop_packaged_style, verify_packaged_style};

use profiles::Geometry;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Layout {
    pub geometry: Geometry,
    pub flyout_open: bool,
    pub reserve: crate::media::Reserve,
    pub native_touch: bool,
}
enum Update {
    Desktop(Layout),
    Settings(Settings),
    Stop,
}
static UPDATES: OnceLock<Mutex<Option<mpsc::Sender<Update>>>> = OnceLock::new();
static LAST_LAYOUT: Mutex<Option<Layout>> = Mutex::new(None);

fn menu_open() -> bool {
    LAST_LAYOUT
        .lock()
        .ok()
        .and_then(|state| *state)
        .is_some_and(|layout| layout.flyout_open)
}

pub fn layout_changed(next: Layout) {
    let Ok(mut last) = LAST_LAYOUT.lock() else {
        return;
    };
    if *last == Some(next) {
        return;
    }
    *last = Some(next);
    send(Update::Desktop(next));
}

pub fn settings_changed(settings: &Settings) {
    send(Update::Settings(settings.clone()));
}

fn send(update: Update) {
    if let Some(Ok(bus)) = UPDATES.get().map(|v| v.lock())
        && let Some(tx) = bus.as_ref()
    {
        let _ = tx.send(update);
    }
}

pub fn check_profiles(settings: &Settings) -> Result<(), String> {
    for geometry in [
        None,
        Some(profiles::BottomIsland {
            start_width: 580,
            start_shift: -110,
            tray_shift: -108,
            search_inset: 30,
        }),
    ] {
        let items = profiles::build(settings, geometry)?;
        if items.len() != profiles::IDS.len()
            || items
                .iter()
                .any(|v| v.as_object().is_none_or(|m| m.is_empty()))
        {
            return Err("A required shell profile is empty.".into());
        }
    }
    Ok(())
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}

pub fn sign_in(path: &Path, enabled: bool) -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let command = format!(
        "\"{}\" --run --config \"{}\"",
        exe.display(),
        path.display()
    );
    let run = wide(OsStr::new(r"Software\Microsoft\Windows\CurrentVersion\Run"));
    let name = wide(OsStr::new("SmartTaskbar Rust"));
    let mut key = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            run.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Cannot open the Windows sign-in settings ({status})."
        ));
    }
    let result = (|| {
        let mut size = 0;
        let mut kind = 0;
        let present = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if present == ERROR_SUCCESS {
            if kind != REG_SZ || size > 65536 {
                return Err("The sign-in entry belongs to another installation.".into());
            }
            let mut data = vec![0u16; (size as usize).div_ceil(2)];
            let result = unsafe {
                RegQueryValueExW(
                    key,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if result != ERROR_SUCCESS {
                return Err(format!("Cannot read the sign-in entry ({result})."));
            }
            if String::from_utf16_lossy(&data).trim_end_matches('\0') != command {
                return Err(
                    "Another SmartTaskbar Rust installation owns the sign-in entry.".into(),
                );
            }
        } else if present != ERROR_FILE_NOT_FOUND {
            return Err(format!("Cannot read the sign-in entry ({present})."));
        }
        if !enabled && present == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        let bytes = wide(OsStr::new(&command));
        let result = if enabled {
            unsafe {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    bytes.as_ptr().cast(),
                    (bytes.len() * 2) as u32,
                )
            }
        } else {
            unsafe { RegDeleteValueW(key, name.as_ptr()) }
        };
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Cannot update Windows sign-in ({result})."))
        }
    })();
    unsafe {
        RegCloseKey(key);
    }
    result
}

pub struct Session {
    worker: Option<thread::JoinHandle<()>>,
}
impl Session {
    pub fn start(config: &Path, settings: &Settings) -> Result<Self, String> {
        let directory = config
            .parent()
            .ok_or("Settings directory is missing.")?
            .to_owned();
        let mut shell = shell::Host::open(&directory)?;
        let mut redirect = search::Redirect::new(&directory);
        if settings.search_redirect {
            redirect.configure(settings)?;
        }
        let (tx, rx) = mpsc::channel();
        *LAST_LAYOUT
            .lock()
            .map_err(|_| "Layout channel unavailable")? = None;
        *UPDATES
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "Layout channel unavailable")? = Some(tx);
        let mut current = settings.clone();
        let worker = thread::spawn(move || {
            let mut next = current.clone();
            let mut layout = Layout::default();
            let mut applied_layout = (None, layout.reserve);
            let mut retry = None;
            let mut observed = false;
            let mut styled = false;
            let mut native_touch = false;
            let settle = Duration::from_millis(350);
            let recover = Duration::from_secs(3);
            loop {
                let update = match retry {
                    Some(delay) => rx.recv_timeout(delay),
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                };
                match update {
                    Ok(Update::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Update::Desktop(next)) => {
                        layout = next;
                        observed = true;
                        retry = (!layout.flyout_open || layout.native_touch).then_some(settle);
                        continue;
                    }
                    Ok(Update::Settings(requested)) => {
                        next = requested;
                        retry = (observed && (!layout.flyout_open || layout.native_touch))
                            .then_some(settle);
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                retry = None;
                if layout.native_touch != native_touch {
                    let result = if layout.native_touch {
                        match shell.as_mut() {
                            Some(host) => host.restore().map(|()| None),
                            None => Ok(None),
                        }
                    } else {
                        shell::Host::open(&directory)
                    };
                    match result {
                        Ok(host) => {
                            shell = host;
                            native_touch = layout.native_touch;
                            styled = false;
                        }
                        Err(error) => {
                            record(&directory, &error);
                            retry = Some(recover);
                            continue;
                        }
                    }
                }
                let requested = (layout.geometry, layout.reserve);
                if !native_touch
                    && (!styled || !next.same_shell_style(&current) || applied_layout != requested)
                {
                    if let Some(host) = shell.as_mut() {
                        match host.apply(&next, layout.geometry, layout.reserve) {
                            Ok(true) => crate::desktop::style_changed(),
                            Ok(false) => {
                                retry = Some(settle);
                                continue;
                            }
                            Err(e) => {
                                record(&directory, &e);
                                retry = Some(recover);
                                continue;
                            }
                        }
                    }
                    applied_layout = requested;
                    styled = true;
                }
                if (next.search_redirect != current.search_redirect
                    || (next.search_redirect && next.web_engine != current.web_engine))
                    && let Err(e) = redirect.configure(&next)
                {
                    record(&directory, &e);
                    retry = Some(recover);
                    continue;
                }
                current.clone_from(&next);
            }
            if let Some(host) = shell.as_mut()
                && let Err(e) = host.restore()
            {
                record(&directory, &e);
            }
        });
        Ok(Self {
            worker: Some(worker),
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(Ok(mut bus)) = UPDATES.get().map(|v| v.lock())
            && let Some(tx) = bus.take()
        {
            let _ = tx.send(Update::Stop);
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
fn record(dir: &Path, err: &str) {
    // Collapse repeated failures into a single bounded status file that settings can display.
    let path = dir.join("last-error.txt");
    if std::fs::read(&path).ok().as_deref() != Some(err.as_bytes()) {
        let _ = atomic_write(&path, err.as_bytes());
    }
}
