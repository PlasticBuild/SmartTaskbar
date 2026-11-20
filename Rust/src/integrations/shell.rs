use super::Geometry;
use crate::{
    profiles::{self, IDS},
    settings::{Settings, atomic_write},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::WaitForSingleObject,
};

const VERSIONS: [&str; 4] = ["1.11.2", "1.8", "1.8", "1.3.1"];

#[derive(Clone, Serialize, Deserialize)]
struct Module {
    id: String,
    enabled: bool,
    settings: Value,
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    root: PathBuf,
    modules: Vec<Module>,
    host_was_running: bool,
}
pub struct Host {
    root: PathBuf,
    journal: PathBuf,
    original: Snapshot,
    current: Vec<Module>,
    changed: bool,
    started: bool,
    ready: bool,
}

fn run_cli(root: &Path, args: &[&str], input: Option<&Value>) -> Result<Value, String> {
    let mut child = Command::new(root.join("windhawk-cli.exe"))
        .arg("--app-root")
        .arg(root)
        .arg("--json")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| format!("Cannot start the styling CLI: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("Styling CLI has no output")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("Styling CLI has no error stream")?;
    let readout = thread::spawn(move || {
        let mut data = Vec::new();
        stdout.read_to_end(&mut data).map(|_| data)
    });
    let readerr = thread::spawn(move || {
        let mut data = String::new();
        stderr.read_to_string(&mut data).map(|_| data)
    });
    if let Some(profile) = input
        && let Some(mut stream) = child.stdin.take()
        && let Err(err) = stream.write_all(profile.to_string().as_bytes())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err.to_string());
    }
    let result = unsafe { WaitForSingleObject(child.as_raw_handle(), 12000) };
    let status = if result == WAIT_OBJECT_0 {
        child.wait().map_err(|e| e.to_string())?
    } else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(if result == WAIT_TIMEOUT {
            "The native styling CLI timed out.".into()
        } else {
            "Windows could not wait for the native styling CLI.".into()
        });
    };
    let output = readout
        .join()
        .map_err(|_| "Could not read styling output")?
        .map_err(|e| e.to_string())?;
    let err = readerr
        .join()
        .map_err(|_| "Could not read styling error")?
        .map_err(|e| e.to_string())?;
    let json: Result<Value, _> = serde_json::from_slice(&output);
    if !status.success() {
        let detail = json
            .as_ref()
            .ok()
            .and_then(|value| value["error"]["message"].as_str())
            .unwrap_or_else(|| err.trim());
        return Err(format!(
            "Styling CLI {} failed ({status}): {detail}",
            args.join(" "),
        ));
    }
    let mut json = json.map_err(|e| format!("Invalid native styling response: {e}"))?;
    json.get_mut("data")
        .map(Value::take)
        .ok_or_else(|| "Missing styling response data".into())
}

fn inventory(root: &Path) -> Result<Vec<Value>, String> {
    let list = run_cli(root, &["mod", "list"], None)?;
    let modules = list["mods"]
        .as_array()
        .ok_or("Malformed module inventory")?;
    if modules.len() != IDS.len() {
        return Err("Use the dedicated four-module SmartTaskbar styling profile.".into());
    }
    IDS.iter()
        .zip(VERSIONS)
        .map(|(id, version)| {
            modules
                .iter()
                .find(|v| v["id"] == *id && v["version"] == version)
                .cloned()
                .ok_or_else(|| format!("Expected {id} {version}"))
        })
        .collect()
}

