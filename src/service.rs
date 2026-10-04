//! The systemd user service is the standard runtime mode: entry points ensure
//! it is installed, enabled, and started, so opening xrs once keeps the proxy
//! running in the background. Systems without a usable systemd user session
//! fall back to managing the Xray process directly.

use crate::xray::XrayRunner;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const UNIT: &str = "xrs.service";

fn unit_path() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/systemd/user").join(UNIT)
}

/// Kept byte-identical with what `xrs service install` has always written so
/// existing units on disk are recognized as current. `run` is the hidden
/// foreground daemon entry point.
fn unit_content(bin: &Path) -> String {
    format!(
        "[Unit]\nDescription=xrs - an xray cli first ultra fast lightweight client\nAfter=network.target\n\n[Service]\nType=simple\nExecStart=\"{}\" run\nRestart=on-failure\nRestartSec=3s\n\n[Install]\nWantedBy=default.target\n",
        bin.display()
    )
}

fn current_binary() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_else(|_| home.join(".local/bin/xrs"))
}

/// `Some(success)` when systemctl ran, `None` when there is no systemctl at
/// all. A missing user bus still reports non-zero, so callers must treat any
/// failure as "systemd unusable" and fall back.
fn systemctl(args: &[&str]) -> Option<bool> {
    Command::new("systemctl")
        .args(["--user"])
        .args(args)
        .status()
        .ok()
        .map(|s| s.success())
}

pub fn is_active() -> bool {
    systemctl(&["is-active", UNIT]) == Some(true)
}

/// Writes the unit file (and reloads the daemon). Rewriting on every install
/// is intentional: it moves the unit to the binary of the running xrs, which
/// is how upgrades propagate.
pub fn install_unit() -> bool {
    let path = unit_path();
    let Some(parent) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return false;
    }
    if std::fs::write(&path, unit_content(&current_binary())).is_err() {
        return false;
    }
    let _ = systemctl(&["daemon-reload"]);
    true
}

fn unit_is_current() -> bool {
    std::fs::read_to_string(unit_path()).is_ok_and(|c| c == unit_content(&current_binary()))
}

/// Xray pid once the freshly started unit has spawned the core.
pub fn wait_running_pid() -> Option<u32> {
    for _ in 0..30 {
        if let Some(pid) = XrayRunner::get_running_pid() {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// Installs, enables, and starts the user service, then confirms the core is
/// up. False when systemd is unusable; callers then manage Xray directly.
pub fn ensure_started() -> bool {
    if is_active() && XrayRunner::is_running() {
        return true;
    }
    if !unit_is_current() && !install_unit() {
        return false;
    }
    // Boot coverage where lingering allows; a failed enable still leaves a
    // startable unit.
    let _ = systemctl(&["enable", UNIT]);
    systemctl(&["start", UNIT]) == Some(true) && wait_running_pid().is_some()
}

/// Stops the unit. False when it was not active — callers then stop Xray
/// directly (fallback mode).
pub fn stop_unit() -> bool {
    if !is_active() {
        return false;
    }
    systemctl(&["stop", UNIT]).unwrap_or(false)
}

/// Restarts (or starts) the unit. False when systemd is unusable.
pub fn restart_unit() -> bool {
    if systemctl(&["restart", UNIT]) != Some(true) {
        return false;
    }
    is_active()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_content_pins_the_binary_and_run_entry() {
        let c = unit_content(Path::new("/usr/local/bin/xrs"));
        assert!(c.contains("ExecStart=\"/usr/local/bin/xrs\" run"));
        assert!(c.contains("Restart=on-failure"));
        assert!(c.contains("RestartSec=3s"));
        assert!(c.contains("WantedBy=default.target"));
    }
}
