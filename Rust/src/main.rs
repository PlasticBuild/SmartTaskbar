#![windows_subsystem = "windows"]

mod appearance;
mod desktop;
mod integrations;
mod join;
mod media;
mod migrate;
mod policy;
mod profiles;
mod settings;
mod updates;

use settings::Settings;
use std::os::windows::ffi::OsStrExt;
use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        FindWindowW, MB_ICONERROR, MB_OK, MessageBoxW, PostMessageW, WM_CLOSE,
    },
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain([0]).collect()
}

struct SingleInstance(HANDLE);
impl SingleInstance {
    fn lock(name: &str) -> Result<Self, String> {
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide(OsStr::new(name)).as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let guard = Self(handle);
        if already_exists {
            Err("SmartTaskbar is already running in this session.".into())
        } else {
            Ok(guard)
        }
    }
}
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Settings,
    Run,
    Stop,
    Pause,
    Resume,
    Inspect,
    Check,
    CheckUpdates,
    Import(PathBuf),
    Recover,
    PrepareUpdate,
    ConfigureStyle,
    VerifyStyle,
    SyncStartup,
    UninstallRedirect,
    Help,
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<(Action, Option<PathBuf>), String> {
    let mut it = args.into_iter();
    let mut action = None;
    let mut path = None;
    while let Some(arg) = it.next() {
        if arg == "--config" {
            if path.is_some() {
                return Err("Only one configuration file can be selected.".into());
            }
            path = Some(PathBuf::from(
                it.next().ok_or("Supply a filename after --config.")?,
            ));
            continue;
        }
        let next = match arg.to_str() {
            Some("--settings") => Action::Settings,
            Some("--run") => Action::Run,
            Some("--stop") => Action::Stop,
            Some("--pause") => Action::Pause,
            Some("--resume") => Action::Resume,
            Some("--import-appearance") => Action::Import(PathBuf::from(
                it.next()
                    .ok_or("Supply the exported appearance JSON file.")?,
            )),
            Some("--inspect") => Action::Inspect,
            Some("--check") => Action::Check,
            Some("--check-updates") => Action::CheckUpdates,
            Some("--recover-shell") => Action::Recover,
            Some("--prepare-update") => Action::PrepareUpdate,
            Some("--configure-style") => Action::ConfigureStyle,
            Some("--verify-style") => Action::VerifyStyle,
            Some("--sync-startup") => Action::SyncStartup,
            Some("--uninstall-redirect") => Action::UninstallRedirect,
            Some("--help" | "-h") => Action::Help,
            _ => return Err(format!("Unknown argument {}", arg.to_string_lossy())),
        };
        if action.replace(next).is_some() {
            return Err("Choose one action at a time.".into());
        }
    }
    Ok((action.unwrap_or(Action::Settings), path))
}

fn csharp_running() -> bool {
    !unsafe {
        FindWindowW(
            std::ptr::null(),
            wide(OsStr::new("SmartTaskbar Monitor")).as_ptr(),
        )
    }
    .is_null()
}

pub(crate) fn engine_running() -> bool {
    use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let handle = unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            0,
            wide(OsStr::new(r"Local\SmartTaskbarRust.Engine")).as_ptr(),
        )
    };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

fn pause_marker(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".paused");
    PathBuf::from(name)
}

pub(crate) fn is_paused(path: &Path) -> bool {
    pause_marker(path).is_file()
}

pub(crate) fn pause(path: &Path) -> Result<(), String> {
    let _lock = SingleInstance::lock(r"Local\SmartTaskbarRust.Pause")?;
    settings::atomic_write(&pause_marker(path), b"paused\n")?;
    // A starting controller may not have opened its stop event yet. It also
    // checks the marker before startup and after a preference directory change.
    if engine_running() {
        let _ = desktop::request_stop();
    }
    Ok(())
}