pub fn configure_packaged_style() -> Result<(), String> {
    let root = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("Windhawk");
    let windows = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing Windows directory")?);
    let mut includes = vec![
        windows.join("explorer.exe"),
        windows.join("System32/ShellHost.exe"),
    ];
    for dir in fs::read_dir(windows.join("SystemApps")).map_err(|e| e.to_string())? {
        let dir = dir.map_err(|e| e.to_string())?.path();
        for name in [
            "StartMenuExperienceHost.exe",
            "SearchHost.exe",
            "SearchApp.exe",
            "ShellExperienceHost.exe",
        ] {
            let executable = dir.join(name);
            if executable.is_file() {
                includes.push(executable);
            }
        }
    }
    let includes = includes
        .iter()
        .map(|p| p.to_string_lossy().replace('/', "\\"))
        .collect::<Vec<_>>()
        .join("|");
    let ini = format!(
        "[Settings]\r\nLoggingVerbosity=0\r\nInclude={includes}\r\nExclude=*\r\nInjectIntoCriticalProcesses=0\r\nInjectIntoIncompatiblePrograms=0\r\nInjectIntoGames=0\r\n"
    );
    atomic_write(&root.join("AppData/Engine/settings.ini"), ini.as_bytes())?;
    verify_packaged_style(&root)?;
    // A normal Windhawk mod install writes its declared defaults. A portable
    // bundle has no installer for each mod, so create that restorable baseline.
    for id in IDS {
        let defaults = run_cli(&root, &["mod", "settings", "defaults", id], None)?;
        let settings = &defaults["settings"];
        if settings
            .as_object()
            .is_none_or(|settings| settings.is_empty())
        {
            return Err(format!("Missing defaults for {id}"));
        }
        run_cli(
            &root,
            &["mod", "settings", "write", id, "-"],
            Some(settings),
        )?;
    }
    Ok(())
}

pub fn verify_packaged_style(root: &Path) -> Result<(), String> {
    let windows = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing Windows directory")?);
    let settings = run_cli(root, &["app", "settings", "get"], None)?;
    if !scope_ok(&settings, &windows) {
        return Err(
            "The bundled styling host does not have the required Windows shell scope.".into(),
        );
    }
    inventory(root)?;
    Ok(())
}

pub fn stop_packaged_style(config: &Path) -> Result<(), String> {
    let Some(dir) = config.parent() else {
        return Ok(());
    };
    let mut root = dir.join("Windhawk");
    if !root.is_dir() {
        root = match fs::read_to_string(dir.join("style-root.txt")) {
            Ok(path) => PathBuf::from(path.trim()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
    }
    let path = root.join("windhawk.exe");
    if !root.is_absolute()
        || !process_paths("windhawk.exe")?.iter().any(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
        })
    {
        return Ok(());
    }
    let windows = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing Windows directory")?);
    if !scope_ok(
        &run_cli(&root, &["app", "settings", "get"], None)?,
        &windows,
    ) {
        return Err("The running styling host is outside SmartTaskbar's shell scope.".into());
    }
    inventory(&root)?;
    let status = Command::new(&path)
        .args(["-exit", "-wait"])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The previous styling host could not be stopped.".into())
    }
}

fn process_paths(name: &str) -> Result<Vec<PathBuf>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    let mut result = Vec::new();
    unsafe {
        let list = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if list == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut more = Process32FirstW(list, &mut entry);
        while more != 0 {
            let end = entry
                .szExeFile
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case(name) {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                let mut buf = vec![0u16; 32768];
                let mut len = buf.len() as u32;
                let ok = !handle.is_null()
                    && QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) != 0;
                if !handle.is_null() {
                    CloseHandle(handle);
                }
                if !ok {
                    CloseHandle(list);
                    return Err(format!("Cannot identify the running {name}."));
                }
                result.push(String::from_utf16_lossy(&buf[..len as usize]).into());
            }
            more = Process32NextW(list, &mut entry);
        }
        CloseHandle(list);
    }
    Ok(result)
}

fn scope_ok(value: &Value, windows: &Path) -> bool {
    let config = &value["settings"];
    let engine = &config["engine"];
    let shell = windows
        .join("explorer.exe")
        .to_string_lossy()
        .to_lowercase();
    let system = windows
        .join("System32/ShellHost.exe")
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    let dir = windows.join("SystemApps").to_string_lossy().to_lowercase() + "\\";
    let Some(paths) = engine["include"].as_array() else {
        return false;
    };
    let names: Vec<_> = paths
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .collect();
    config["safeMode"] == false
        && names.len() == paths.len()
        && names.contains(&shell)
        && names.iter().all(|p| {
            p == &shell
                || p == &system
                || (p.starts_with(&dir)
                    && !p[dir.len()..].split('\\').any(|s| s == "..")
                    && matches!(
                        Path::new(p).file_name().and_then(OsStr::to_str),
                        Some(
                            "startmenuexperiencehost.exe"
                                | "searchhost.exe"
                                | "searchapp.exe"
                                | "shellexperiencehost.exe"
                        )
                    ))
        })
        && engine["exclude"] == serde_json::json!(["*"])
        && [
            "injectIntoCriticalProcesses",
            "injectIntoIncompatiblePrograms",
            "injectIntoGames",
        ]
        .iter()
        .all(|k| engine[k] == false)
}

