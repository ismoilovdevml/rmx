#!/bin/bash

# Set package information
PKG_NAME=rmx
PKG_VERSION=0.3.0
PKG_MAINTAINER="Otabek Ismoilov"
PKG_DESCRIPTION="A program written in the Rust programming language for deleting large and very large files"
REPO_URL="https://github.com/ismoilovdevml/rmx"

# Clone the Rust program
echo "Cloning the Rust program..."
git clone "${REPO_URL}"
cd "${PKG_NAME}"

# Build the Rust program
echo "Building Rust program..."
cargo build --release

# Create directory for the Debian package
echo "Creating Debian package directory..."
DEB_DIR="${PKG_NAME}-deb"
mkdir "${DEB_DIR}"
mkdir "${DEB_DIR}/DEBIAN"

# Create control file
echo "Creating control file..."
cat > "${DEB_DIR}/DEBIAN/control" << EOF
Package: ${PKG_NAME}
Version: ${PKG_VERSION}
Section: base
Priority: optional
Architecture: amd64
Depends: 
Maintainer: ${PKG_MAINTAINER}
Description: ${PKG_DESCRIPTION}
EOF

# Move program into the package directory
echo "Moving program into package directory..."
mkdir -p "${DEB_DIR}/usr/bin"
cp target/release/${PKG_NAME} "${DEB_DIR}/usr/bin"

# Build the Debian package
echo "Building Debian package..."
dpkg-deb --build "${DEB_DIR}"

# Install the Debian package
echo "Installing Debian package..."
sudo dpkg -i "${DEB_DIR}.deb"

echo "Installation completed!"
