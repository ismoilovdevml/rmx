#!/usr/bin/env bash
#
# Remove rmx installed by install.sh.
#
# Environment:
#   INSTALL_DIR   install prefix used at install time (default: /usr/local/bin)

set -euo pipefail

BINARY_NAME="rmx"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
BINARY_PATH="$INSTALL_DIR/$BINARY_NAME"

die() {
    echo "Error: $*" >&2
    exit 1
}

echo "Uninstalling rmx..."

if [ ! -e "$BINARY_PATH" ] && [ ! -L "$BINARY_PATH" ]; then
    die "rmx is not installed at $BINARY_PATH (set INSTALL_DIR if you used a custom prefix)"
fi
if [ -d "$BINARY_PATH" ] && [ ! -L "$BINARY_PATH" ]; then
    die "$BINARY_PATH is a directory, not the rmx binary; refusing to remove it"
fi

SUDO=""
if [ ! -w "$INSTALL_DIR" ] && [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "no write access to $INSTALL_DIR and sudo is not available"
    echo "Need sudo to remove from $INSTALL_DIR"
    SUDO="sudo"
fi

echo "Removing $BINARY_PATH..."
$SUDO rm -f "$BINARY_PATH"

if [ -e "$BINARY_PATH" ] || [ -L "$BINARY_PATH" ]; then
    die "failed to remove $BINARY_PATH"
fi
echo "rmx has been uninstalled."
