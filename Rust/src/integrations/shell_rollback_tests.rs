use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

// Compile a disposable CLI so the test also exercises the real process status,
// stdin JSON, independent reads and on-disk side effects of a failed child.
const FIXTURE: &str = r##"
use std::{env, fs, io::{self, Read}, path::Path, process};
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    assert_eq!(args[0], "--app-root");
    let root=Path::new(&args[1]);
    assert!(root.join("fixture-only").is_file());
    assert_eq!((&args[2],&args[3]),(&"--json".to_owned(),&"mod".to_owned()));
    let names=fs::read_to_string(root.join("ids")).unwrap();
    if args[4]=="list" {
        let entries=names.lines().map(|id|{
            let enabled=fs::read_to_string(root.join(format!("{id}.enabled"))).unwrap();
            format!(r#"{{"id":"{id}","enabled":{enabled}}}"#)
        }).collect::<Vec<_>>().join(",");
        println!(r#"{{"data":{{"mods":[{entries}]}}}}"#);
        return;
    }
    let id=if args[4]=="settings" {&args[6]} else {&args[5]};
    assert!(names.lines().any(|n| n==id));
    let profile=root.join(format!("{id}.json"));
    if args[4]=="settings" && args[5]=="get" {
        let saved=fs::read_to_string(profile).unwrap();
        println!(r#"{{"data":{{"settings":{saved}}}}}"#); return;
    }
    if args[4]=="settings" && args[5]=="write" {
        assert_eq!(args[7],"-");
        let mut input=String::new(); io::stdin().read_to_string(&mut input).unwrap();
        fs::write(profile,input).unwrap();
    } else {
        assert!(matches!(args[4].as_str(),"enable"|"disable"));
        fs::write(root.join(format!("{id}.enabled")),if args[4]=="enable" {"true"} else {"false"}).unwrap();
    }
    let fail=root.join("fail-after-commit");
    if fail.exists() && fs::read_to_string(&fail).unwrap()==*id {
        fs::remove_file(fail).unwrap(); eprintln!("fixture failure after commit"); process::exit(19);
    }
    println!(r#"{{"data":{{}}}}"#);
}
"##;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn committed_but_failed_cli_writes_restore_the_original_snapshot_and_can_retry() {
    let path = std::env::temp_dir().join(format!(
        "smarttaskbar-rollback-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let _scratch = Scratch(path.clone());
    let source = path.join("main.rs");
    fs::write(&source, FIXTURE).unwrap();
    let build = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition=2024", "--crate-name", "fixture_cli"])
        .arg(&source)
        .arg("-o")
        .arg(path.join("windhawk-cli.exe"))
        .current_dir(&path)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    fs::write(path.join("fixture-only"), b"").unwrap();
    fs::write(path.join("ids"), IDS.join("\n")).unwrap();
    let originals: Vec<_> = IDS
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let m = Module {
                id: id.into(),
                settings: serde_json::json!({"original":id,"custom":i}),
                enabled: i % 2 == 1,
            };
            fs::write(path.join(format!("{id}.json")), m.settings.to_string()).unwrap();
            fs::write(path.join(format!("{id}.enabled")), m.enabled.to_string()).unwrap();
            m
        })
        .collect();
    let journal = path.join("shell-restore.json");
    let original = Snapshot {
        root: path.clone(),
        modules: originals.clone(),
        host_was_running: true,
    };
    let saved = serde_json::to_vec(&original).unwrap();
    fs::write(&journal, &saved).unwrap();
    // A prepared fake host exercises the production write/restore path without
    // discovering processes or starting a styling host on the interactive desktop.
    let mut host = Host {
        root: path.clone(),
        journal: journal.clone(),
        original,
        current: originals.clone(),
        changed: true,
        started: false,
        ready: true,
    };
    let second = IDS[1];
    fs::write(path.join("fail-after-commit"), second).unwrap();
    assert!(
        host.apply(&Settings::default(), None)
            .unwrap_err()
            .contains("fixture failure after commit")
    );
    let live = host.read_current().unwrap();
    assert_ne!(live[1].settings, originals[1].settings);
    assert!(live[0].enabled);
    assert_eq!(fs::read(&journal).unwrap(), saved);
    fs::write(path.join("fail-after-commit"), IDS[0]).unwrap();
    assert!(
        host.restore()
            .unwrap_err()
            .contains("fixture failure after commit")
    );
    assert_eq!(fs::read(&journal).unwrap(), saved);
    host.restore().unwrap();
    assert!(!journal.exists());
    for (m, before) in host.read_current().unwrap().iter().zip(&originals) {
        assert_eq!(
            (&m.id, m.enabled, &m.settings),
            (&before.id, before.enabled, &before.settings)
        );
    }
    host.restore().unwrap();
}
