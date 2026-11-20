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

pub type Geometry = Option<(i32, i32, i32)>;
type Layout = (Geometry, bool, crate::media::Reserve);
enum Update {
    Desktop(Layout),
    Stop,
}
static UPDATES: OnceLock<Mutex<Option<mpsc::Sender<Update>>>> = OnceLock::new();
static LAST_LAYOUT: Mutex<Option<Layout>> = Mutex::new(None);

fn menu_open() -> bool {
    LAST_LAYOUT
        .lock()
        .ok()
        .and_then(|state| *state)
        .is_some_and(|(_, open, _)| open)
}

pub fn layout_changed(geometry: Geometry, flyout_open: bool, reserve: crate::media::Reserve) {
    let next = (geometry, flyout_open, reserve);
    let Ok(mut last) = LAST_LAYOUT.lock() else {
        return;
    };
    if *last == Some(next) {
        return;
    }
    *last = Some(next);
    if let Some(Ok(bus)) = UPDATES.get().map(|v| v.lock())
        && let Some(tx) = bus.as_ref()
    {
        let _ = tx.send(Update::Desktop(next));
    }
}

pub fn check_profiles(settings: &Settings) -> Result<(), String> {
    for geometry in [None, Some((580, -108, 30))] {
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
        let geometry = None;
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
        let config = config.to_owned();
        let mut current = settings.clone();
        let worker = thread::spawn(move || {
            let mut layout = (geometry, false, crate::media::Reserve::default());
            let mut applied_layout = (geometry, layout.2);
            let mut pending = false;
            let mut observed = false;
            let mut styled = false;
            let stamp = || std::fs::metadata(&config).and_then(|v| v.modified()).ok();
            let mut loaded = stamp();
            // Preferences can be edited by a separate settings window. A single bounded wait
            // releases the thread between changes and leaves the optional redirector dormant.
            loop {
                match rx.recv_timeout(if pending {
                    Duration::from_millis(350)
                } else {
                    Duration::from_secs(3)
                }) {
                    Ok(Update::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Update::Desktop(next)) => {
                        layout = next;
                        observed = true;
                        pending = true;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                pending = false;
                if !observed || layout.1 {
                    continue;
                }
                let modified = stamp();
                let requested = (layout.0, layout.2);
                if styled && modified == loaded && applied_layout == requested {
                    continue;
                }
                let next = if modified == loaded {
                    current.clone()
                } else {
                    match Settings::load(&config) {
                        Ok(v) => v,
                        Err(e) => {
                            record(&directory, &e);
                            continue;
                        }
                    }
                };
                if !styled || next != current || applied_layout != requested {
                    if let Some(host) = shell.as_mut() {
                        match host.apply(&next, layout.0, layout.2) {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(e) => {
                                record(&directory, &e);
                                continue;
                            }
                        }
                    }
                    if (next.search_redirect != current.search_redirect
                        || (next.search_redirect && next.web_engine != current.web_engine))
                        && let Err(e) = redirect.configure(&next)
                    {
                        record(&directory, &e);
                        continue;
                    }
                    current = next;
                    applied_layout = requested;
                    styled = true;
                }
                loaded = modified;
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
