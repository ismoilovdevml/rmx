//! `rmx upgrade` / `rmx check-update`.
//!
//! Transport is the system `curl` (HTTPS only, TLS >= 1.2, bounded timeouts);
//! the latest tag is resolved from the `/releases/latest` redirect rather than
//! the JSON API, so there is no rate limit and no JSON parser in the binary.
//! Every download is checked against the release's `SHA256SUMS.txt` before
//! anything is extracted, and the upgrade fails closed when that file or the
//! asset's line in it is missing.

use crate::style::{self, Stream};
use std::collections::hash_map::RandomState;
use std::env;
use std::ffi::{CString, OsString};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO_URL: &str = "https://github.com/ismoilovdevml/rmx";
const SUMS_FILE: &str = "SHA256SUMS.txt";

type Version = (u64, u64, u64);

fn info(text: &str) {
    println!("{}", style::paint(Stream::Stdout, style::CYAN, text));
}

fn good(text: &str) {
    println!("{}", style::paint(Stream::Stdout, style::GREEN, text));
}

fn warn(text: &str) {
    eprintln!("{}", style::paint(Stream::Stderr, style::YELLOW, text));
}

/// Get the target triple for the current platform
fn get_target_triple() -> Result<&'static str, String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        _ => Err(format!(
            "Unsupported platform: {}-{}. Please install manually.",
            os, arch
        )),
    }
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// Parse `X.Y.Z` (plain decimal components, nothing else).
fn parse_version(s: &str) -> Option<Version> {
    let mut parts = s.split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let v = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(v)
}

/// Parse a release tag. Only `vX.Y.Z` is accepted: the tag ends up in URLs,
/// so anything else (pre-release suffixes, slashes, query strings) is refused.
fn parse_tag(tag: &str) -> Option<Version> {
    parse_version(tag.strip_prefix('v')?)
}

fn current_version() -> Result<Version, String> {
    parse_version(CURRENT_VERSION)
        .ok_or_else(|| format!("Cannot parse own version '{}'", CURRENT_VERSION))
}

// ---------------------------------------------------------------------------
// Transport (system curl)
// ---------------------------------------------------------------------------

fn curl() -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsSL",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--tlsv1.2",
        "--connect-timeout",
        "10",
        "--max-time",
        "120",
        "--retry",
        "2",
    ]);
    cmd
}

fn run_curl(mut cmd: Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            "`curl` is required for upgrades but was not found in PATH. Install curl and retry."
                .to_string()
        } else {
            format!("Failed to run curl: {}", e)
        }
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Failed to download {}: {}", what, stderr.trim()));
    }
    Ok(out.stdout)
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = curl();
    cmd.arg("-o").arg(dest).arg(url);
    run_curl(cmd, url).map(|_| ())
}

/// Resolve the latest release tag by following the `/releases/latest`
/// redirect to `/releases/tag/vX.Y.Z`.
fn resolve_latest_tag() -> Result<String, String> {
    let mut cmd = curl();
    cmd.arg("-o")
        .arg(Path::new("/dev/null"))
        .args(["-w", "%{url_effective}"])
        .arg(format!("{REPO_URL}/releases/latest"));
    let out = run_curl(cmd, "latest release info")?;
    let url = String::from_utf8_lossy(&out);
    let url = url.trim();
    let prefix = format!("{REPO_URL}/releases/tag/");
    let tag = url
        .strip_prefix(&prefix)
        .ok_or_else(|| format!("Unexpected latest-release URL: {}", url))?;
    if parse_tag(tag).is_none() {
        return Err(format!("Refusing unexpected release tag: {:?}", tag));
    }
    Ok(tag.to_string())
}

fn release_url(tag: &str, file: &str) -> String {
    format!("{REPO_URL}/releases/download/{tag}/{file}")
}

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*chunk);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            self.update(&[0]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finish()))
}

/// Find `asset`'s digest in `sha256sum`-format text (`<hex>  <name>` or
/// `<hex> *<name>`).
fn expected_digest(sums: &str, asset: &str) -> Result<String, String> {
    let mut found: Option<String> = None;
    for line in sums.lines() {
        let mut it = line.split_whitespace();
        let (Some(hash), Some(name), None) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if name.strip_prefix('*').unwrap_or(name) != asset {
            continue;
        }
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "Malformed checksum line for {} in {}",
                asset, SUMS_FILE
            ));
        }
        let hash = hash.to_ascii_lowercase();
        if found.as_ref().is_some_and(|f| *f != hash) {
            return Err(format!(
                "Conflicting checksums for {} in {}",
                asset, SUMS_FILE
            ));
        }
        found = Some(hash);
    }
    found.ok_or_else(|| {
        format!(
            "{} has no entry for {}; refusing to install an unverified binary",
            SUMS_FILE, asset
        )
    })
}

