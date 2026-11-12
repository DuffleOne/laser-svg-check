//! The login agent that runs `laser-check watch`. Opening the app installs it,
//! pointing at wherever the app lives, so dragging the app into Applications
//! from the disk image is the whole install.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Config, home};

const LABEL: &str = "one.duffle.laser-check";

fn plist_path() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

fn domain() -> String {
    // SAFETY: getuid has no preconditions and can't fail.
    format!("gui/{}", unsafe { getuid() })
}

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Only a copy inside an installed .app gets the agent: not a dev build in
/// target/, and not the app still sitting on the mounted disk image.
fn installable(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    s.contains(".app/Contents/MacOS/") && !s.starts_with("/Volumes/") && !s.contains("/AppTranslocation/")
}

fn plist(exe: &Path) -> String {
    let log = home().join("Library/Logs/laser-check.log");
    // The version comment means an update rewrites the file, which restarts
    // the watcher on the new build.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- written by Laser Check {version} -->
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>watch</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
        exe = xml_escape(&exe.to_string_lossy()),
        log = xml_escape(&log.to_string_lossy()),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Called when the app opens: installs or updates the agent, or removes it if
/// `start_at_login` is off. Does nothing when the agent is already right.
pub fn sync(cfg: &Config) {
    if !cfg.start_at_login {
        if plist_path().exists() {
            let _ = uninstall();
        }
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if installable(&exe)
        && let Err(e) = install(&exe)
    {
        eprintln!("couldn't set up the watcher: {e}");
    }
}

pub fn install(exe: &Path) -> std::io::Result<()> {
    let path = plist_path();
    let want = plist(exe);
    if std::fs::read_to_string(&path).is_ok_and(|have| have == want) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))?;
    std::fs::create_dir_all(home().join("Library/Logs"))?;
    std::fs::write(&path, want)?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{LABEL}", domain())])
        .output();
    let out = Command::new("launchctl")
        .args(["bootstrap", &domain()])
        .arg(&path)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(())
}

pub fn uninstall() -> std::io::Result<()> {
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{LABEL}", domain())])
        .output();
    match std::fs::remove_file(plist_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_installed_apps_get_the_agent() {
        assert!(installable(Path::new(
            "/Applications/Laser Check.app/Contents/MacOS/laser-check"
        )));
        assert!(installable(Path::new(
            "/Users/x/Applications/Laser Check.app/Contents/MacOS/laser-check"
        )));
        assert!(!installable(Path::new(
            "/Volumes/Laser Check/Laser Check.app/Contents/MacOS/laser-check"
        )));
        assert!(!installable(Path::new(
            "/private/var/folders/x/AppTranslocation/ABC/d/Laser Check.app/Contents/MacOS/laser-check"
        )));
        assert!(!installable(Path::new("/Users/x/src/target/release/laser-check")));
    }
}
