#!/bin/bash

set -e

REPO="ismoilovdevml/rmx"
BINARY_NAME="rmx"
INSTALL_DIR="/usr/local/bin"

echo "🚀 Installing RMX - Fast alternative to rm command"
echo ""

detect_os() {
    case "$(uname -s)" in
        Linux*)     echo "linux";;
        Darwin*)    echo "macos";;
        *)          echo "unknown";;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)   echo "x86_64";;
        aarch64|arm64)  echo "aarch64";;
        *)              echo "unknown";;
    esac
}

OS=$(detect_os)
ARCH=$(detect_arch)

if [ "$OS" = "unknown" ] || [ "$ARCH" = "unknown" ]; then
    echo "❌ Unsupported OS or architecture"
    echo "OS: $OS, Architecture: $ARCH"
    exit 1
fi

echo "✓ Detected: $OS ($ARCH)"

if ! command -v cargo &> /dev/null; then
    echo "⚠️  Rust not found. Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "✓ Rust is already installed"
fi

TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

echo "📦 Downloading RMX..."
git clone --depth=1 https://github.com/$REPO.git
cd rmx

echo "🔨 Building RMX (optimized release)..."
cargo build --release

BINARY_PATH="target/release/$BINARY_NAME"

if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Build failed"
    exit 1
fi

echo "📋 Installing to $INSTALL_DIR..."

if [ -w "$INSTALL_DIR" ]; then
    cp "$BINARY_PATH" "$INSTALL_DIR/"
else
    echo "🔐 Need sudo access to install to $INSTALL_DIR"
    sudo cp "$BINARY_PATH" "$INSTALL_DIR/"
fi

chmod +x "$INSTALL_DIR/$BINARY_NAME"

cd ~
rm -rf "$TEMP_DIR"

echo ""
echo "✅ RMX installed successfully!"
echo ""
echo "Usage:"
echo "  rmx /path/to/directory    - Delete all files in directory"
echo "  rmx version               - Show version"
echo "  rmx about                 - Show information"
echo ""
echo "⚡ RMX is now ready to use!"