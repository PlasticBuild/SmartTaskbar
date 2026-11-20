use super::{lease::Snapshot, native};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
    System::{Console::*, Diagnostics::ToolHelp::*, Threading::CREATE_NO_WINDOW},
};

pub(super) struct Monitor {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<String>,
}
impl Monitor {
    pub fn start() -> Result<Self, String> {
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("--recover-shell")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot start the taskbar recovery process: {e}"))?;
        let input = child.stdin.take();
        let output = child
            .stdout
            .take()
            .ok_or("Cannot open the recovery response pipe.")?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let this = Self {
            child,
            input,
            replies,
        };
        this.ack()?;
        Ok(this)
    }
    fn ack(&self) -> Result<(), String> {
        let reply = self
            .replies
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "The taskbar recovery process is unavailable.".to_string())?;
        if reply == "ok" { Ok(()) } else { Err(reply) }
    }
    fn send(&mut self, line: &str) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("The recovery pipe is closed.")?;
        writeln!(input, "{line}")
            .and_then(|_| input.flush())
            .map_err(|e| e.to_string())?;
        self.ack()
    }
    pub fn watch(&mut self, bar: native::Bar) -> Result<(), String> {
        self.send(&format!("watch {} {}", bar.hwnd as usize, bar.pid))
    }
    pub fn forget(&mut self, h: usize) -> Result<(), String> {
        self.send(&format!("forget {h}"))
    }
    pub fn refresh(&mut self, h: usize) -> Result<(), String> {
        self.send(&format!("outline {h}"))
    }
    pub fn check(&mut self) -> Result<(), String> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            _ => Err("The recovery process stopped. Restoring the taskbars.".into()),
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        drop(self.input.take()); /* EOF triggers restoration of anything still claimed. */
    }
}

fn parent_id() -> Option<u32> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    let mut present = unsafe { Process32FirstW(snap, &mut entry) };
    while present != 0 {
        if entry.th32ProcessID == std::process::id() {
            found = Some(entry.th32ParentProcessID);
            break;
        }
        present = unsafe { Process32NextW(snap, &mut entry) };
    }
    unsafe { CloseHandle(snap) };
    found
}

pub(super) fn recover() -> Result<(), String> {
    let owner = parent_id().ok_or("No recovery parent.")?;
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok());
    let parent = native::process_path(owner).and_then(|p| std::fs::canonicalize(p).ok());
    if exe.is_none()
        || parent != exe
        || unsafe { GetFileType(GetStdHandle(STD_INPUT_HANDLE)) } != FILE_TYPE_PIPE
    {
        return Err("The recovery monitor requires a private engine pipe.".into());
    }
    let _dpi = super::DpiScope::enter();
    let mut snapshots = HashMap::<usize, Snapshot>::new();
    let mut output = std::io::stdout().lock();
    writeln!(output, "ok")
        .and_then(|_| output.flush())
        .map_err(|e| e.to_string())?;
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let mut fields = line.split_ascii_whitespace();
        let verb = fields.next();
        let handle = fields.next().and_then(|s| s.parse::<usize>().ok());
        let result = match (verb, handle) {
            (Some("watch"), Some(h)) => {
                let pid = fields.next().and_then(|s| s.parse::<u32>().ok());
                match pid
                    .filter(|pid| *pid == native::shell_pid())
                    .and_then(|pid| native::bar_info(h as HWND, pid))
                {
                    Some(bar) => Snapshot::capture(bar, owner).map(|s| {
                        snapshots.insert(h, s);
                    }),
                    None => Err("Explorer changed while opening the recovery record.".to_string()),
                }
            }
            (Some("forget"), Some(h)) => {
                snapshots.remove(&h);
                Ok(())
            }
            (Some("outline"), Some(h)) => match snapshots.get_mut(&h) {
                Some(s) => s.refresh_outline(),
                None => Err("No taskbar recovery record.".into()),
            },
            _ => Err("Unknown recovery record.".into()),
        };
        if writeln!(output, "{}", result.err().as_deref().unwrap_or("ok"))
            .and_then(|_| output.flush())
            .is_err()
        {
            break;
        }
    }
    for snap in snapshots.values() {
        snap.restore()
    }
    Ok(())
}
