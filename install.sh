#!/usr/bin/env bash
# CogZ install script — downloads the latest release binary from GitHub.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/main/install.sh | bash
#
# Or:
#   ./install.sh
#
# Installs to ~/.local/bin/cogz (or /usr/local/bin/cogz if run as root).
# Creates ~/.local/share/cogz/models/ for the model cache.

set -euo pipefail

REPO="balaianu/CogZ"
GITHUB_API="https://api.github.com/repos/${REPO}/releases/latest"

# Detect architecture.
ARCH=$(uname -m)
OS=$(uname -s | tr '[:upper:]' '[:lower:]')

case "${OS}-${ARCH}" in
    linux-x86_64)  ASSET="cogz-x86_64-unknown-linux-gnu" ;;
    linux-aarch64) ASSET="cogz-aarch64-unknown-linux-gnu" ;;
    darwin-arm64)  ASSET="cogz-aarch64-apple-darwin" ;;
    *)
        echo "Unsupported platform: ${OS}-${ARCH}"
        echo "Supported: linux-x86_64, linux-aarch64, darwin-arm64 (Apple Silicon)"
        echo "Windows: use install.ps1"
        echo "macOS Intel: not supported (use Rosetta 2 or FTS-only mode)"
        exit 1
        ;;
esac

# Determine install directory.
if [ "$(id -u)" -eq 0 ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "${INSTALL_DIR}"
fi

echo "Installing CogZ for ${OS}-${ARCH}..."

# Fetch the latest release download URL.
echo "Fetching latest release..."
DOWNLOAD_URL=$(curl -fsSL "${GITHUB_API}" | grep -o "browser_download_url.*${ASSET}\"" | head -1 | sed 's/browser_download_url: "//;s/"$//')

if [ -z "${DOWNLOAD_URL}" ]; then
    echo "Error: could not find ${ASSET} in the latest release."
    echo "Check https://github.com/${REPO}/releases for available assets."
    exit 1
fi

# Download to a temporary file.
TMP_FILE=$(mktemp /tmp/cogz-download-XXXXXX)
echo "Downloading ${ASSET}..."
curl -fsSL -o "${TMP_FILE}" "${DOWNLOAD_URL}"

# Verify checksum from SHA256SUMS.
CHECKSUM_URL=$(curl -fsSL "${GITHUB_API}" | grep -o "browser_download_url.*SHA256SUMS\"" | head -1 | sed 's/browser_download_url: "//;s/"$//')
if [ -n "${CHECKSUM_URL}" ]; then
    TMP_SUMS=$(mktemp /tmp/cogz-sums-XXXXXX)
    curl -fsSL -o "${TMP_SUMS}" "${CHECKSUM_URL}"
    EXPECTED_HASH=$(grep "  ${ASSET}$" "${TMP_SUMS}" | awk '{print $1}')
    if [ -z "${EXPECTED_HASH}" ]; then
        echo "Error: SHA256SUMS downloaded but no entry for ${ASSET}."
        echo "Refusing to install unverified binary."
        rm -f "${TMP_FILE}" "${TMP_SUMS}"
        exit 1
    fi
    ACTUAL_HASH=$(sha256sum "${TMP_FILE}" | awk '{print $1}')
    if [ "${EXPECTED_HASH}" != "${ACTUAL_HASH}" ]; then
        echo "Error: checksum mismatch."
        echo "  Expected: ${EXPECTED_HASH}"
        echo "  Actual:   ${ACTUAL_HASH}"
        rm -f "${TMP_FILE}" "${TMP_SUMS}"
        exit 1
    fi
    echo "Checksum verified."
    rm -f "${TMP_SUMS}"
else
    echo "WARNING: SHA256SUMS not found in release. Installing without checksum verification."
fi

# Make executable.
chmod +x "${TMP_FILE}"

# Install.
mv "${TMP_FILE}" "${INSTALL_DIR}/cogz"
echo "Installed to ${INSTALL_DIR}/cogz"

# Create model cache directory.
MODEL_DIR="${HOME}/.local/share/cogz/models"
mkdir -p "${MODEL_DIR}"
echo "Model cache: ${MODEL_DIR}"

# Check PATH.
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        ;;
    *)
        echo ""
        echo "WARNING: ${INSTALL_DIR} is not on your PATH."
        echo "Add this line to your shell config (~/.bashrc, ~/.zshrc, etc.):"
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        ;;
esac

# Verify.
echo ""
echo "Verification:"
"${INSTALL_DIR}/cogz" --version

echo ""
echo "Next steps:"
echo "  cd ~/your-project"
echo "  cogz init"
echo "  cogz index"
echo ""
echo "To enable agent hooks (Devin/Claude Code):"
echo "  See docs/packaging.md → Hook integration"
echo "  Add cogz capture-event entries to your agent's hook config"
echo ""
echo "Done."
