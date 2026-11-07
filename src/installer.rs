//! Platform-aware Xray release discovery and staged installation.
use crate::storage::{get_data_dir, load_config};
use colored::*;
use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn platform_asset(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("Xray-linux-64.zip"),
        ("linux", "aarch64") => Ok("Xray-linux-arm64-v8a.zip"),
        ("macos", "x86_64") => Ok("Xray-macos-64.zip"),
        ("macos", "aarch64") => Ok("Xray-macos-arm64-v8a.zip"),
        _ => Err(eyre!("Unsupported Xray installation platform: {os}/{arch}")),
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    published_at: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn select_release(releases: &[Release], asset_name: &str) -> Result<(String, String)> {
    // GitHub timestamps are UTC RFC3339 strings, so lexical order is chronological.
    // Do not filter prereleases: Xray publishes its newest builds that way too.
    let release = releases.iter()
        .filter(|r| !r.draft && r.published_at.is_some())
        .max_by_key(|r| r.published_at.as_deref())
        .ok_or_else(|| eyre!("GitHub returned no published Xray releases"))?;
    if release.tag_name.is_empty() {
        return Err(eyre!("Latest Xray release has an empty version tag"));
    }
    let asset = release.assets.iter().find(|a| a.name == asset_name)
        .ok_or_else(|| eyre!("Latest Xray release {} has no {asset_name} asset", release.tag_name))?;
    if !asset.browser_download_url.starts_with("https://github.com/") {
        return Err(eyre!("Invalid download URL for {asset_name}"));
    }
    Ok((release.tag_name.clone(), asset.browser_download_url.clone()))
}

fn latest_release(asset_name: &str) -> Result<(String, String)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build().into();
    let mut releases = Vec::new();
    // Follow pagination rather than assuming the first entry is newest by
    // publication time (GitHub orders this endpoint by creation time).
    for page in 1.. {
        let mut response = agent.get(&format!("https://api.github.com/repos/XTLS/Xray-core/releases?per_page=100&page={page}"))
            .header("User-Agent", concat!("xrs/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()
            .wrap_err("Could not resolve latest Xray release from GitHub; check connectivity or API rate limits. Existing installation unchanged")?;
        let body = response.body_mut().read_to_string()
            .wrap_err("Could not read Xray release metadata")?;
        let batch: Vec<Release> = serde_json::from_str(&body)
            .wrap_err("Invalid Xray release metadata from GitHub")?;
        let last_page = batch.len() < 100;
        releases.extend(batch);
        if last_page { break; }
    }
    select_release(&releases, asset_name)
}

/// Only removes the unique directory this installation created.
struct StagingDir(PathBuf);

impl StagingDir {
    fn new(data_dir: &Path) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(data_dir)?;
        loop {
            let path = data_dir.join(format!(".xray-install-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.0) {
            log::warn!("Could not clean up installer staging directory {}: {e}", self.0.display());
        }
    }
}

fn install_files(preserve_geodata: bool) -> &'static [&'static str] {
    if preserve_geodata { &["xray"] } else { &["geoip.dat", "geosite.dat", "xray"] }
}

fn validate_binary(path: &Path) -> Result<String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    let output = Command::new(path).arg("version").output()
        .wrap_err("Downloaded Xray binary cannot run on this platform; existing installation unchanged")?;
    if !output.status.success() {
        return Err(eyre!("Downloaded Xray failed version validation ({}): {}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    let stdout = String::from_utf8(output.stdout).wrap_err("Xray version output is not UTF-8")?;
    let version = stdout.lines().next().filter(|s| s.starts_with("Xray ") && s.split_whitespace().count() >= 2)
        .ok_or_else(|| eyre!("Downloaded Xray returned invalid version output"))?;
    Ok(version.to_string())
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let status = Command::new("curl")
        .args(["--location", "--fail", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "30", "--max-time", "300", "--output"])
        .arg(dest).arg(url).status().wrap_err("Failed to run curl")?;
    if !status.success() { return Err(eyre!("Xray download failed ({status}); existing installation unchanged")); }
    Ok(())
}

fn install_staged(stage: &Path, data_dir: &Path, preserve_geodata: bool) -> Result<String> {
    // Check all required files and the executable before touching installed files.
    for file in install_files(preserve_geodata) {
        let metadata = fs::symlink_metadata(stage.join(file))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(eyre!("Missing or invalid file in Xray archive: {file}"));
        }
    }
    let version = validate_binary(&stage.join("xray"))?;
    for file in install_files(preserve_geodata) {
        // Same-filesystem rename avoids truncating a running executable.
        fs::rename(stage.join(file), data_dir.join(file))
            .wrap_err_with(|| format!("Could not install {file}"))?;
    }
    Ok(version)
}

pub fn install_xray_and_assets() -> Result<()> {
    let asset = platform_asset(std::env::consts::OS, std::env::consts::ARCH)?;
    let (tag, url) = latest_release(asset)?;
    let data_dir = get_data_dir();
    let stage = StagingDir::new(&data_dir)?;
    println!("{}", format!("1. Downloading latest Xray-core {tag} ({asset})...").cyan());
    let archive = stage.0.join("xray.zip");
    download(&url, &archive)?;
    let cfg = load_config();
    let preserve_geodata = cfg.routing.rules.iter().any(|r| r.id == "iran_bypass" && r.enabled);
    println!("{}", "2. Extracting and validating Xray binary and routing data...".cyan());
    let status = Command::new("unzip").arg("-o").arg(&archive)
        .args(install_files(preserve_geodata)).arg("-d").arg(&stage.0)
        .status().wrap_err("Failed to run unzip")?;
    if !status.success() { return Err(eyre!("Failed to extract Xray archive; existing installation unchanged")); }
    let version = install_staged(&stage.0, &data_dir, preserve_geodata)?;

    // Capabilities are a Linux feature; never invoke getcap/pkexec on macOS.
    #[cfg(target_os = "linux")]
    if cfg.tun.enabled
        && let Err(e) = crate::xray::check_or_setup_tun_caps(&data_dir.join("xray"))
    {
        log::warn!("Could not restore TUN capabilities after the update: {e}");
    }
    println!("{}", "✔ Xray core installed successfully!".green().bold());
    println!("  Version:  {}", version.cyan());
    println!("  Location: {}", data_dir.display());
    if preserve_geodata {
        println!("  Iran routing preset stays installed (data: chocolate4u).");
    } else {
        println!("  Optional: Iran routing rules via '{}'", "xrs route setup-iran".cyan());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn supported_platforms_and_no_silent_fallback() {
        for (os, arch, expected) in [
            ("linux", "x86_64", "Xray-linux-64.zip"),
            ("linux", "aarch64", "Xray-linux-arm64-v8a.zip"),
            ("macos", "x86_64", "Xray-macos-64.zip"),
            ("macos", "aarch64", "Xray-macos-arm64-v8a.zip"),
        ] { assert_eq!(platform_asset(os, arch).expect("platform"), expected); }
        assert!(platform_asset("windows", "x86_64").is_err());
        assert!(platform_asset("linux", "riscv64").is_err());
    }

    fn release(tag: &str, published: &str, draft: bool) -> serde_json::Value {
        json!({"tag_name": tag, "published_at": published, "draft": draft,
            "prerelease": true, "assets": [{"name": "Xray-macos-64.zip",
            "browser_download_url": format!("https://github.com/XTLS/Xray-core/releases/download/{tag}/Xray-macos-64.zip")}]})
    }

    #[test]
    fn newest_published_release_includes_prereleases_but_not_drafts() {
        let releases: Vec<Release> = serde_json::from_value(json!([
            release("draft", "2026-12-01T00:00:00Z", true),
            release("old", "2026-09-01T00:00:00Z", false),
            release("new", "2026-11-01T00:00:00Z", false)
        ])).expect("fixture");
        let (tag, url) = select_release(&releases, "Xray-macos-64.zip").expect("release");
        assert_eq!(tag, "new");
        assert!(url.contains("/new/Xray-macos-64.zip"));
        assert!(select_release(&releases, "missing.zip").is_err());
        assert!(select_release(&[], "Xray-macos-64.zip").is_err());
        assert!(serde_json::from_str::<Vec<Release>>(r#"[{"tag_name":42}]"#).is_err());
    }

    #[test]
    fn missing_latest_asset_never_falls_back_to_older_release() {
        let mut newest = release("new", "2026-11-01T00:00:00Z", false);
        newest["assets"] = json!([]);
        let releases: Vec<Release> = serde_json::from_value(json!([
            release("old", "2026-09-01T00:00:00Z", false), newest
        ])).expect("fixture");
        assert!(select_release(&releases, "Xray-macos-64.zip").is_err());
    }

    #[test]
    fn validates_before_replacement_and_preserves_preset_geodata() {
        let base = StagingDir::new(&std::env::temp_dir()).expect("base");
        let stage = StagingDir::new(&base.0).expect("stage");
        fs::write(base.0.join("xray"), "old binary").expect("old");
        fs::write(base.0.join("geoip.dat"), "preset").expect("geodata");
        fs::write(stage.0.join("xray"), "#!/bin/sh\nexit 1\n").expect("binary");
        assert!(install_staged(&stage.0, &base.0, true).is_err());
        assert_eq!(fs::read_to_string(base.0.join("xray")).expect("old"), "old binary");
        fs::write(stage.0.join("xray"), "#!/bin/sh\necho 'Xray 26.11.1 test'\n").expect("binary");
        assert!(install_staged(&stage.0, &base.0, false).is_err(), "missing geodata");
        assert_eq!(install_staged(&stage.0, &base.0, true).expect("install"), "Xray 26.11.1 test");
        assert_eq!(fs::read_to_string(base.0.join("geoip.dat")).expect("preset"), "preset");
        assert_eq!(install_files(false), &["geoip.dat", "geosite.dat", "xray"]);
    }

    #[test]
    fn staging_is_unique_and_cleaned_up() {
        let base = StagingDir::new(&std::env::temp_dir()).expect("base");
        let first = StagingDir::new(&base.0).expect("stage");
        let second = StagingDir::new(&base.0).expect("stage");
        assert_ne!(first.0, second.0);
        let path = first.0.clone();
        drop(first);
        assert!(!path.exists());
    }

    #[test]
    fn failed_download_does_not_touch_installed_core() {
        let base = StagingDir::new(&std::env::temp_dir()).expect("base");
        let stage = StagingDir::new(&base.0).expect("stage");
        fs::write(base.0.join("xray"), "existing core").expect("old");
        // Disallowed protocol fails without a network dependency.
        assert!(download("file:///nonexistent-xrs-archive", &stage.0.join("xray.zip")).is_err());
        assert_eq!(fs::read_to_string(base.0.join("xray")).expect("old"), "existing core");
    }
}
