#!/bin/sh
set -e

# Install Whetstone from a GitHub release.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
#
# Pin a specific version:
#   curl -fsSL ... | sh -s -- --version v0.1.0
#
# Override install dir:
#   INSTALL_DIR=/usr/local/bin curl -fsSL ... | sh

REPO="angusbezzina/whetstone"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="whetstone"
REQUESTED_VERSION=""

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --version|-v)
                REQUESTED_VERSION="$2"
                shift 2
                ;;
            --version=*)
                REQUESTED_VERSION="${1#--version=}"
                shift
                ;;
            --help|-h)
                echo "Usage: install.sh [--version v0.1.0]"
                echo ""
                echo "Environment variables:"
                echo "  INSTALL_DIR      Where to place the binary (default: ~/.local/bin)"
                exit 0
                ;;
            *)
                echo "Unknown option: $1" >&2
                exit 1
                ;;
        esac
    done
}

# Test escape hatch: when WHETSTONE_INSTALL_LOCAL_SRC points at an existing
# file, install.sh installs that binary instead of downloading from a GitHub
# release. CI uses this to exercise the install path end-to-end without
# needing a tagged release. Not intended for end-user use.
main() {
    parse_args "$@"
    detect_platform
    if [ -n "${WHETSTONE_INSTALL_LOCAL_SRC:-}" ]; then
        install_from_local_source
        print_success
        return
    fi
    get_version
    download_and_verify
    install_binary
    print_success
}

install_from_local_source() {
    if [ ! -f "$WHETSTONE_INSTALL_LOCAL_SRC" ]; then
        echo "Error: WHETSTONE_INSTALL_LOCAL_SRC does not point at a file: $WHETSTONE_INSTALL_LOCAL_SRC" >&2
        exit 1
    fi
    VERSION="local-$(date +%Y%m%d%H%M%S)"
    mkdir -p "$INSTALL_DIR"
    cp "$WHETSTONE_INSTALL_LOCAL_SRC" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
    echo "Installed local binary to ${INSTALL_DIR}/${BINARY_NAME}"
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

get_version() {
    if [ -n "$REQUESTED_VERSION" ]; then
        VERSION="$REQUESTED_VERSION"
        echo "Requested version: $VERSION"
    else
        echo "Fetching latest release..."
        VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"

        if [ -z "$VERSION" ]; then
            echo "Error: could not determine latest version" >&2
            exit 1
        fi

        echo "Latest version: $VERSION"
    fi
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

main "$@"
