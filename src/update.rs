//! Self-update: check for new releases and replace the running binary.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

use crate::output;
use crate::resolve::http;

const REPO: &str = "angusbezzina/whetstone";

/// Detect the platform target triple matching the release binary naming.
fn detect_target() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (arch, os) {
        ("x86_64", "linux") => Ok("x86_64-unknown-linux-gnu"),
        ("aarch64", "linux") => Ok("aarch64-unknown-linux-gnu"),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin"),
        ("aarch64", "macos") => Ok("aarch64-apple-darwin"),
        _ => bail!("Unsupported platform: {arch}-{os}"),
    }
}

/// Fetch the latest release tag from GitHub.
fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let json = http::http_get_json(&url, 15)
        .context("Failed to fetch latest release from GitHub")?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .context("No tag_name in release response")?;
    Ok(tag.to_string())
}

/// Download bytes from a URL with an optional progress bar.
fn download_bytes(url: &str, show_progress: bool) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent("whetstone-update")
        .build()?;

    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        bail!(
            "Download failed: HTTP {} from {}",
            resp.status(),
            url
        );
    }

    let total_size = resp.content_length().unwrap_or(0);

    let pb = if show_progress && total_size > 0 {
        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {msg}")
                .unwrap()
                .progress_chars("\u{2501}\u{257a}\u{2500}"),
        );
        pb.set_message("Downloading...");
        pb
    } else {
        ProgressBar::hidden()
    };

    let mut bytes = Vec::with_capacity(total_size as usize);
    let mut reader = resp;
    let mut buf = [0u8; 8192];
    loop {
        use std::io::Read;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        pb.set_position(bytes.len() as u64);
    }
    pb.finish_and_clear();
    Ok(bytes)
}

/// Verify sha256 checksum of bytes against the checksums file content.
fn verify_checksum(binary_bytes: &[u8], checksums_text: &str, binary_name: &str) -> Result<()> {
    let expected = checksums_text
        .lines()
        .find(|line| line.contains(binary_name))
        .and_then(|line| line.split_whitespace().next())
        .context(format!(
            "Checksum not found for {binary_name} in checksums file"
        ))?;

    let mut hasher = Sha256::new();
    hasher.update(binary_bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        bail!(
            "Checksum mismatch:\n  expected: {expected}\n  actual:   {actual}"
        );
    }
    Ok(())
}

/// Replace the current binary with new bytes.
/// Uses a write-to-temp + rename strategy for atomicity.
fn replace_binary(new_bytes: &[u8], target_path: &PathBuf) -> Result<()> {
    let dir = target_path
        .parent()
        .context("Cannot determine binary directory")?;

    let tmp_path = dir.join(".whetstone-update.tmp");

    // Write new binary to temp file
    {
        let mut f = fs::File::create(&tmp_path)
            .context("Failed to create temp file for update")?;
        f.write_all(new_bytes)
            .context("Failed to write update binary")?;
    }

    // Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755))?;
    }

    // Atomic rename
    fs::rename(&tmp_path, target_path).context("Failed to replace binary (rename failed)")?;

    Ok(())
}

/// Check for and optionally apply an update.
pub fn check_and_update(force: bool, check_only: bool) -> Result<serde_json::Value> {
    let current_version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let show_progress = !output::is_piped();

    // Step 1: fetch latest version
    if show_progress {
        eprintln!("Checking for updates...");
    }
    let latest_version = fetch_latest_version()?;

    let is_current = current_version == latest_version;

    if check_only || (is_current && !force) {
        let status = if is_current { "up_to_date" } else { "update_available" };
        return Ok(serde_json::json!({
            "status": status,
            "current_version": current_version,
            "latest_version": latest_version,
            "message": if is_current {
                format!("Already up to date ({current_version})")
            } else {
                format!("Update available: {current_version} → {latest_version}")
            },
        }));
    }

    if is_current {
        return Ok(serde_json::json!({
            "status": "up_to_date",
            "current_version": current_version,
            "latest_version": latest_version,
            "message": format!("Already up to date ({current_version})"),
        }));
    }

    // Step 2: detect platform
    let target = detect_target()?;
    let binary_name = format!("whetstone-{target}");

    if show_progress {
        eprintln!(
            "Updating {current_version} → {latest_version} ({target})"
        );
    }

    // Step 3: download binary + checksums
    let binary_url = format!(
        "https://github.com/{REPO}/releases/download/{latest_version}/{binary_name}"
    );
    let checksums_url = format!(
        "https://github.com/{REPO}/releases/download/{latest_version}/checksums-sha256.txt"
    );

    let binary_bytes = download_bytes(&binary_url, show_progress)?;

    let checksums_text = http::http_get(&checksums_url, 30)
        .context("Failed to download checksums")?;

    // Step 4: verify checksum
    verify_checksum(&binary_bytes, &checksums_text, &binary_name)?;
    if show_progress {
        eprintln!("Checksum verified.");
    }

    // Step 5: replace the binary
    let exe_path = std::env::current_exe()
        .context("Cannot determine current executable path")?
        .canonicalize()
        .context("Cannot resolve executable path")?;

    replace_binary(&binary_bytes, &exe_path)?;

    // Step 6: also update the `wh` symlink/binary if it exists next to us
    let exe_dir = exe_path.parent().unwrap();
    let wh_path = exe_dir.join("wh");
    if wh_path.exists() || wh_path.is_symlink() {
        // If wh is a symlink pointing to whetstone, it's already updated.
        // If it's a separate binary, replace it too.
        let is_symlink = wh_path.is_symlink();
        if !is_symlink {
            let _ = replace_binary(&binary_bytes, &wh_path.to_path_buf());
        }
    }

    if show_progress {
        eprintln!(
            "Updated to {latest_version}!"
        );
    }

    Ok(serde_json::json!({
        "status": "updated",
        "previous_version": current_version,
        "new_version": latest_version,
        "target": target,
        "binary_path": exe_path.to_string_lossy(),
        "message": format!("Updated {current_version} → {latest_version}"),
    }))
}
