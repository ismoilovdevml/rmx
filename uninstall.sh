#!/bin/bash

set -e

BINARY_NAME="rmx"
INSTALL_DIR="/usr/local/bin"
BINARY_PATH="$INSTALL_DIR/$BINARY_NAME"

echo "🗑️  Uninstalling RMX..."
echo ""

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ RMX is not installed at $BINARY_PATH"
    exit 1
fi

# Remove binary
echo "📋 Removing $BINARY_PATH..."

if [ -w "$INSTALL_DIR" ]; then
    rm -f "$BINARY_PATH"
else
    echo "🔐 Need sudo access to remove from $INSTALL_DIR"
    sudo rm -f "$BINARY_PATH"
fi

# Verify removal
if [ ! -f "$BINARY_PATH" ]; then
    echo ""
    echo "✅ RMX has been successfully uninstalled!"
    echo ""
else
    echo ""
    echo "❌ Failed to uninstall RMX"
    exit 1
fi