pub(crate) fn resume(path: &Path) -> Result<(), String> {
    let _lock = SingleInstance::lock(r"Local\SmartTaskbarRust.Pause")?;
    let marker = pause_marker(path);
    let paused = marker.is_file();
    if paused && engine_running() {
        return Err("SmartTaskbar is still restoring the Windows taskbar.".into());
    }
    if paused {
        std::fs::remove_file(&marker).map_err(|err| err.to_string())?;
    }
    if let Err(error) = start_engine(path) {
        if paused {
            settings::atomic_write(&marker, b"paused\n")?;
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn start_engine(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if engine_running() {
        return Ok(());
    }
    if csharp_running() {
        return Err("The C# taskbar is already controlling this desktop.".into());
    }
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .arg("--run")
        .arg("--config")
        .arg(path)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    for _ in 0..20 {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("The engine could not start ({status})."));
        }
        if engine_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Err("The taskbar engine did not report ready.".into())
}

pub(crate) fn apply_settings(path: &Path, old: &Settings, new: &Settings) -> Result<(), String> {
    let _lock = SingleInstance::lock(r"Local\SmartTaskbarRust.SettingsWrite")?;
    new.validate()?;
    if Settings::load(path)? != *old {
        return Err("Settings changed in another window. Reopen settings before applying.".into());
    }
    if new.start_at_sign_in != old.start_at_sign_in {
        integrations::sign_in(path, new.start_at_sign_in)?;
    }
    if let Err(err) = new.save(path) {
        if new.start_at_sign_in != old.start_at_sign_in {
            integrations::sign_in(path, old.start_at_sign_in)?;
        }
        return Err(err);
    }
    Ok(())
}

fn prepare_update(path: &Path) -> Result<(), String> {
    if csharp_running() {
        return Err("Close the previous SmartTaskbar controller before installing.".into());
    }
    // Older builds may still have their separate Start window open during an upgrade.
    let windows = || {
        let legacy_start = unsafe {
            FindWindowW(
                windows_sys::w!("SmartTaskbar.Rust.WebSearchPane"),
                std::ptr::null(),
            )
        };
        [appearance::window(), legacy_start]
    };
    if windows().iter().any(|hwnd| !hwnd.is_null()) {
        for hwnd in windows().into_iter().filter(|hwnd| !hwnd.is_null()) {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while windows().iter().any(|hwnd| !hwnd.is_null()) {
            if Instant::now() >= deadline {
                return Err("Close SmartTaskbar Settings before continuing.".into());
            }
            thread::sleep(Duration::from_millis(80));
        }
    }
    if engine_running() {
        let deadline = Instant::now() + Duration::from_secs(30);
        // The mutex can exist while the engine is still opening its stop event.
        let mut requested = false;
        while engine_running() {
            if !requested {
                requested = desktop::request_stop().is_ok();
            }
            if Instant::now() >= deadline {
                return Err("SmartTaskbar did not finish restoring the shell.".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    if path.with_file_name("shell-restore.json").exists() {
        return Err("The previous SmartTaskbar session still has a pending shell recovery.".into());
    }
    integrations::stop_packaged_style(path)
}

fn execute() -> Result<(), String> {
    let (action, path) = parse(env::args_os().skip(1))?;
    if action == Action::Recover {
        return desktop::recover();
    }
    if action == Action::Stop {
        return desktop::request_stop();
    }
    if action == Action::ConfigureStyle {
        return integrations::configure_packaged_style();
    }
    if action == Action::CheckUpdates {
        println!("{:?}", updates::check()?);
        return Ok(());
    }
    if action == Action::Help {
        println!(
            "smarttaskbar-rs [--settings | --run | --stop | --pause | --resume | --inspect | --check | --import-appearance PROFILE] [--config FILE]"
        );
        return Ok(());
    }
    let path = std::path::absolute(path.unwrap_or(settings::default_path()?))
        .map_err(|e| e.to_string())?;
    if action == Action::PrepareUpdate {
        return prepare_update(&path);
    }
    if action == Action::VerifyStyle {
        return integrations::verify_packaged_style(&path.with_file_name("Windhawk"));
    }
    if action == Action::UninstallRedirect {
        return integrations::search::uninstall(&path);
    }
    if action == Action::Pause {
        return pause(&path);
    }
    if action == Action::Resume {
        return resume(&path);
    }
    if let Action::Import(profile) = &action {
        let bytes = std::fs::read(profile).map_err(|e| format!("{}: {e}", profile.display()))?;
        let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        migrate::from_export(&value)?.save_new(&path)?;
        println!("Imported appearance into {}", path.display());
        return Ok(());
    }
    let settings = Settings::load(&path)?;
    match action {
        Action::Settings => appearance::show(&path),
        Action::SyncStartup => integrations::sign_in(&path, settings.start_at_sign_in),
        Action::Inspect => {
            println!("{}", desktop::inspect()?);
            Ok(())
        }
        Action::Check => {
            integrations::check_profiles(&settings)?;
            println!(
                "Configuration and four shell profiles validated. Desktop and sign-in settings untouched."
            );
            Ok(())
        }
        Action::Run => {
            if is_paused(&path) {
                return Ok(());
            }
            let _instance = SingleInstance::lock(r"Local\SmartTaskbarRust.Engine")?;
            if is_paused(&path) {
                return Ok(());
            }
            if csharp_running() {
                return Err("The C# SmartTaskbar engine is already managing this desktop. Close it before running the Rust engine.".into());
            }
            let _session = integrations::Session::start(&path, &settings)?;
            desktop::run(&settings, &path)
        }
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("{error}");
        if env::args_os().len() == 1 || env::args_os().any(|v| v == "--settings") {
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    wide(OsStr::new(&error)).as_ptr(),
                    wide(OsStr::new("SmartTaskbar")).as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_the_executable_opens_preferences() {
        assert_eq!(parse([]).unwrap(), (Action::Settings, None));
    }
    #[test]
    fn a_settings_file_can_contain_spaces() {
        let (act, path) =
            parse(["--config", "F:/Test Dir/preferences.json", "--check"].map(Into::into)).unwrap();
        assert_eq!(act, Action::Check);
        assert_eq!(path.unwrap(), Path::new("F:/Test Dir/preferences.json"));
    }
    #[test]
    fn refuses_ambiguous_actions() {
        assert!(parse(["--check", "--run"].map(Into::into)).is_err());
    }

    #[test]
    fn pause_marker_is_separate_for_each_preferences_file() {
        let a = Path::new("F:/Tests/prefs.json");
        assert_eq!(pause_marker(a), Path::new("F:/Tests/prefs.json.paused"));
        assert_ne!(
            pause_marker(a),
            pause_marker(Path::new("F:/Tests/other.json"))
        );
        assert_eq!(parse(["--pause"].map(Into::into)).unwrap().0, Action::Pause);
        assert_eq!(
            parse(["--resume"].map(Into::into)).unwrap().0,
            Action::Resume
        );
    }
}
