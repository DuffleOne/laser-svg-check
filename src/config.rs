use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Folders to watch for new SVGs. `~` is expanded.
    pub watch_dirs: Vec<String>,
    /// Warn about the same file (by content hash) again after this many days.
    pub rewarn_days: f64,
    /// Lines closer than this count as on top of each other.
    pub tolerance_mm: f64,
    /// Ignore files whose doubled-up length is shorter than this.
    pub min_overlap_mm: f64,
    /// macOS system sound played with the alert, or "" for none.
    pub alert_sound: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_dirs: vec!["~/Downloads".into()],
            rewarn_days: 3.0,
            tolerance_mm: 0.05,
            min_overlap_mm: 2.0,
            alert_sound: "Sosumi".into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub tolerance_mm: f64,
    pub min_overlap_mm: f64,
}

impl Config {
    pub fn path() -> PathBuf {
        home().join(".config/laser-check/config.toml")
    }

    /// Loads the config, writing the defaults out on first run so there's
    /// something to edit.
    pub fn load() -> Config {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                eprintln!("bad config {}: {e}; using defaults", path.display());
                Config::default()
            }),
            Err(_) => {
                let c = Config::default();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, toml::to_string_pretty(&c).unwrap_or_default());
                c
            }
        }
    }

    pub fn settings(&self) -> Settings {
        Settings {
            tolerance_mm: self.tolerance_mm,
            min_overlap_mm: self.min_overlap_mm,
        }
    }

    pub fn dirs(&self) -> Vec<PathBuf> {
        self.watch_dirs.iter().map(|d| expand(d)).collect()
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

pub fn state_dir() -> PathBuf {
    home().join("Library/Application Support/laser-check")
}
