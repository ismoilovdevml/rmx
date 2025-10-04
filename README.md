# RMX ⚡

> Blazing fast alternative to `rm` command written in Rust - **1.67x faster** on average!

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)

## 🚀 Features

- **⚡ Blazing Fast** - 1.67x average speedup, up to 2.07x for many files
- **📊 Always Shows Stats** - Deleted files count, total size, and execution time
- **🔄 Smart Parallelism** - Adaptive threshold-based parallel processing
- **🎯 Cross-platform** - Works on Linux and macOS
- **💪 Highly Optimized** - LTO, aggressive inlining, lock-free atomics
- **✅ Full Compatibility** - Drop-in replacement for `rm -rf`

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

### Benchmark Results (vs standard `rm`)

| Test Scenario | Files | rm Time | rmx Time | **Speedup** |
|--------------|-------|---------|----------|-------------|
| 50K × 10B (ultra-tiny) | 50,000 | 2.77s | 1.33s | **2.07x** 🔥 |
| 30K × 100B (tiny) | 30,000 | 1.52s | 0.82s | **1.84x** 🔥 |
| 5K × 10KB (small) | 5,000 | 0.29s | 0.16s | **1.83x** 🔥 |
| 1K × 1MB (medium) | 1,000 | 0.11s | 0.06s | **1.83x** 🔥 |
| Nested (50×200) | 10,000 | 0.52s | 0.37s | **1.40x** |
| 200 × 10MB (large) | 200 | 0.05s | 0.04s | 1.05x |

**AVERAGE SPEEDUP: 1.67x** 🚀

### Performance by File Count

- **< 1,000 files**: ~1.0x (equivalent to rm)
- **1,000 - 10,000 files**: ~1.4x faster
- **10,000 - 50,000 files**: ~1.8x faster
- **> 50,000 files**: ~2.1x faster

### Best Use Cases

✅ **Perfect for:**
- Deleting `node_modules` directories
- Cleaning build artifacts (`target/`, `build/`, `dist/`)
- Removing log directories
- CI/CD cleanup tasks
- Development workspace cleanup

## 🏗️ How It Works

### Key Optimizations

1. **Adaptive Parallelism**
   - Sequential processing for <1000 files (avoids overhead)
   - Parallel processing (Rayon) for ≥1000 files (maximum speed)

2. **Cached Metadata Access**
   - Uses `DirEntry::metadata()` instead of `fs::metadata()`
   - Leverages filesystem cache for better performance

3. **Lock-free Atomic Counters**
   - `AtomicUsize` and `AtomicU64` with `Relaxed` ordering
   - Zero mutex contention in parallel code

4. **Aggressive Inlining**
   - `#[inline(always)]` on hot path functions
   - Better compiler optimizations

5. **Compiler Optimizations**
   ```toml
   opt-level = 3        # Maximum optimization
   lto = true           # Link-time optimization
   codegen-units = 1    # Better inlining
   strip = true         # Smaller binary
   ```

## 🛠️ Development

### Build
```bash
cargo build --release
```

### Run tests
```bash
cargo test
cargo clippy --release
```

### Run benchmarks
```bash
./benchmark_quick.sh     # Quick 5-test benchmark
./benchmark_medium.sh    # Medium 6-test benchmark
./benchmark_heavy.sh     # Heavy 8-test benchmark
```

## 📊 Benchmarking

See [FINAL_BENCHMARK_RESULTS.md](FINAL_BENCHMARK_RESULTS.md) for detailed benchmark results and methodology.

## 📝 License

MIT License - see [LICENSE](LICENSE) file

## 🙏 Credits

Inspired by [Manuchehr Usmonov's](https://github.com/yetimdasturchi) C implementation of rm alternative.

Optimized and enhanced by Claude Code.

## ⚠️ Warning

**Use with caution!** This tool permanently deletes files. Always double-check the path before running.

---

Made with ❤️ and Rust 🦀
