//! Runtime DNS on the Xray-owned Linux TUN, never on an uplink.
use crate::storage::get_state_dir;
use crate::xray::{is_valid_ifname, resolve_tool};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

// A routing destination, NOT an upstream and NOT a listener on the TUN address.
// tun-in's port-53 rule hands A/AAAA questions to Xray's configured DNS engine.
const DESTINATION: &str = "1.1.1.1";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Bounded execution with argv, including elevation; no shell or inherited stdin.
fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot execute {program}: {e}"))?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!("{program} {} failed ({status})", args.join(" ")));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} timed out or could not be waited for: {result:?}"
                ));
            }
        }
    }
}

fn command(args: &[&str]) -> Result<(), String> {
    let tool = resolve_tool("resolvectl");
    let mut elevated = vec!["-n", tool.as_str()];
    elevated.extend_from_slice(args);
    if run("sudo", &elevated).is_ok() {
        return Ok(());
    }
    let mut elevated = vec![tool.as_str()];
    elevated.extend_from_slice(args);
    run("pkexec", &elevated).map_err(|e| {
        format!(
            "TUN DNS: {e}. Check sudo/polkit permission for resolvectl and systemd-resolved.service"
        )
    })
}

pub fn preflight() -> Result<(), String> {
    check_resolved(cfg!(target_os = "linux"), |program, args| {
        run(program, args)
    })
}

fn check_resolved(
    linux: bool,
    mut execute: impl FnMut(&str, &[&str]) -> Result<(), String>,
) -> Result<(), String> {
    if !linux {
        return Err("TUN system DNS integration requires Linux with systemd-resolved; macOS TUN is not supported. Use SOCKS/HTTP proxy mode.".into());
    }
    execute("systemctl", &["is-active", "--quiet", "systemd-resolved.service"])
        .and_then(|()| execute(&resolve_tool("resolvectl"), &["status"]))
        .map_err(|e| format!("TUN DNS requires active systemd-resolved and resolvectl: {e}. Configure resolved for your distribution or use SOCKS/HTTP proxy mode; resolver files were not changed."))
}

