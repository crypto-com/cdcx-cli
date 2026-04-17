#!/bin/sh
# Local install — copies the release binary from target/release to ~/.local/bin.
# Usage: ./install-local.sh
set -e

BINARY="cdcx"
SOURCE="target/release/${BINARY}"
INSTALL_DIR="${HOME}/.local/bin"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { printf "${CYAN}${BOLD}==>${NC} %s\n" "$1"; }
ok()    { printf "${GREEN}${BOLD} ✓${NC} %s\n" "$1"; }
error() { printf "${RED}${BOLD}Error:${NC} %s\n" "$1" >&2; exit 1; }

if [ ! -f "$SOURCE" ]; then
    error "Binary not found at ${SOURCE}. Run 'cargo build --release' first."
fi

info "Installing ${BINARY} from ${SOURCE}..."
FILE_INFO=$(file -b "$SOURCE" 2>/dev/null || echo "unknown")
info "Binary: $(ls -lh "$SOURCE" | awk '{print $5}') — ${FILE_INFO}"

mkdir -p "$INSTALL_DIR"
cp "$SOURCE" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

ok "Installed to ${INSTALL_DIR}/${BINARY}"

# Verify
RESOLVED=$(command -v "$BINARY" 2>/dev/null || true)
if [ "$RESOLVED" = "${INSTALL_DIR}/${BINARY}" ]; then
    printf "\n"
    ok "cdcx is ready:"
    printf "  ${BOLD}cdcx --help${NC}\n"
    printf "  ${BOLD}cdcx tui${NC}\n"
    printf "\n"
elif [ -n "$RESOLVED" ]; then
    printf "\n"
    ok "Installed to ${INSTALL_DIR}/${BINARY}, but your shell resolves cdcx to ${RESOLVED}."
    printf "  To use the new version, either remove the old binary or put ${INSTALL_DIR} earlier in PATH:\n"
    printf "  ${BOLD}export PATH=\"\${HOME}/.local/bin:\${PATH}\"${NC}\n"
    printf "\n"
else
    printf "\n"
    ok "Installed, but ${INSTALL_DIR} is not in your PATH. Add it:"
    printf "  ${BOLD}export PATH=\"\${HOME}/.local/bin:\${PATH}\"${NC}\n"
    printf "\n"
fi
