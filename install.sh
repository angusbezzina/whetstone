#!/bin/sh
set -e

REPO="angusbezzina/whetstone"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="whetstone"

main() {
    detect_platform
    get_latest_version
    download_and_verify
    install_binary
    print_success
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  OS_TARGET="unknown-linux-gnu" ;;
        Darwin) OS_TARGET="apple-darwin" ;;
        *)
            echo "Error: unsupported operating system: $OS" >&2
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64)          ARCH_TARGET="x86_64" ;;
        arm64|aarch64)   ARCH_TARGET="aarch64" ;;
        *)
            echo "Error: unsupported architecture: $ARCH" >&2
            exit 1
            ;;
    esac

    TARGET="${ARCH_TARGET}-${OS_TARGET}"
    echo "Detected platform: $TARGET"
}

get_latest_version() {
    echo "Fetching latest release..."
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"

    if [ -z "$VERSION" ]; then
        echo "Error: could not determine latest version" >&2
        exit 1
    fi

    echo "Latest version: $VERSION"
}

download_and_verify() {
    BINARY_URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}-${TARGET}"
    CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/checksums-sha256.txt"

    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "Downloading ${BINARY_NAME}-${TARGET}..."
    curl -fsSL -o "${TMPDIR}/${BINARY_NAME}" "$BINARY_URL"

    echo "Downloading checksums..."
    curl -fsSL -o "${TMPDIR}/checksums-sha256.txt" "$CHECKSUMS_URL"

    echo "Verifying checksum..."
    EXPECTED="$(grep "${BINARY_NAME}-${TARGET}" "${TMPDIR}/checksums-sha256.txt" | awk '{print $1}')"

    if [ -z "$EXPECTED" ]; then
        echo "Error: checksum not found for ${BINARY_NAME}-${TARGET}" >&2
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL="$(sha256sum "${TMPDIR}/${BINARY_NAME}" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL="$(shasum -a 256 "${TMPDIR}/${BINARY_NAME}" | awk '{print $1}')"
    else
        echo "Warning: neither sha256sum nor shasum found, skipping verification" >&2
        return
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "Error: checksum mismatch" >&2
        echo "  expected: $EXPECTED" >&2
        echo "  actual:   $ACTUAL" >&2
        exit 1
    fi

    echo "Checksum verified."
}

install_binary() {
    mkdir -p "$INSTALL_DIR"
    cp "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    echo "Installed to ${INSTALL_DIR}/${BINARY_NAME}"
}

print_success() {
    echo ""
    echo "whetstone ${VERSION} installed successfully!"

    case ":$PATH:" in
        *":${INSTALL_DIR}:"*)
            ;;
        *)
            echo ""
            echo "Add ${INSTALL_DIR} to your PATH:"
            echo ""
            echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            echo ""
            echo "Add the line above to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
            ;;
    esac
}

main
