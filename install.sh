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
        Darwin*)    echo "darwin";;
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

# Determine target triple
if [ "$OS" = "linux" ]; then
    TARGET="${ARCH}-unknown-linux-musl"
elif [ "$OS" = "darwin" ]; then
    TARGET="${ARCH}-apple-darwin"
else
    echo "❌ Unsupported operating system: $OS"
    exit 1
fi

echo "📦 Target: $TARGET"

# Get latest release version
echo "🔍 Fetching latest release..."
LATEST_RELEASE=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST_RELEASE" ]; then
    echo "❌ Could not fetch latest release"
    exit 1
fi

echo "✓ Latest version: $LATEST_RELEASE"

# Download URL
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_RELEASE/rmx-${TARGET}.tar.gz"

echo "📥 Downloading $BINARY_NAME from $DOWNLOAD_URL..."

TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

# Download binary
if ! curl -sSL -o "rmx.tar.gz" "$DOWNLOAD_URL"; then
    echo "❌ Download failed"
    echo "URL: $DOWNLOAD_URL"
    exit 1
fi

# Extract binary
echo "📦 Extracting..."
tar xzf rmx.tar.gz

if [ ! -f "$BINARY_NAME" ]; then
    echo "❌ Binary not found in archive"
    exit 1
fi

# Make it executable
chmod +x "$BINARY_NAME"

# Install binary
echo "📋 Installing to $INSTALL_DIR..."

if [ -w "$INSTALL_DIR" ]; then
    mv "$BINARY_NAME" "$INSTALL_DIR/"
else
    echo "🔐 Need sudo access to install to $INSTALL_DIR"
    sudo mv "$BINARY_NAME" "$INSTALL_DIR/"
fi

# Cleanup
cd ~
rm -rf "$TEMP_DIR"

echo ""
echo "✅ RMX $LATEST_RELEASE installed successfully!"
echo ""
echo "Usage:"
echo "  rmx -rf /path/to/directory    - Delete directory recursively"
echo "  rmx version                   - Show version"
echo "  rmx about                     - Show information"
echo ""
echo "⚡ RMX is now ready to use!"