fn verify_checksum(file: &Path, asset: &str, sums: &str) -> Result<(), String> {
    let expected = expected_digest(sums, asset)?;
    let actual = sha256_file(file).map_err(|e| format!("Failed to hash {}: {}", asset, e))?;
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for {}: expected {}, got {}. Download is corrupt or tampered with; aborting.",
            asset, expected, actual
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Private temp dir
// ---------------------------------------------------------------------------

/// A directory created with `mkdtemp(3)` (mode 0700, unpredictable name) and
/// removed on drop, including on every error path.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self, String> {
        let template = env::temp_dir().join("rmx-upgrade.XXXXXX");
        let template = CString::new(template.into_os_string().into_vec())
            .map_err(|_| "Temporary directory path contains a NUL byte".to_string())?;
        let mut bytes = template.into_bytes_with_nul();
        // SAFETY: `bytes` is a writable, NUL-terminated buffer that outlives the call.
        let ret = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast::<libc::c_char>()) };
        if ret.is_null() {
            return Err(format!(
                "Failed to create temporary directory: {}",
                io::Error::last_os_error()
            ));
        }
        bytes.pop(); // trailing NUL
        Ok(TempDir(PathBuf::from(OsString::from_vec(bytes))))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Extract, sanity-check, install
// ---------------------------------------------------------------------------

fn extract(tarball: &Path, dir: &Path) -> Result<PathBuf, String> {
    let out_dir = dir.join("extract");
    fs::create_dir(&out_dir).map_err(|e| format!("Failed to create extract dir: {}", e))?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(&out_dir)
        .arg("rmx")
        .status()
        .map_err(|e| format!("Failed to run tar: {}", e))?;
    if !status.success() {
        return Err("Failed to extract tarball".to_string());
    }
    let binary = out_dir.join("rmx");
    let meta = fs::symlink_metadata(&binary)
        .map_err(|e| format!("Extracted archive has no rmx binary: {}", e))?;
    if !meta.file_type().is_file() {
        return Err("Extracted rmx is not a regular file; aborting".to_string());
    }
    Ok(binary)
}

/// Run the new binary once and make sure it reports the version we expect.
fn sanity_check(binary: &Path, version: &str) -> Result<(), String> {
    let out = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("New binary failed to run: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !stdout.contains(&format!("v{version}")) {
        return Err(format!(
            "New binary did not report v{} (got {:?}); aborting",
            version,
            stdout.trim()
        ));
    }
    Ok(())
}

/// Create a uniquely named staging file next to `target` with `O_EXCL`.
fn create_staging(dir: &Path) -> io::Result<(PathBuf, fs::File)> {
    let mut last_err = io::Error::new(ErrorKind::AlreadyExists, "no unique staging name");
    for _ in 0..8 {
        let random = RandomState::new().build_hasher().finish();
        let name = format!(".rmx-upgrade.{}.{:016x}", std::process::id(), random);
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&path)
        {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last_err = e,
            Err(e) => return Err(e),
        }
    }
    Err(last_err)
}

fn copy_into(new_binary: &Path, file: &mut fs::File) -> io::Result<()> {
    let mut src = fs::File::open(new_binary)?;
    io::copy(&mut src, file)?;
    // The umask may have trimmed the create mode.
    file.set_permissions(fs::Permissions::from_mode(0o755))?;
    file.flush()?;
    file.sync_all()
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
///
/// The staging file has an unpredictable name and is created with `O_EXCL`, so
/// nothing pre-planted in the install directory can be picked up instead.
fn install_binary(new_binary: &Path, current_exe: &Path) -> Result<(), String> {
    let dir = current_exe.parent().ok_or_else(|| {
        format!(
            "Cannot determine install directory for {}",
            current_exe.display()
        )
    })?;

    let (staging, mut file) = create_staging(dir).map_err(|e| {
        if e.kind() == ErrorKind::PermissionDenied {
            permission_denied(current_exe)
        } else {
            format!("Failed to stage new binary in {}: {}", dir.display(), e)
        }
    })?;

    if let Err(e) = copy_into(new_binary, &mut file) {
        drop(file);
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "Failed to stage new binary in {}: {}",
            dir.display(),
            e
        ));
    }
    drop(file);

    if let Err(e) = fs::rename(&staging, current_exe) {
        let _ = fs::remove_file(&staging);
        if e.kind() == ErrorKind::PermissionDenied {
            return Err(permission_denied(current_exe));
        }
        return Err(format!("Failed to install: {}", e));
    }

    Ok(())
}

