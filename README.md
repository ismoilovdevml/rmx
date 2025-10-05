# RMX ⚡

> Blazing fast alternative to `rm` command written in Rust - **2x faster** on average!

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)

## 🚀 Features

- **⚡ Blazing Fast** - 2x faster for medium-large files
- **📊 Always Shows Stats** - Deleted files count, total size, and execution time
- **🔄 Smart Parallelism** - Adaptive threshold (2000+ files)
- **🎯 Cross-platform** - Works on Linux and macOS
- **💪 Highly Optimized** - LTO, aggressive inlining, lock-free atomics
- **✅ Full Compatibility** - Drop-in replacement for `rm -rf`

## 📥 Installation

### Quick Install (Linux & macOS)

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/rmx/main/install.sh | bash
```

### Manual Installation

Download pre-built binaries from [Releases](https://github.com/ismoilovdevml/rmx/releases/latest):

```bash
# Linux x86_64
wget https://github.com/ismoilovdevml/rmx/releases/latest/download/rmx-x86_64-unknown-linux-musl.tar.gz
tar xzf rmx-x86_64-unknown-linux-musl.tar.gz
sudo mv rmx /usr/local/bin/

# macOS (Intel)
wget https://github.com/ismoilovdevml/rmx/releases/latest/download/rmx-x86_64-apple-darwin.tar.gz
tar xzf rmx-x86_64-apple-darwin.tar.gz
sudo mv rmx /usr/local/bin/

# macOS (Apple Silicon)
wget https://github.com/ismoilovdevml/rmx/releases/latest/download/rmx-aarch64-apple-darwin.tar.gz
tar xzf rmx-aarch64-apple-darwin.tar.gz
sudo mv rmx /usr/local/bin/
```

### Build from Source

1. Install Rust (if not already installed):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. Clone and build:
```bash
git clone https://github.com/ismoilovdevml/rmx.git
cd rmx
cargo build --release
sudo cp target/release/rmx /usr/local/bin/
```

### Uninstall

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/rmx/main/uninstall.sh | bash
```

Or manually:
```bash
sudo rm /usr/local/bin/rmx
```

## 🎯 Usage

### Basic usage
```bash
rmx -rf /path/to/directory
```

### Example output
```
✓ Deleted: 10000 files, 1 directories
✓ Total size: 9.77 MB
✓ Time taken: 300.85ms
```

### Verbose mode (shows each file)
```bash
rmx -rfv /path/to/directory
```

### Available flags
```bash
rmx -r          # Remove directories recursively
rmx -f          # Force deletion, ignore errors
rmx -i          # Interactive mode (prompt before deletion)
rmx -v          # Verbose (show each file being deleted)
rmx -d          # Remove empty directories

# Combine flags
rmx -rf /tmp/test
rmx -rfi /important/data
```

### Other commands
```bash
rmx version    # Show version
rmx about      # Show program information
rmx dev        # Show developer info
```

## ⚡ Performance

| Test | Files | rm | rmx | Speedup |
|------|-------|-----|-----|---------|
| 1,000 × 1MB | 1,000 | 0.06s | 0.03s | **2.0x** ⚡ |
| 5,000 × 1KB | 5,000 | 0.25s | 0.12s | **2.1x** ⚡ |
| Nested | 1,000 | 0.06s | 0.03s | **2.0x** ⚡ |

*Tested on macOS Apple Silicon*

**Best for:**
- Node.js `node_modules` cleanup
- Build artifacts (`target/`, `dist/`)
- Log directories
- CI/CD cleanup tasks

## 🏗️ Optimizations

- **Adaptive Parallelism** - Sequential for <2000 files, parallel for ≥2000 files
- **Lock-free Atomics** - `AtomicUsize` and `AtomicU64` with Relaxed ordering
- **Cached Metadata** - Uses `DirEntry::metadata()` for filesystem cache
- **Aggressive Inlining** - Hot path functions marked `#[inline(always)]`

## 🛠️ Development

### Build
```bash
cargo build --release
```

### Run tests
```bash
cargo test
cargo clippy --release
cargo fmt
```

## 📝 License

MIT License - see [LICENSE](LICENSE) file

## 🙏 Credits

Inspired by [Manuchehr Usmonov's](https://github.com/yetimdasturchi) C implementation of rm alternative.

---

Made with ❤️ and Rust 🦀
