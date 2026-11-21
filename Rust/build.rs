use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn compiler() -> PathBuf {
    if let Some(path) = env::var_os("RC") {
        return path.into();
    }
    if let Some(path) = env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join("rc.exe"))
            .find(|path| path.is_file())
    }) {
        return path;
    }
    let host = env::var("HOST").unwrap_or_default();
    let arch = if host.starts_with("aarch64-") {
        "arm64"
    } else if host.starts_with("x86_64-") {
        "x64"
    } else {
        "x86"
    };
    let in_version = |dir: &Path| {
        let path = dir.join(arch).join("rc.exe");
        path.is_file().then_some(path)
    };
    if let Some(path) =
        env::var_os("WindowsSdkVerBinPath").and_then(|dir| in_version(Path::new(&dir)))
    {
        return path;
    }
    ["WindowsSdkDir", "ProgramFiles(x86)", "ProgramFiles"]
        .into_iter()
        .filter_map(|key| {
            let root = PathBuf::from(env::var_os(key)?);
            let bin = if key == "WindowsSdkDir" {
                root.join("bin")
            } else {
                root.join("Windows Kits/10/bin")
            };
            fs::read_dir(bin)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let version = entry
                        .file_name()
                        .to_str()?
                        .split('.')
                        .map(str::parse::<u32>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?;
                    Some((version, in_version(&entry.path())?))
                })
                .max_by(|a, b| a.0.cmp(&b.0))
                .map(|(_, path)| path)
        })
        .next()
        .unwrap_or_else(|| PathBuf::from("rc.exe"))
}

fn main() {
    if env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some(OsStr::new("windows")) {
        return;
    }
    println!("cargo:rerun-if-changed=assets/taskbar.ico");
    println!("cargo:rerun-if-changed=assets/taskbar.rc");
    println!("cargo:rerun-if-env-changed=RC");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("smarttaskbar.res");
    let status = Command::new(compiler())
        .args(["/nologo", "/fo"])
        .arg(&output)
        .arg("assets/taskbar.rc")
        .status()
        .expect("Install the Windows SDK resource compiler or set RC to its path.");
    assert!(status.success(), "Could not compile the SmartTaskbar icon.");
    println!(
        "cargo:rustc-link-arg-bin=smarttaskbar-rs={}",
        output.display()
    );
}
