use serde::Deserialize;
use std::env;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use termion::{color, style};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_API_URL: &str = "https://api.github.com/repos/ismoilovdevml/rmx/releases/latest";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Get the target triple for the current platform
fn get_target_triple() -> Result<String, String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let target = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => {
            return Err(format!(
                "Unsupported platform: {}-{}. Please install manually.",
                os, arch
            ))
        }
    };

    Ok(target.to_string())
}

/// Check if a new version is available
pub fn check_for_updates() -> Result<Option<String>, String> {
    println!(
        "{}Checking for updates...{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    let response = ureq::get(GITHUB_API_URL)
        .set("User-Agent", "rmx-updater")
        .call()
        .map_err(|e| format!("Failed to check for updates: {}", e))?;

    let release: Release = response
        .into_json()
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v');

    if latest_version != CURRENT_VERSION {
        println!(
            "{}New version available: {} (current: {}){}",
            color::Fg(color::LightGreen),
            latest_version,
            CURRENT_VERSION,
            style::Reset
        );
        Ok(Some(latest_version.to_string()))
    } else {
        println!(
            "{}You are already using the latest version: {}{}",
            color::Fg(color::LightGreen),
            CURRENT_VERSION,
            style::Reset
        );
        Ok(None)
    }
}

/// Swap the freshly downloaded binary in for the running one.
///
/// The download lives under the temp dir, which is very often a *different*
/// filesystem from the install prefix — `/tmp` is tmpfs on most Linux distros —
/// so renaming straight across it fails with `EXDEV`. Copy into the target's own
/// directory first and rename within that filesystem: the rename is then atomic,
/// so a failure can never leave a half-written binary in place, and replacing a
/// running executable this way is fine on Unix (the old inode stays alive for
/// the running process).
fn install_binary(new_binary: &Path, current_exe: &Path) -> Result<(), String> {
    let dir = current_exe.parent().ok_or_else(|| {
        format!(
            "Cannot determine install directory for {}",
            current_exe.display()
        )
    })?;
    let staging = dir.join(".rmx-upgrade.tmp");

    if let Err(e) = fs::copy(new_binary, &staging) {
        if e.kind() == ErrorKind::PermissionDenied {
            return Err(permission_denied(new_binary, current_exe));
        }
        return Err(format!(
            "Failed to stage new binary in {}: {}",
            dir.display(),
            e
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&staging, fs::Permissions::from_mode(0o755)) {
            let _ = fs::remove_file(&staging);
            return Err(format!("Failed to set permissions: {}", e));
        }
    }

    if let Err(e) = fs::rename(&staging, current_exe) {
        let _ = fs::remove_file(&staging);
        if e.kind() == ErrorKind::PermissionDenied {
            return Err(permission_denied(new_binary, current_exe));
        }
        return Err(format!("Failed to install: {}", e));
    }

    Ok(())
}

/// Print manual-install instructions and return the error to bail out with.
fn permission_denied(new_binary: &Path, current_exe: &Path) -> String {
    println!(
        "{}Permission denied. Please run:{}",
        color::Fg(color::LightYellow),
        style::Reset
    );
    println!(
        "  sudo install -m 755 {} {}",
        new_binary.display(),
        current_exe.display()
    );
    println!(
        "\n{}Or run the upgrade with sudo:{}",
        color::Fg(color::LightYellow),
        style::Reset
    );
    println!("  sudo rmx upgrade");
    "Permission denied. See instructions above.".to_string()
}

/// Download and install the latest version
pub fn upgrade() -> Result<(), String> {
    println!(
        "{}Starting upgrade process...{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    // Get latest release info
    let response = ureq::get(GITHUB_API_URL)
        .set("User-Agent", "rmx-updater")
        .call()
        .map_err(|e| format!("Failed to fetch release info: {}", e))?;

    let release: Release = response
        .into_json()
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v');

    // Check if already up to date
    if latest_version == CURRENT_VERSION {
        println!(
            "{}Already up to date (v{}){}",
            color::Fg(color::LightGreen),
            CURRENT_VERSION,
            style::Reset
        );
        return Ok(());
    }

    // Get target triple
    let target = get_target_triple()?;
    let asset_name = format!("rmx-{}.tar.gz", target);

    // Find the asset for this platform
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            format!(
                "No binary found for your platform ({}). Please install manually.",
                target
            )
        })?;

    println!(
        "{}Downloading rmx v{}...{}",
        color::Fg(color::LightCyan),
        latest_version,
        style::Reset
    );

    // Download the tarball
    let response = ureq::get(&asset.browser_download_url)
        .call()
        .map_err(|e| format!("Failed to download: {}", e))?;

    let mut tarball_data = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut tarball_data)
        .map_err(|e| format!("Failed to read download: {}", e))?;

    // Create temp directory
    let temp_dir = env::temp_dir().join(format!("rmx-upgrade-{}", latest_version));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    // Save tarball
    let tarball_path = temp_dir.join(&asset_name);
    let mut file = fs::File::create(&tarball_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    file.write_all(&tarball_data)
        .map_err(|e| format!("Failed to write tarball: {}", e))?;

    println!(
        "{}Extracting...{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    // Extract tarball using tar command
    let extract_status = std::process::Command::new("tar")
        .args([
            "-xzf",
            tarball_path.to_str().unwrap(),
            "-C",
            temp_dir.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to extract tarball: {}", e))?;

    if !extract_status.success() {
        return Err("Failed to extract tarball".to_string());
    }

    // Get current binary path. Resolve symlinks so we replace the real file
    // rather than turning a symlink into a regular binary.
    let current_exe =
        env::current_exe().map_err(|e| format!("Failed to get current path: {}", e))?;
    let current_exe = fs::canonicalize(&current_exe).unwrap_or(current_exe);

    let new_binary = temp_dir.join("rmx");

    println!(
        "{}Installing...{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    install_binary(&new_binary, &current_exe)?;

    println!(
        "{}✓ Successfully upgraded to v{}!{}",
        color::Fg(color::LightGreen),
        latest_version,
        style::Reset
    );

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "{}Run 'rmx version' to verify the update{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(tag: &str) -> std::path::PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = env::temp_dir().join(format!(
            "rmx-upgrade-test-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// The download and the install prefix live in separate directories, which
    /// is the situation that used to hit EXDEV. Staging inside the target's own
    /// directory means the rename never crosses a filesystem.
    #[test]
    fn installs_from_a_separate_directory() {
        let root = scratch("swap");
        let download_dir = root.join("download");
        let install_dir = root.join("bin");
        fs::create_dir_all(&download_dir).unwrap();
        fs::create_dir_all(&install_dir).unwrap();

        let new_binary = download_dir.join("rmx");
        fs::write(&new_binary, b"NEW BINARY").unwrap();

        let current_exe = install_dir.join("rmx");
        fs::write(&current_exe, b"OLD BINARY").unwrap();

        install_binary(&new_binary, &current_exe).unwrap();

        assert_eq!(fs::read(&current_exe).unwrap(), b"NEW BINARY");
        // No staging file left behind.
        assert!(!install_dir.join(".rmx-upgrade.tmp").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn installed_binary_is_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("mode");
        let new_binary = root.join("downloaded");
        fs::write(&new_binary, b"#!/bin/sh\n").unwrap();
        // Arrive non-executable, as a fresh copy would.
        fs::set_permissions(&new_binary, fs::Permissions::from_mode(0o644)).unwrap();

        let current_exe = root.join("installed");
        fs::write(&current_exe, b"old").unwrap();

        install_binary(&new_binary, &current_exe).unwrap();

        let mode = fs::metadata(&current_exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "expected 0755, got {:o}", mode & 0o777);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_download_leaves_the_existing_binary_alone() {
        let root = scratch("missing");
        let current_exe = root.join("rmx");
        fs::write(&current_exe, b"OLD BINARY").unwrap();

        let err = install_binary(&root.join("does-not-exist"), &current_exe).unwrap_err();

        assert!(err.contains("Failed to stage"), "unexpected error: {err}");
        assert_eq!(fs::read(&current_exe).unwrap(), b"OLD BINARY");
        assert!(!root.join(".rmx-upgrade.tmp").exists());

        let _ = fs::remove_dir_all(&root);
    }
}
