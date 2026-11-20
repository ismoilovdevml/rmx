#!/usr/bin/env bash
#
# Install rmx from GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/ismoilovdevml/rmx/main/install.sh | bash
#
# Environment:
#   INSTALL_DIR   install prefix (default: /usr/local/bin)
#   RMX_VERSION   release tag to install, e.g. v0.8.0 (default: latest)
#
# The archive is verified against the release's SHA256SUMS.txt before it is
# extracted; installation is refused if that file or the archive's entry in it
# is missing (releases before v0.8.0 do not publish checksums).

set -euo pipefail

REPO="ismoilovdevml/rmx"
REPO_URL="https://github.com/$REPO"
BINARY_NAME="rmx"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
RMX_VERSION="${RMX_VERSION:-}"

CURL=(curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2
    --connect-timeout 10 --max-time 120 --retry 2)

die() {
    echo "Error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not installed"
}

need curl
need tar
need install
need awk

echo "Installing rmx - fast alternative to rm"

case "$(uname -s)" in
    Linux*)  OS="linux" ;;
    Darwin*) OS="darwin" ;;
    *)       die "unsupported OS: $(uname -s)" ;;
esac

case "$(uname -m)" in
    x86_64|amd64)  ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)             die "unsupported architecture: $(uname -m)" ;;
esac

if [ "$OS" = "linux" ]; then
    TARGET="${ARCH}-unknown-linux-musl"
else
    TARGET="${ARCH}-apple-darwin"
fi
ASSET="rmx-${TARGET}.tar.gz"
echo "Target: $TARGET"

if command -v sha256sum >/dev/null 2>&1; then
    SHA_CHECK=(sha256sum -c)
elif command -v shasum >/dev/null 2>&1; then
    SHA_CHECK=(shasum -a 256 -c)
else
    die "need 'sha256sum' or 'shasum' to verify the download"
fi

# Resolve the tag from the /releases/latest redirect (no API rate limit).
if [ -n "$RMX_VERSION" ]; then
    TAG="$RMX_VERSION"
    case "$TAG" in v*) ;; *) TAG="v$TAG" ;; esac
else
    echo "Resolving latest release..."
    EFFECTIVE_URL=$("${CURL[@]}" -o /dev/null -w '%{url_effective}' "$REPO_URL/releases/latest") \
        || die "could not reach $REPO_URL/releases/latest"
    TAG="${EFFECTIVE_URL#"$REPO_URL/releases/tag/"}"
    [ "$TAG" != "$EFFECTIVE_URL" ] || die "unexpected latest-release URL: $EFFECTIVE_URL"
fi

# The tag goes into URLs: accept vX.Y.Z only.
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid release tag: '$TAG' (expected vX.Y.Z)"
echo "Version: $TAG"

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

DOWNLOAD_BASE="$REPO_URL/releases/download/$TAG"

echo "Downloading SHA256SUMS.txt..."
if ! "${CURL[@]}" -o "$TEMP_DIR/SHA256SUMS.txt" "$DOWNLOAD_BASE/SHA256SUMS.txt"; then
    die "SHA256SUMS.txt is not available for $TAG; refusing to install an unverified binary.
       (Releases before v0.8.0 do not publish checksums.)"
fi

echo "Downloading $ASSET..."
"${CURL[@]}" -o "$TEMP_DIR/$ASSET" "$DOWNLOAD_BASE/$ASSET" \
    || die "download failed: $DOWNLOAD_BASE/$ASSET"

echo "Verifying SHA-256 checksum..."
awk -v f="$ASSET" '$2 == f || $2 == "*" f' "$TEMP_DIR/SHA256SUMS.txt" > "$TEMP_DIR/asset.sha256"
[ "$(wc -l < "$TEMP_DIR/asset.sha256")" -eq 1 ] \
    || die "SHA256SUMS.txt has no single entry for $ASSET; refusing to install"
(cd "$TEMP_DIR" && "${SHA_CHECK[@]}" asset.sha256 >/dev/null) \
    || die "checksum mismatch for $ASSET; the download is corrupt or has been tampered with"

echo "Extracting..."
mkdir "$TEMP_DIR/extract"
tar -xzf "$TEMP_DIR/$ASSET" -C "$TEMP_DIR/extract" "$BINARY_NAME" \
    || die "could not extract $BINARY_NAME from $ASSET"
NEW_BINARY="$TEMP_DIR/extract/$BINARY_NAME"
if [ -L "$NEW_BINARY" ] || [ ! -f "$NEW_BINARY" ]; then
    die "$BINARY_NAME in the archive is not a regular file"
fi

VERSION_OUT=$("$NEW_BINARY" --version 2>/dev/null || true)
case "$VERSION_OUT" in
    *"$TAG"*) ;;
    *) die "downloaded binary does not report $TAG (got: '$VERSION_OUT')" ;;
esac

SUDO=""
if [ -d "$INSTALL_DIR" ]; then
    [ -w "$INSTALL_DIR" ] || SUDO="sudo"
else
    PARENT="$INSTALL_DIR"
    while [ ! -d "$PARENT" ]; do PARENT=$(dirname "$PARENT"); done
    [ -w "$PARENT" ] || SUDO="sudo"
fi
if [ -n "$SUDO" ]; then
    [ "$(id -u)" -ne 0 ] || SUDO=""
fi
if [ -n "$SUDO" ]; then
    need sudo
    echo "Need sudo to write to $INSTALL_DIR"
fi

echo "Installing to $INSTALL_DIR..."
$SUDO mkdir -p "$INSTALL_DIR"
$SUDO install -m 755 "$NEW_BINARY" "$INSTALL_DIR/$BINARY_NAME"

echo ""
echo "rmx $TAG installed to $INSTALL_DIR/$BINARY_NAME"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Note: $INSTALL_DIR is not in your PATH" ;;
esac
echo ""
echo "Usage:"
echo "  rmx -rf /path/to/directory    Delete a directory recursively"
echo "  rmx --version                 Show version"
echo "  rmx upgrade                   Upgrade to the latest release"