fn configure(
    name: &str,
    mut execute: impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    for args in [
        vec!["dns", name, DESTINATION],
        vec!["domain", name, "~."],
        vec!["default-route", name, "yes"],
        vec!["dnsovertls", name, "no"],
        vec!["flush-caches"],
    ] {
        if let Err(error) = execute(&args) {
            let revert = execute(&["revert", name]);
            let flush = execute(&["flush-caches"]);
            return Err(format!(
                "{error}; DNS rollback: {revert:?}; cache flush: {flush:?}"
            ));
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct OwnedLink {
    name: String,
    index: u32,
}
fn state_file() -> PathBuf {
    get_state_dir().join("tun-dns.json")
}
fn link_index(name: &str) -> Option<u32> {
    if !is_valid_ifname(name) {
        return None;
    }
    let path = PathBuf::from("/sys/class/net").join(name);
    // Do not revert an unrelated physical link even if a stale name is reused.
    if !path.join("tun_flags").exists() {
        return None;
    }
    fs::read_to_string(path.join("ifindex"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn activate(name: &str) -> Result<(), String> {
    let index = link_index(name).ok_or("TUN DNS: created TUN interface is missing")?;
    // Journal before the first mutation, so a killed activation can be cleaned up.
    fs::create_dir_all(get_state_dir()).map_err(|e| e.to_string())?;
    let content = serde_json::to_vec(&OwnedLink {
        name: name.into(),
        index,
    })
    .map_err(|e| e.to_string())?;
    let pending = state_file().with_extension("tmp");
    fs::write(&pending, content)
        .and_then(|()| fs::rename(&pending, state_file()))
        .map_err(|e| format!("Cannot journal TUN DNS ownership: {e}"))?;
    configure(name, command)
}

/// Link deletion already removes resolved's per-link state. Only revert a
/// surviving link when both its name and kernel index still match our journal.
pub fn cleanup() -> Result<(), String> {
    cleanup_in(&state_file(), link_index, command)
}

fn cleanup_in(
    path: &std::path::Path,
    index: impl Fn(&str) -> Option<u32>,
    mut execute: impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Cannot read TUN DNS ownership: {e}")),
    };
    let link: OwnedLink = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Invalid TUN DNS ownership journal: {e}"))?;
    if is_valid_ifname(&link.name) && index(&link.name) == Some(link.index) {
        execute(&["revert", &link.name])?;
    }
    execute(&["flush-caches"])?;
    fs::remove_file(path).map_err(|e| format!("Cannot remove TUN DNS journal: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configures_only_the_actual_tun_and_flushes_last() {
        let mut calls = Vec::new();
        configure("custom-tun", |args| {
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(())
        })
        .expect("setup");
        assert_eq!(
            calls,
            [
                vec!["dns", "custom-tun", "1.1.1.1"],
                vec!["domain", "custom-tun", "~."],
                vec!["default-route", "custom-tun", "yes"],
                vec!["dnsovertls", "custom-tun", "no"],
                vec!["flush-caches"]
            ]
        );
    }
    #[test]
    fn every_partial_failure_reverts_and_flushes_including_privilege_errors() {
        for fail in 0..5 {
            let mut calls = Vec::new();
            let error = configure("custom-tun", |args| {
                calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                if calls.len() == fail + 1 {
                    Err("permission denied".into())
                } else {
                    Ok(())
                }
            })
            .expect_err("failure");
            assert!(error.contains("permission denied"));
            assert_eq!(calls[calls.len() - 2], ["revert", "custom-tun"]);
            assert_eq!(calls[calls.len() - 1], ["flush-caches"]);
        }
    }
    #[test]
    fn repeated_activation_reapplies_all_settings() {
        let mut count = 0;
        for _ in 0..3 {
            configure("recreated-tun", |_| {
                count += 1;
                Ok(())
            })
            .expect("reconnect");
        }
        assert_eq!(count, 15);
    }
    #[test]
    fn unavailable_resolved_and_unsupported_platform_are_actionable() {
        for fail in 0..2 {
            let mut calls = 0;
            let error = check_resolved(true, |_, _| {
                calls += 1;
                if calls == fail + 1 {
                    Err("missing or inactive".into())
                } else {
                    Ok(())
                }
            })
            .expect_err("unavailable");
            assert!(error.contains("active systemd-resolved and resolvectl"));
            assert_eq!(calls, fail + 1);
        }
        check_resolved(true, |_, _| Ok(())).expect("active resolved");
        assert!(
            check_resolved(false, |_, _| panic!("no Linux commands on macOS"))
                .expect_err("macOS")
                .contains("macOS")
        );
    }
    #[test]
    fn cleanup_is_idempotent_and_does_not_touch_a_recreated_link() {
        let path = std::env::temp_dir().join(format!("xrs-dns-test-{}", std::process::id()));
        for current_index in [Some(42), Some(43), None] {
            let bytes = serde_json::to_vec(&OwnedLink {
                name: "custom-tun".into(),
                index: 42,
            })
            .expect("serialize");
            fs::write(&path, bytes).expect("journal");
            // Failed cleanup must retain the journal for the next startup.
            assert!(cleanup_in(&path, |_| current_index, |_| Err("denied".into())).is_err());
            assert!(path.exists());
            let mut calls = Vec::new();
            cleanup_in(
                &path,
                |_| current_index,
                |args| {
                    calls.push(args.join(" "));
                    Ok(())
                },
            )
            .expect("cleanup");
            assert_eq!(calls.last().map(String::as_str), Some("flush-caches"));
            assert_eq!(
                calls.contains(&"revert custom-tun".into()),
                current_index == Some(42)
            );
            cleanup_in(
                &path,
                |_| panic!("already cleaned"),
                |_| panic!("no commands on repeated stop"),
            )
            .expect("idempotent");
        }
    }
    #[test]
    fn physical_and_invalid_links_cannot_be_owned() {
        assert_eq!(link_index("lo"), None);
        assert_eq!(link_index("../etc"), None);
    }
}
