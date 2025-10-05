use serde::Deserialize;
use std::env;
use std::fs;
use std::io::Write;
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

    // Get current binary path
    let current_exe =
        env::current_exe().map_err(|e| format!("Failed to get current path: {}", e))?;

    let new_binary = temp_dir.join("rmx");

    // Make the new binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&new_binary)
            .map_err(|e| format!("Failed to get binary metadata: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&new_binary, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    println!(
        "{}Installing...{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    // Replace the current binary
    // Try to move first, if permission denied, suggest using sudo
    match fs::rename(&new_binary, &current_exe) {
        Ok(_) => {
            println!(
                "{}✓ Successfully upgraded to v{}!{}",
                color::Fg(color::LightGreen),
                latest_version,
                style::Reset
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // Save the path for manual installation
            println!(
                "{}Permission denied. Please run:{}",
                color::Fg(color::LightYellow),
                style::Reset
            );
            println!(
                "  sudo mv {} {}",
                new_binary.display(),
                current_exe.display()
            );
            println!(
                "\n{}Or run the upgrade with sudo:{}",
                color::Fg(color::LightYellow),
                style::Reset
            );
            println!("  sudo rmx upgrade");
            return Err("Permission denied. See instructions above.".to_string());
        }
        Err(e) => return Err(format!("Failed to install: {}", e)),
    }

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "{}Run 'rmx version' to verify the update{}",
        color::Fg(color::LightCyan),
        style::Reset
    );

    Ok(())
}
