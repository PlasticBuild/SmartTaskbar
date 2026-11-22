use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reveal {
    pub zone: Zone,
    pub outside_ms: u16,
    pub shared_ms: u16,
}

impl Default for Reveal {
    fn default() -> Self {
        Self {
            zone: Zone::Entire,
            outside_ms: 120,
            shared_ms: 220,
        }
    }
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
    #[serde(alias = "match_start_search_width")]
    pub match_start_width: bool,
    pub radius: u8,
    pub transparency: u8,
    pub icon_size: u8,
    pub hover_lift: u8,
    pub app_label_delay_ms: u16,
    pub click_depth: u8,
    pub animate: bool,
    pub alignment: Alignment,
    pub zone: Zone,
    pub outside_dwell_ms: u16,
    pub shared_dwell_ms: u16,
    pub monitor_reveal: BTreeMap<String, Reveal>,
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
            match_start_width: false,
            radius: 4,
            transparency: 65,
            icon_size: 100,
            hover_lift: 2,
            app_label_delay_ms: 250,
            click_depth: 1,
            animate: true,
            alignment: Alignment::Center,
            zone: Zone::Entire,
            outside_dwell_ms: Reveal::default().outside_ms,
            shared_dwell_ms: Reveal::default().shared_ms,
            monitor_reveal: BTreeMap::new(),
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

    pub fn same_shell_style(&self, other: &Self) -> bool {
        self.island == other.island
            && self.match_start_width == other.match_start_width
            && self.radius == other.radius
            && self.transparency == other.transparency
            && self.icon_size == other.icon_size
            && self.hover_lift == other.hover_lift
            && self.click_depth == other.click_depth
            && self.animate == other.animate
            && self.alignment == other.alignment
    }

    pub fn reveal(&self, monitor: Option<&str>) -> Reveal {
        monitor
            .and_then(|key| self.monitor_reveal.get(key).copied())
            .unwrap_or(Reveal {
                zone: self.zone,
                outside_ms: self.outside_dwell_ms,
                shared_ms: self.shared_dwell_ms,
            })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.radius > 24
            || self.transparency > 85
            || self.hover_lift > 8
            || self.app_label_delay_ms > 1500
            || self.click_depth > 5
            || !(Self::MIN_ICON_SIZE..=Self::MAX_ICON_SIZE).contains(&self.icon_size)
        {
            return Err("The appearance values are outside the supported range.".into());
        }
        if !PROVIDERS.contains(&self.web_engine.as_str()) {
            return Err(format!("Unknown search provider: {}", self.web_engine));
        }
        let valid = |r: Reveal| r.outside_ms <= 1000 && (150..=1000).contains(&r.shared_ms);
        if !valid(self.reveal(None))
            || self
                .monitor_reveal
                .iter()
                .any(|(id, r)| id.is_empty() || !valid(*r))
        {
            return Err("Taskbar reveal delays are outside the supported range.".into());
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
        assert!(!s.start_at_sign_in && !s.media_player && !s.search_redirect);
        assert!(!s.match_start_width);
        assert_eq!(s.media_side, MediaSide::Right);
        assert_eq!(s.icon_size, 100);
        assert_eq!(s.radius, 8);
        assert_eq!(s.reveal(Some("previously disconnected")), Reveal::default());
    }

    #[test]
    fn display_overrides_follow_identity_and_older_prefs_supply_defaults() {
        let mut settings: Settings = serde_json::from_str(r#"{"zone":"Island"}"#).unwrap();
        let custom = Reveal {
            zone: Zone::Center,
            outside_ms: 80,
            shared_ms: 400,
        };
        settings.monitor_reveal.insert("display-b".into(), custom);
        assert_eq!(settings.reveal(Some("display-b")), custom);
        assert_eq!(settings.reveal(Some("display-a")).zone, Zone::Island);
        assert_eq!(settings.reveal(None).shared_ms, 220);
        settings
            .monitor_reveal
            .get_mut("display-b")
            .unwrap()
            .shared_ms = 0;
        assert!(settings.validate().is_err());
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
            match_start_width: true,
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
