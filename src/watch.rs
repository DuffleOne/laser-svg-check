//! The background part. No UI here: it sleeps on FSEvents and only starts
//! the (much heavier) gpui process when there's something to show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

use crate::config::Config;
use crate::store::{Seen, hash};

/// How long a file has to sit unchanged before we read it, so we don't catch
/// AirDrop or a browser halfway through writing.
const SETTLE: Duration = Duration::from_millis(1500);

pub fn run() -> Result<()> {
    let cfg = Config::load();
    let (tx, rx) = mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
        }
    })?;
    let mut pending: HashMap<PathBuf, (Instant, u64)> = HashMap::new();
    for dir in cfg.dirs() {
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => eprintln!("watching {}", dir.display()),
            Err(e) => eprintln!("can't watch {}: {e}", dir.display()),
        }
        // Catch anything that landed in the last day while we weren't running.
        for p in recent_svgs(&dir, Duration::from_secs(86400)) {
            pending.insert(p, (Instant::now() - SETTLE, u64::MAX));
        }
    }

    loop {
        let wait = if pending.is_empty() {
            Duration::from_secs(3600)
        } else {
            Duration::from_millis(400)
        };
        match rx.recv_timeout(wait) {
            Ok(p) => {
                if crate::is_svg(&p) {
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    pending.insert(p, (Instant::now(), size));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let mut ready = vec![];
        pending.retain(|p, (t, size)| {
            if t.elapsed() < SETTLE {
                return true;
            }
            let Ok(meta) = std::fs::metadata(p) else {
                return false;
            };
            if meta.len() != *size {
                // Still growing; look again later.
                *t = Instant::now();
                *size = meta.len();
                return true;
            }
            ready.push(p.clone());
            false
        });
        if ready.is_empty() {
            continue;
        }

        // Parsing a big drawing takes tens of MB that malloc then keeps, so
        // each batch is checked in a short-lived child and the watcher stays
        // at a few MB.
        let exe = std::env::current_exe().unwrap_or_default();
        match std::process::Command::new(exe).arg("scan").args(&ready).status() {
            Ok(st) if !st.success() => eprintln!("scan exited with {st}"),
            Err(e) => eprintln!("couldn't run scan: {e}"),
            _ => {}
        }
    }
    Ok(())
}

/// Checks a batch of new files and pops one alert for the ones with problems.
/// Runs as its own process (`laser-check scan ...`), started by the watcher.
pub fn scan(paths: &[PathBuf]) {
    // Re-read the config each time so threshold tweaks apply without a restart.
    let cfg = Config::load();
    let mut seen = Seen::load();
    let alert: Vec<PathBuf> = paths.iter().filter_map(|p| check(p, &cfg, &mut seen)).collect();
    seen.save();
    if !alert.is_empty() {
        spawn_alert(&alert);
    }
}

fn check(p: &Path, cfg: &Config, seen: &mut Seen) -> Option<PathBuf> {
    let bytes = std::fs::read(p).ok()?;
    let h = hash(&bytes);
    if !seen.due(&h, cfg.rewarn_days) {
        return None;
    }
    let s = cfg.settings();
    let doc = crate::geom::parse(String::from_utf8_lossy(&bytes).into_owned()).ok()?;
    let an = crate::analyze::run(doc, &s);
    let bad = an.has_problems(&s);
    eprintln!("{}: {:.1} mm doubled", p.display(), an.removed_mm);
    seen.record(h, p, !bad);
    bad.then(|| p.to_path_buf())
}

fn recent_svgs(dir: &Path, within: Duration) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let cutoff = SystemTime::now() - within;
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| crate::is_svg(p))
        .filter(|p| {
            std::fs::metadata(p).is_ok_and(|m| {
                // AirDrop keeps the sender's mtime, so look at when it arrived too.
                let t = [m.modified().ok(), m.created().ok()].into_iter().flatten().max();
                t.is_some_and(|t| t >= cutoff)
            })
        })
        .collect()
}

/// Opens the alert window. Goes through LaunchServices when we're inside the
/// app bundle so the window process isn't a child of this one.
pub fn spawn_alert(paths: &[PathBuf]) {
    let exe = std::env::current_exe().unwrap_or_default();
    let bundle = exe
        .ancestors()
        .find(|a| a.extension().is_some_and(|e| e == "app"));
    let res = match bundle {
        Some(app) => std::process::Command::new("/usr/bin/open")
            .arg("-n")
            .arg("-a")
            .arg(app)
            .arg("--args")
            .arg("alert")
            .args(paths)
            .spawn(),
        None => std::process::Command::new(&exe).arg("alert").args(paths).spawn(),
    };
    if let Err(e) = res {
        eprintln!("couldn't open alert: {e}");
    }
}
