#!/bin/bash
#
# Bootstrap script for installing Zig compiler
#
# This script downloads and installs Zig to .context/zig/ for building
# libghostty-vt.
#
# Usage: ./scripts/bootstrap-zig.sh

set -euo pipefail

ZIG_VERSION="0.15.2"
INSTALL_DIR=".context/zig"

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64)
        ARCH="x86_64"
        ;;
    arm64|aarch64)
        ARCH="aarch64"
        ;;
    *)
        echo "Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

case "$OS" in
    darwin)
        OS="macos"
        ;;
    linux)
        OS="linux"
        ;;
    *)
        echo "Unsupported OS: $OS"
        exit 1
        ;;
esac

# Zig uses arch-os order in filenames
ARCHIVE_NAME="zig-${ARCH}-${OS}-${ZIG_VERSION}"
DOWNLOAD_URL="https://ziglang.org/download/${ZIG_VERSION}/${ARCHIVE_NAME}.tar.xz"

echo "=== Zig Bootstrap ==="
echo "Version: $ZIG_VERSION"
echo "Platform: $OS-$ARCH"
echo "Install directory: $INSTALL_DIR"
echo ""

# Check if already installed
if [ -x "$INSTALL_DIR/zig" ]; then
    INSTALLED_VERSION=$("$INSTALL_DIR/zig" version 2>/dev/null || echo "unknown")
    if [ "$INSTALLED_VERSION" = "$ZIG_VERSION" ]; then
        echo "Zig $ZIG_VERSION already installed at $INSTALL_DIR/zig"
        exit 0
    else
        echo "Upgrading from $INSTALLED_VERSION to $ZIG_VERSION"
    fi
fi

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

echo "Downloading $DOWNLOAD_URL..."
curl -L --progress-bar "$DOWNLOAD_URL" -o "$TEMP_DIR/zig.tar.xz"

# Extract (use xz + tar since macOS tar doesn't support .tar.xz natively)
echo "Extracting..."
xz -d "$TEMP_DIR/zig.tar.xz"
tar -xf "$TEMP_DIR/zig.tar" -C "$TEMP_DIR"

# Install
echo "Installing to $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"/*
mv "$TEMP_DIR/$ARCHIVE_NAME"/* "$INSTALL_DIR/"

# Verify
if [ -x "$INSTALL_DIR/zig" ]; then
    echo ""
    echo "=== Installation Complete ==="
    echo "Zig installed at: $INSTALL_DIR/zig"
    echo "Version: $($INSTALL_DIR/zig version)"
    echo ""
    echo "Add to your PATH or set ZIG environment variable:"
    echo "  export ZIG=$(pwd)/$INSTALL_DIR/zig"
else
    echo "ERROR: Installation failed"
    exit 1
fi
