use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Alignment {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Zone {
    #[default]
    Entire,
    Center,
    Island,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MediaSide {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub island: bool,
    pub radius: u8,
    pub transparency: u8,
    pub icon_size: u8,
    pub hover_lift: u8,
    pub click_depth: u8,
    pub animate: bool,
    pub alignment: Alignment,
    pub zone: Zone,
    pub start_at_sign_in: bool,
    pub search_redirect: bool,
    pub web_engine: String,
    pub media_player: bool,
    pub media_side: MediaSide,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            island: true,
            radius: 4,
            transparency: 65,
            icon_size: 100,
            hover_lift: 2,
            click_depth: 1,
            animate: true,
            alignment: Alignment::Center,
            zone: Zone::Entire,
            start_at_sign_in: false,
            search_redirect: false,
            web_engine: "Google".into(),
            media_player: false,
            media_side: MediaSide::Right,
        }
    }
}

pub const PROVIDERS: &[&str] = &[
    "Google",
    "DuckDuckGo",
    "Brave",
    "Bing",
    "Ecosia",
    "Startpage",
    "Yahoo",
];

impl Settings {
    pub const MIN_ICON_SIZE: u8 = 70;
    pub const MAX_ICON_SIZE: u8 = 125;

    pub fn validate(&self) -> Result<(), String> {
        if self.radius > 24
            || self.transparency > 85
            || self.hover_lift > 8
            || self.click_depth > 5
            || !(Self::MIN_ICON_SIZE..=Self::MAX_ICON_SIZE).contains(&self.icon_size)
        {
            return Err("The appearance values are outside the supported range.".into());
        }
        if !PROVIDERS.contains(&self.web_engine.as_str()) {
            return Err(format!("Unknown search provider: {}", self.web_engine));
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(bytes) => {
                let settings: Settings = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                settings.validate()?;
                Ok(settings)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        atomic_write(
            path,
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
    pub fn save_new(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        write_document(
            path,
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
            false,
        )
    }
}

pub fn default_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.with_file_name("settings.json"))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_document(path, bytes, true)
}

fn write_document(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("Missing settings directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file;
    let pending = loop {
        let candidate = parent.join(format!(
            ".smarttaskbar-{}-{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(f) => {
                file = f;
                break candidate;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
        let flags = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        if unsafe { MoveFileExW(wide(&pending).as_ptr(), wide(path).as_ptr(), flags) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result.map_err(|e: std::io::Error| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_features_default_to_off() {
        let s: Settings = serde_json::from_str(r#"{"radius":8}"#).unwrap();
        assert!(!s.search_redirect && !s.start_at_sign_in && !s.media_player);
        assert_eq!(s.media_side, MediaSide::Right);
        assert_eq!(s.icon_size, 100);
        assert_eq!(s.radius, 8);
    }
    #[test]
    fn invalid_values_are_rejected_before_writing() {
        let s = Settings {
            radius: 255,
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
    #[test]
    fn writes_replace_a_readable_document() {
        let folder =
            std::env::temp_dir().join(format!("stbr-settings-test-{}", std::process::id()));
        let p = folder.join("preferences.json");
        let s = Settings::default();
        s.save(&p).unwrap();
        let next = Settings {
            alignment: Alignment::Right,
            animate: false,
            media_player: true,
            media_side: MediaSide::Left,
            ..s
        };
        next.save(&p).unwrap();
        assert_eq!(Settings::load(&p).unwrap(), next);
        assert!(Settings::default().save_new(&p).is_err());
        assert_eq!(Settings::load(&p).unwrap(), next);
        fs::remove_file(p).unwrap();
        fs::remove_dir(folder).unwrap();
    }
}
