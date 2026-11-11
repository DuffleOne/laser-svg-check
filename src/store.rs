//! Remembers files by content hash so the watcher doesn't nag about the same
//! drawing over and over, or about the `-fixed` copies it writes itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::state_dir;

#[derive(Serialize, Deserialize, Default)]
pub struct Seen {
    entries: HashMap<String, Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub path: String,
    /// When we last checked or warned, unix seconds.
    pub at: u64,
    pub clean: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn file() -> PathBuf {
    state_dir().join("seen.json")
}

pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

impl Seen {
    pub fn load() -> Seen {
        std::fs::read(file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self) {
        // Forget anything not touched in 90 days.
        let cutoff = now().saturating_sub(90 * 86400);
        self.entries.retain(|_, e| e.at >= cutoff);
        let _ = std::fs::create_dir_all(state_dir());
        let tmp = file().with_extension("tmp");
        if let Ok(json) = serde_json::to_vec_pretty(self)
            && std::fs::write(&tmp, json).is_ok()
        {
            let _ = std::fs::rename(&tmp, file());
        }
    }

    /// Whether this content is worth looking at: never seen, or warned about
    /// more than `rewarn_days` ago. Clean files are never looked at again.
    pub fn due(&self, hash: &str, rewarn_days: f64) -> bool {
        match self.entries.get(hash) {
            None => true,
            Some(e) if e.clean => false,
            Some(e) => now().saturating_sub(e.at) as f64 >= rewarn_days * 86400.0,
        }
    }

    pub fn record(&mut self, hash: String, path: &Path, clean: bool) {
        self.entries.insert(
            hash,
            Entry {
                path: path.display().to_string(),
                at: now(),
                clean,
            },
        );
    }
}

/// Marks a file we wrote ourselves (a fixed copy) as clean, so the watcher
/// skips it without even parsing it.
pub fn mark_clean(path: &Path) {
    if let Ok(bytes) = std::fs::read(path) {
        let mut seen = Seen::load();
        seen.record(hash(&bytes), path, true);
        seen.save();
    }
}
