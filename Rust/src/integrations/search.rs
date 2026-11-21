use crate::settings::{Settings, atomic_write};
use sha2::{Digest, Sha256};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use windows_sys::Win32::System::JobObjects::*;

const CHECKSUM: &str = "f3c4f9de4553e9f88b71ce189d3d1234fa560d247443d4c2d417e3b6461cd36a";
const HIDDEN: u32 = 0x08000000;

fn folder(base: &Path) -> PathBuf {
    base.join("optional").join("search")
}

pub(crate) fn installed(config: &Path) -> bool {
    config
        .parent()
        .is_some_and(|base| folder(base).join("MSEdgeRedirect.exe").is_file())
}

pub(crate) fn uninstall(config: &Path) -> Result<(), String> {
    if !installed(config) {
        return Ok(());
    }
    let base = config.parent().ok_or("Settings directory is missing.")?;
    let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
    let folder = folder(&base);
    if fs::canonicalize(&folder).map_err(|e| e.to_string())? != folder {
        return Err("The MSEdgeRedirect folder is outside this installation.".into());
    }
    let exe = folder.join("MSEdgeRedirect.exe");
    if fs::canonicalize(&exe).map_err(|e| e.to_string())? != exe {
        return Err("The helper file does not belong to this installation.".into());
    }
    verify(&exe)?;
    let old = Settings::load(config)?;
    if old.search_redirect {
        let mut next = old.clone();
        next.search_redirect = false;
        crate::apply_settings(config, &old, &next)?;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match fs::remove_file(&exe) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e)
                if matches!(e.raw_os_error(), Some(5 | 32))
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Could not remove MSEdgeRedirect: {e}")),
        }
    }
    for file in [
        "LICENSE-MSEdgeRedirect.txt",
        "MSEdgeRedirect-0.8.0.0-source.tar.gz",
    ] {
        match fs::remove_file(folder.join(file)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not remove the MSEdgeRedirect files: {e}")),
        }
    }
    Ok(())
}

pub struct Redirect {
    folder: PathBuf,
    child: Option<Child>,
    job: Option<OwnedHandle>,
}
impl Redirect {
    pub fn new(base: &Path) -> Self {
        Self {
            folder: folder(base),
            child: None,
            job: None,
        }
    }
    fn stop(&mut self) {
        drop(self.job.take());
        if let Some(mut child) = self.child.take() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
    pub fn configure(&mut self, settings: &Settings) -> Result<(), String> {
        self.stop();
        if !settings.search_redirect || !self.folder.join("MSEdgeRedirect.exe").is_file() {
            return Ok(());
        }
        settings.validate()?;
        let sys = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing Windows directory")?)
            .join("System32");
        let reg = sys.join("reg.exe");
        for key in [
            r"HKCU\Software\MSEdgeRedirect",
            r"HKLM\Software\MSEdgeRedirect",
        ] {
            if Command::new(&reg)
                .args(["query", key])
                .creation_flags(HIDDEN)
                .output()
                .is_ok_and(|s| s.status.success())
            {
                return Err("Windows already has a separate MSEdgeRedirect installation.".into());
            }
        }
        for protocol in ["http", "https"] {
            let key = format!(
                r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{protocol}\UserChoice"
            );
            if let Ok(output) = Command::new(&reg)
                .args(["query", &key, "/v", "ProgId"])
                .creation_flags(HIDDEN)
                .output()
                && String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains("msedge")
            {
                return Err(
                    "Select your preferred default browser before turning on search redirection."
                        .into(),
                );
            }
        }
        let exe = self.folder.join("MSEdgeRedirect.exe");
        verify(&exe)?;
        let convert = if settings.web_engine == "Bing" { 0 } else { 1 };
        let ini = format!(
            "[Settings]\r\nNoBing={convert}\r\nSearch={0}\r\nNoImgs={convert}\r\nImages={0}\r\nNoUpdates=1\r\n",
            settings.web_engine
        );
        atomic_write(
            &self.folder.join("MSEdgeRedirect/Settings.ini"),
            ini.as_bytes(),
        )?;
        let job = helper_job()?;
        let mut child = Command::new(&exe)
            .args(["/portable", "/hide"])
            .current_dir(&self.folder)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(HIDDEN)
            .spawn()
            .map_err(|e| e.to_string())?;
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
            let err = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Could not bind the optional helper to this session: {err}"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Search helper exited at startup ({status})."));
        }
        self.child = Some(child);
        self.job = Some(job);
        Ok(())
    }
}
impl Drop for Redirect {
    fn drop(&mut self) {
        self.stop();
    }
}
fn helper_job() -> Result<OwnedHandle, String> {
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(job)
}
fn verify(path: &Path) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 32768];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if format!("{:x}", hash.finalize()) != CHECKSUM {
        return Err("The optional helper did not match the published SHA-256 checksum.".into());
    }
    Ok(())
}
