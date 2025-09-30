# RMX ⚡

> Fast alternative to `rm` command written in Rust

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)

## 🚀 Features

- **⚡ Blazing Fast** - Uses parallel processing with Rayon for maximum performance
- **📊 Detailed Stats** - Shows deleted files count, total size, and execution time
- **🎯 Cross-platform** - Works on Linux and macOS
- **🔒 Safe** - Skips hidden files by default
- **💪 Optimized** - Built with LTO and maximum optimization flags

## 📥 Installation

### Quick Install (Linux & macOS)

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/rmx/master/install.sh | bash
```

### Manual Installation

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

## 🎯 Usage

### Delete files in a directory
```bash
rmx /path/to/directory
```

### Example output
```
Deleted files: 55853
Total size of deleted files: 108.83 GB
Time taken to delete: 449.65ms
```

### Other commands
```bash
rmx version    # Show version
rmx about      # Show information about the program
rmx dev        # Show developer info
```

## ⚡ Performance

RMX is designed to be faster than the standard `rm` command, especially for large directories:

- **v0.4.0**: Optimized parallel processing with filter_map
- **v0.3.0**: 55,853 files (108.83 GB) in 449.65ms
- **v0.2.0**: 55,586 files (108.83 GB) in 1.72s

**3.8x faster!** 🚀

## 🏗️ How It Works

RMX uses:
- **Rayon** for parallel file processing
- **filter_map** for efficient iteration
- **LTO & optimization** for maximum performance
- Recursive directory traversal
- Real-time statistics calculation

## 🛠️ Development

### Build
```bash
cargo build --release
```

### Run
```bash
cargo run -- rmx /path/to/directory
```


## 📝 License

MIT License - see [LICENSE](LICENSE) file


## 🙏 Credits

Inspired by [Manuchehr Usmonov's](https://github.com/yetimdasturchi) C implementation of rm alternative.

## ⚠️ Warning

**Use with caution!** This tool permanently deletes files. Always double-check the path before running.

---

Made with ❤️ and Rust 🦀