impl Host {
    pub fn open(dir: &Path) -> Result<Option<Self>, String> {
        let program = std::env::current_exe().map_err(|e| e.to_string())?;
        let beside = program.with_file_name("Windhawk");
        let chosen = if let Some(root) = std::env::var_os("SMARTTASKBAR_STYLE_ROOT") {
            Some(PathBuf::from(root))
        } else if beside.is_dir() {
            Some(beside)
        } else {
            let file = program.with_file_name("style-root.txt");
            match fs::read_to_string(file) {
                Ok(s) => Some(PathBuf::from(s.trim())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            }
        };
        let Some(root) = chosen else { return Ok(None) };
        if !root.is_absolute()
            || !root.join("windhawk-cli.exe").is_file()
            || !root.join("windhawk.exe").is_file()
        {
            return Err(
                "Select a prepared portable Windhawk folder for the native appearance profiles."
                    .into(),
            );
        }
        let windows =
            PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing Windows directory")?);
        if !scope_ok(
            &run_cli(&root, &["app", "settings", "get"], None)?,
            &windows,
        ) {
            return Err(
                "The native styling host is outside the configured Explorer and shell menu scope."
                    .into(),
            );
        }
        let host_path = root.join("windhawk.exe");
        let hosts = process_paths("windhawk.exe")?;
        if hosts.iter().any(|p| {
            !p.to_string_lossy()
                .eq_ignore_ascii_case(&host_path.to_string_lossy())
        }) {
            return Err("Another Windhawk installation is active.".into());
        }
        let mut current = Vec::new();
        for (id, found) in IDS.iter().zip(inventory(&root)?) {
            let settings =
                run_cli(&root, &["mod", "settings", "get", id], None)?["settings"].clone();
            if !settings.is_object() {
                return Err(format!("Missing settings for {id}"));
            }
            current.push(Module {
                id: id.to_string(),
                enabled: found["enabled"].as_bool().ok_or("Malformed enabled flag")?,
                settings,
            });
        }
        let journal = dir.join("shell-restore.json");
        let original = if journal.exists() {
            let saved: Snapshot =
                serde_json::from_slice(&fs::read(&journal).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if saved.root != root
                || saved.modules.len() != IDS.len()
                || saved
                    .modules
                    .iter()
                    .zip(IDS)
                    .any(|(m, id)| m.id != id || !m.settings.is_object())
            {
                return Err("The saved native profile belongs to another installation.".into());
            }
            saved
        } else {
            Snapshot {
                root: root.clone(),
                modules: current.clone(),
                host_was_running: !hosts.is_empty(),
            }
        };
        atomic_write(
            &journal,
            &serde_json::to_vec(&original).map_err(|e| e.to_string())?,
        )?;
        let started = !original.host_was_running && !hosts.is_empty();
        Ok(Some(Self {
            root,
            journal,
            original,
            current,
            changed: true,
            started,
            ready: !hosts.is_empty(),
        }))
    }
    fn ensure_running(&mut self) -> Result<(), String> {
        if self.ready {
            return Ok(());
        }
        let path = self.root.join("windhawk.exe");
        Command::new(&path)
            .arg("-tray-only")
            .current_dir(&self.root)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        self.started = true;
        let until = Instant::now() + Duration::from_secs(3);
        while !process_paths("windhawk.exe")?.iter().any(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
        }) {
            if Instant::now() >= until {
                return Err("The native styling host did not start.".into());
            }
            thread::sleep(Duration::from_millis(80));
        }
        self.ready = true;
        Ok(())
    }
    fn read_current(&self) -> Result<Vec<Module>, String> {
        let list = run_cli(&self.root, &["mod", "list"], None)?;
        let inventory = list["mods"]
            .as_array()
            .ok_or("Cannot verify the native module inventory.")?;
        IDS.iter()
            .map(|id| {
                let enabled = inventory
                    .iter()
                    .find(|m| m["id"] == *id)
                    .and_then(|m| m["enabled"].as_bool())
                    .ok_or_else(|| format!("Cannot read the enabled state of {id}"))?;
                let settings =
                    run_cli(&self.root, &["mod", "settings", "get", id], None)?["settings"].clone();
                if !settings.is_object() {
                    return Err(format!("Missing native profile for {id}"));
                }
                Ok(Module {
                    id: id.to_string(),
                    enabled,
                    settings,
                })
            })
            .collect()
    }
    fn set(&mut self, index: usize, requested: &Value, enabled: bool) -> Result<(), String> {
        let m = &mut self.current[index];
        if m.settings != *requested {
            run_cli(
                &self.root,
                &["mod", "settings", "write", &m.id, "-"],
                Some(requested),
            )?;
            m.settings = requested.clone();
        }
        if m.enabled != enabled {
            run_cli(
                &self.root,
                &["mod", if enabled { "enable" } else { "disable" }, &m.id],
                None,
            )?;
            m.enabled = enabled;
        }
        Ok(())
    }
    pub fn apply(
        &mut self,
        settings: &Settings,
        geometry: Geometry,
        reserve: crate::media::Reserve,
    ) -> Result<bool, String> {
        if super::menu_open() {
            return Ok(false);
        }
        self.ensure_running()?;
        let profiles = profiles::build_with_reserve(settings, geometry, reserve)?;
        for (index, item) in profiles.iter().enumerate() {
            if super::menu_open() {
                return Ok(false);
            }
            self.set(index, item, true)?;
        }
        Ok(true)
    }
    pub fn restore(&mut self) -> Result<(), String> {
        if !self.changed {
            return Ok(());
        }
        self.current = self.read_current()?;
        for (index, m) in self.original.modules.clone().into_iter().enumerate() {
            self.set(index, &m.settings, m.enabled)?;
        }
        if self
            .read_current()?
            .iter()
            .zip(&self.original.modules)
            .any(|(actual, old)| actual.enabled != old.enabled || actual.settings != old.settings)
        {
            return Err("The original shell profile has not been fully restored. Its recovery file was retained.".into());
        }
        if self.started && !self.original.host_was_running {
            let path = self.root.join("windhawk.exe");
            if process_paths("windhawk.exe")?.iter().any(|p| {
                p.to_string_lossy()
                    .eq_ignore_ascii_case(&path.to_string_lossy())
            }) {
                let status = Command::new(path)
                    .args(["-exit", "-wait"])
                    .creation_flags(0x08000000)
                    .status()
                    .map_err(|e| e.to_string())?;
                if !status.success() {
                    return Err("Could not close this portable styling host.".into());
                }
            }
        }
        fs::remove_file(&self.journal).map_err(|e| e.to_string())?;
        self.changed = false;
        Ok(())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
#[path = "shell_rollback_tests.rs"]
mod shell_rollback_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Value {
        serde_json::json!({"settings":{"safeMode":false,"engine":{
            "exclude":["*"], "include":[r"C:\Windows\explorer.exe", r"C:\Windows\SystemApps\SearchPackage\SearchHost.exe"],
            "injectIntoCriticalProcesses":false, "injectIntoIncompatiblePrograms":false, "injectIntoGames":false
        }}})
    }
    #[test]
    fn accepts_the_dedicated_shell_host() {
        assert!(scope_ok(&config(), Path::new(r"C:\Windows")));
    }
    #[test]
    fn leaves_foreign_apps_and_unexpected_installs_alone() {
        let mut v = config();
        v["settings"]["engine"]["include"][1] = serde_json::json!(r"D:\Apps\SearchHost.exe");
        assert!(!scope_ok(&v, Path::new(r"C:\Windows")));
        v = config();
        v["settings"]["engine"]["exclude"] = serde_json::json!([]);
        assert!(!scope_ok(&v, Path::new(r"C:\Windows")));
    }
}