/// Print instructions and return the error to bail out with.
fn permission_denied(current_exe: &Path) -> String {
    warn(&format!(
        "Permission denied writing {}. Re-run with sudo:",
        current_exe.display()
    ));
    eprintln!("  sudo rmx upgrade");
    "Permission denied. See instructions above.".to_string()
}

/// Verify, extract, sanity-check and install a downloaded release tarball.
fn install_verified(
    dir: &Path,
    tarball: &Path,
    asset: &str,
    sums: &str,
    version: &str,
    current_exe: &Path,
) -> Result<(), String> {
    info("Verifying SHA-256 checksum...");
    verify_checksum(tarball, asset, sums)?;
    info("Extracting...");
    let binary = extract(tarball, dir)?;
    sanity_check(&binary, version)?;
    info("Installing...");
    install_binary(&binary, current_exe)
}

/// Download release `tag` for `target` and install it over `current_exe`.
fn perform_upgrade(tag: &str, target: &str, current_exe: &Path) -> Result<(), String> {
    if parse_tag(tag).is_none() {
        return Err(format!("Refusing unexpected release tag: {:?}", tag));
    }
    let version = &tag[1..];
    let tmp = TempDir::new()?;
    let asset = format!("rmx-{}.tar.gz", target);

    info(&format!("Downloading {}...", SUMS_FILE));
    let sums_path = tmp.path().join(SUMS_FILE);
    download(&release_url(tag, SUMS_FILE), &sums_path).map_err(|e| {
        format!(
            "{} is not available for {}; refusing to install an unverified binary ({})",
            SUMS_FILE, tag, e
        )
    })?;
    let sums = fs::read_to_string(&sums_path)
        .map_err(|e| format!("Failed to read {}: {}", SUMS_FILE, e))?;

    info(&format!("Downloading {}...", asset));
    let tarball = tmp.path().join(&asset);
    download(&release_url(tag, &asset), &tarball)?;

    install_verified(tmp.path(), &tarball, &asset, &sums, version, current_exe)
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Check if a new version is available
pub fn check_for_updates() -> Result<Option<String>, String> {
    info("Checking for updates...");
    let current = current_version()?;
    let tag = resolve_latest_tag()?;
    let latest = parse_tag(&tag).ok_or_else(|| format!("Bad release tag {:?}", tag))?;
    let latest_str = &tag[1..];

    if latest > current {
        good(&format!(
            "New version available: {} (current: {})",
            latest_str, CURRENT_VERSION
        ));
        Ok(Some(latest_str.to_string()))
    } else if latest < current {
        good(&format!(
            "You are running v{}, newer than the latest release v{}",
            CURRENT_VERSION, latest_str
        ));
        Ok(None)
    } else {
        good(&format!(
            "You are already using the latest version: {}",
            CURRENT_VERSION
        ));
        Ok(None)
    }
}

/// Download and install the latest version
pub fn upgrade() -> Result<(), String> {
    info("Starting upgrade process...");

    let target = get_target_triple()?;
    let current = current_version()?;
    let tag = resolve_latest_tag()?;
    let latest = parse_tag(&tag).ok_or_else(|| format!("Bad release tag {:?}", tag))?;

    if latest == current {
        good(&format!("Already up to date (v{})", CURRENT_VERSION));
        return Ok(());
    }
    if latest < current {
        good(&format!(
            "Installed v{} is newer than the latest release {}; nothing to do",
            CURRENT_VERSION, tag
        ));
        return Ok(());
    }

    // Resolve symlinks so we replace the real file rather than turning a
    // symlink into a regular binary.
    let current_exe =
        env::current_exe().map_err(|e| format!("Failed to get current path: {}", e))?;
    let current_exe = fs::canonicalize(&current_exe).unwrap_or(current_exe);

    perform_upgrade(&tag, target, &current_exe)?;

    good(&format!("✓ Successfully upgraded to {}!", tag));
    info("Run 'rmx --version' to verify the update");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(tag: &str) -> PathBuf {
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

    fn staging_leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".rmx-upgrade"))
            .collect()
    }

    fn sha_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        to_hex(&h.finish())
    }

    // --- SHA-256 ----------------------------------------------------------

    #[test]
    fn sha256_standard_vectors() {
        assert_eq!(
            sha_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_million_a_streamed_in_odd_chunks() {
        let mut h = Sha256::new();
        let chunk = [b'a'; 997];
        let mut left = 1_000_000usize;
        while left > 0 {
            let n = left.min(chunk.len());
            h.update(&chunk[..n]);
            left -= n;
        }
        assert_eq!(
            to_hex(&h.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    // --- checksum file ----------------------------------------------------

    #[test]
    fn verify_checksum_accepts_match_and_rejects_mismatch() {
        let root = scratch("sum");
        let f = root.join("rmx-x.tar.gz");
        fs::write(&f, b"abc").unwrap();
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let sums = format!("{}  other.tar.gz\n{}  rmx-x.tar.gz\n", "0".repeat(64), good);
        verify_checksum(&f, "rmx-x.tar.gz", &sums).unwrap();
        // sha256sum binary-mode marker and upper-case hex are fine too.
        let sums = format!("{} *rmx-x.tar.gz\n", good.to_uppercase());
        verify_checksum(&f, "rmx-x.tar.gz", &sums).unwrap();

        let bad = format!("{}  rmx-x.tar.gz\n", "f".repeat(64));
        let err = verify_checksum(&f, "rmx-x.tar.gz", &bad).unwrap_err();
        assert!(err.contains("Checksum mismatch"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_entry_fails_closed() {
        let sums = format!("{}  rmx-other.tar.gz\n", "a".repeat(64));
        let err = expected_digest(&sums, "rmx-x.tar.gz").unwrap_err();
        assert!(err.contains("no entry"), "{err}");
        assert!(expected_digest("", "rmx-x.tar.gz").is_err());
        // Prefix of the name must not match.
        let sums = format!("{}  rmx-x.tar.gz.sig\n", "a".repeat(64));
        assert!(expected_digest(&sums, "rmx-x.tar.gz").is_err());
        // Malformed / conflicting entries are errors, not skipped.
        assert!(expected_digest("abc  rmx-x.tar.gz\n", "rmx-x.tar.gz").is_err());
        let sums = format!(
            "{}  rmx-x.tar.gz\n{}  rmx-x.tar.gz\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert!(expected_digest(&sums, "rmx-x.tar.gz").is_err());
    }

    // --- versions ---------------------------------------------------------

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("0.8.0"), Some((0, 8, 0)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1..3",
            "-1.2.3",
            "1.2.3-rc1",
            " 1.2.3",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert_eq!(parse_tag("v0.7.0"), Some((0, 7, 0)));
        for bad in [
            "0.7.0",
            "V0.7.0",
            "v0.7.0/../x",
            "v0.7.0?x=1",
            "v0.7.0-rc1",
            "vv0.7.0",
            "v0.7",
            "v99999999999.0.0",
        ] {
            assert_eq!(parse_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn version_ordering_is_numeric() {
        let v = |s| parse_version(s).unwrap();
        assert!(v("0.8.0") > v("0.7.0"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.7.0") < v("0.8.0"), "no downgrade from 0.8.0 to 0.7.0");
        assert!(current_version().is_ok());
    }

    // --- temp dir ---------------------------------------------------------

    #[test]
    fn temp_dir_is_private_and_removed_on_drop() {
        let t = TempDir::new().unwrap();
        let p = t.path().to_path_buf();
        assert!(p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("rmx-upgrade."));
        let mode = fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "got {:o}", mode & 0o777);
        fs::write(p.join("junk"), b"x").unwrap();
        let t2 = TempDir::new().unwrap();
        assert_ne!(t2.path(), p, "names must be unique");
        drop(t);
        assert!(!p.exists());
    }

    // --- install ----------------------------------------------------------

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
        assert_eq!(staging_leftovers(&install_dir), Vec::<String>::new());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installed_binary_is_executable() {
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
        assert_eq!(staging_leftovers(&root), Vec::<String>::new());

        let _ = fs::remove_dir_all(&root);
    }

    /// A pre-planted file (or symlink) with a staging-like name is never used.
    #[test]
    fn preplanted_staging_name_is_not_reused() {
        let root = scratch("plant");
        let planted = root.join(".rmx-upgrade.tmp");
        fs::write(&planted, b"EVIL").unwrap();
        let new_binary = root.join("new");
        fs::write(&new_binary, b"NEW").unwrap();
        let current_exe = root.join("rmx");
        fs::write(&current_exe, b"OLD").unwrap();

        install_binary(&new_binary, &current_exe).unwrap();
        assert_eq!(fs::read(&current_exe).unwrap(), b"NEW");
        assert_eq!(fs::read(&planted).unwrap(), b"EVIL");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_rejects_symlinked_binary() {
        let root = scratch("symlink");
        let src = root.join("src");
        fs::create_dir(&src).unwrap();
        std::os::unix::fs::symlink("/bin/sh", src.join("rmx")).unwrap();
        let tarball = root.join("t.tar.gz");
        let ok = Command::new("tar")
            .arg("-czf")
            .arg(&tarball)
            .arg("-C")
            .arg(&src)
            .arg("rmx")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let work = root.join("work");
        fs::create_dir(&work).unwrap();
        let err = extract(&tarball, &work).unwrap_err();
        assert!(err.contains("not a regular file"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sanity_check_requires_matching_version() {
        let root = scratch("sanity");
        let bin = root.join("fake");
        fs::write(&bin, b"#!/bin/sh\necho \"rmx v9.9.9\"\n").unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        sanity_check(&bin, "9.9.9").unwrap();
        assert!(sanity_check(&bin, "9.9.8").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    // --- live network tests (not run by default) --------------------------
    //
    // These hit github.com and install only into a scratch copy under the
    // temp dir. Run with: cargo test --release -- --ignored live_

    fn live_target() -> &'static str {
        get_target_triple().unwrap()
    }

    #[test]
    #[ignore]
    fn live_resolves_latest_tag() {
        let tag = resolve_latest_tag().unwrap();
        println!("latest tag: {tag}");
        assert!(parse_tag(&tag).is_some());
    }

    /// v0.7.0 predates SHA256SUMS.txt, so a real upgrade to it must fail
    /// closed and leave the scratch install untouched.
    #[test]
    #[ignore]
    fn live_v0_7_0_fails_closed_without_sums() {
        let root = scratch("live-closed");
        let exe = root.join("rmx");
        fs::write(&exe, b"OLD BINARY").unwrap();
        let err = perform_upgrade("v0.7.0", live_target(), &exe).unwrap_err();
        println!("error: {err}");
        assert!(err.contains("SHA256SUMS.txt is not available"), "{err}");
        assert_eq!(fs::read(&exe).unwrap(), b"OLD BINARY");
        assert_eq!(staging_leftovers(&root), Vec::<String>::new());
        let _ = fs::remove_dir_all(&root);
    }

    /// Full flow against the real v0.7.0 tarball, with a SUMS file built from
    /// an independent `shasum -a 256` of the download.
    #[test]
    #[ignore]
    fn live_v0_7_0_installs_with_synthetic_sums() {
        let root = scratch("live-ok");
        let exe = root.join("rmx");
        fs::write(&exe, b"OLD BINARY").unwrap();

        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_path_buf();
        let asset = format!("rmx-{}.tar.gz", live_target());
        let tarball = tmp.path().join(&asset);
        download(&release_url("v0.7.0", &asset), &tarball).unwrap();

        let out = Command::new("shasum")
            .args(["-a", "256"])
            .arg(&tarball)
            .output()
            .unwrap();
        assert!(out.status.success());
        let digest = String::from_utf8(out.stdout).unwrap();
        let digest = digest.split_whitespace().next().unwrap().to_string();
        println!("shasum -a 256 {asset}: {digest}");
        assert_eq!(sha256_file(&tarball).unwrap(), digest, "own SHA-256 agrees");

        // Tampered SUMS: refused, scratch install untouched.
        let bad = format!("{}  {}\n", "0".repeat(64), asset);
        let err = install_verified(tmp.path(), &tarball, &asset, &bad, "0.7.0", &exe).unwrap_err();
        println!("tampered sums: {err}");
        assert!(err.contains("Checksum mismatch"));
        assert_eq!(fs::read(&exe).unwrap(), b"OLD BINARY");

        // Correct SUMS: installed and runnable.
        let sums = format!("{}  {}\n", digest, asset);
        install_verified(tmp.path(), &tarball, &asset, &sums, "0.7.0", &exe).unwrap();
        let out = Command::new(&exe).arg("--version").output().unwrap();
        let v = String::from_utf8_lossy(&out.stdout);
        println!("installed scratch binary --version: {}", v.trim());
        assert!(v.contains("v0.7.0"));
        assert_eq!(staging_leftovers(&root), Vec::<String>::new());

        drop(tmp);
        assert!(!tmp_path.exists(), "temp dir cleaned up");
        let _ = fs::remove_dir_all(&root);
    }
}
