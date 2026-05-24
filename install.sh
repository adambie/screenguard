#!/usr/bin/env bash
set -euo pipefail

# ── config ────────────────────────────────────────────────────────────────────
REPO="adambie/screenguard"
RELEASES_URL="https://github.com/${REPO}/releases/latest/download"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/screenguard"
DATA_DIR="/var/lib/screenguard"
SYSTEMD_DIR="/etc/systemd/system"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; RESET='\033[0m'

# ── helpers ───────────────────────────────────────────────────────────────────
info()    { echo -e "${GREEN}[✓]${RESET} $*"; }
warn()    { echo -e "${YELLOW}[!]${RESET} $*"; }
error()   { echo -e "${RED}[✗]${RESET} $*" >&2; exit 1; }
header()  { echo -e "\n${BOLD}$*${RESET}"; }

need_cmd() { command -v "$1" &>/dev/null || error "Required command not found: $1"; }
download() { curl -fsSL "$1" -o "$2" || error "Download failed: $1"; }

confirm() {
    local prompt="$1" default="${2:-y}"
    local yn
    [[ $default == y ]] && prompt+=" [Y/n] " || prompt+=" [y/N] "
    read -rp "$prompt" yn
    yn="${yn:-$default}"
    [[ $yn =~ ^[Yy] ]]
}

# ── preflight ─────────────────────────────────────────────────────────────────
[[ $EUID -eq 0 ]] || error "Please run as root: sudo bash install.sh"
need_cmd curl
need_cmd systemctl

header "ScreenGuard installer"
echo "  GitHub: https://github.com/${REPO}"
echo

# ── architecture ──────────────────────────────────────────────────────────────
ARCH=$(uname -m)
case $ARCH in
    x86_64)  BIN_ARCH="x86_64"  ;;
    aarch64) BIN_ARCH="aarch64" ;;
    armv7l)  error "armv7 is not yet supported. Only x86_64 and aarch64 are available." ;;
    *)       error "Unsupported architecture: $ARCH" ;;
esac
info "Architecture: ${ARCH}"

# ── what to install ───────────────────────────────────────────────────────────
header "What would you like to install?"
echo "  1) Agent only   — managed machine (child's computer)"
echo "  2) Server only  — management server"
echo "  3) Both         — server + agent on this machine"
echo
while true; do
    read -rp "Enter choice [1-3]: " choice
    case $choice in
        1) INSTALL_AGENT=1; INSTALL_SERVER=0; break ;;
        2) INSTALL_AGENT=0; INSTALL_SERVER=1; break ;;
        3) INSTALL_AGENT=1; INSTALL_SERVER=1; break ;;
        *) warn "Please enter 1, 2, or 3" ;;
    esac
done

# ── agent: server discovery ───────────────────────────────────────────────────
SERVER_URL=""
if [[ ${INSTALL_AGENT:-0} -eq 1 && ${INSTALL_SERVER:-0} -eq 0 ]]; then
    header "How should the agent find the server?"
    echo "  1) mDNS auto-discovery  — server broadcasts itself on the local network"
    echo "  2) Fixed URL            — you know the server's address"
    echo
    while true; do
        read -rp "Enter choice [1-2]: " disc
        case $disc in
            1) SERVER_URL=""; break ;;
            2)
                read -rp "Server URL (e.g. http://192.168.1.100:8080): " SERVER_URL
                [[ -n $SERVER_URL ]] && break || warn "URL cannot be empty"
                ;;
            *) warn "Please enter 1 or 2" ;;
        esac
    done
elif [[ ${INSTALL_AGENT:-0} -eq 1 && ${INSTALL_SERVER:-0} -eq 1 ]]; then
    # Both on same machine — agent connects to local server
    SERVER_URL="http://127.0.0.1:8080"
fi

# ── server: port ──────────────────────────────────────────────────────────────
SERVER_PORT=8080
if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    header "Server configuration"
    read -rp "Listen port [8080]: " input_port
    SERVER_PORT="${input_port:-8080}"
fi

# ── confirm ───────────────────────────────────────────────────────────────────
header "Summary"
[[ ${INSTALL_SERVER:-0} -eq 1 ]] && echo "  • Install server  (port ${SERVER_PORT})"
if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    if [[ -n $SERVER_URL ]]; then
        echo "  • Install agent   (server: ${SERVER_URL})"
    else
        echo "  • Install agent   (mDNS auto-discovery)"
    fi
fi
echo "  • Binaries    → ${INSTALL_DIR}/"
echo "  • Config      → ${CONFIG_DIR}/"
[[ ${INSTALL_SERVER:-0} -eq 1 ]] && echo "  • Database    → ${DATA_DIR}/"
echo
confirm "Proceed?" || { echo "Aborted."; exit 0; }

# ── download ──────────────────────────────────────────────────────────────────
header "Downloading binaries"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    info "Downloading screenguard-server..."
    download "${RELEASES_URL}/screenguard-server-${BIN_ARCH}" "${TMP}/screenguard-server"
    chmod +x "${TMP}/screenguard-server"
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    info "Downloading screenguard-agent..."
    download "${RELEASES_URL}/screenguard-agent-${BIN_ARCH}" "${TMP}/screenguard-agent"
    chmod +x "${TMP}/screenguard-agent"
fi

download "${RELEASES_URL}/screenguard-server.service" "${TMP}/screenguard-server.service"
download "${RELEASES_URL}/screenguard-agent.service"  "${TMP}/screenguard-agent.service"

# ── install ───────────────────────────────────────────────────────────────────
header "Installing"

mkdir -p "${CONFIG_DIR}"

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    cp "${TMP}/screenguard-server" "${INSTALL_DIR}/screenguard-server"
    info "Installed ${INSTALL_DIR}/screenguard-server"

    mkdir -p "${DATA_DIR}"

    # Write server config (only if it doesn't exist yet)
    if [[ ! -f "${CONFIG_DIR}/server.toml" ]]; then
        cat > "${CONFIG_DIR}/server.toml" <<EOF
listen_addr = "0.0.0.0"
listen_port = ${SERVER_PORT}
db_path     = "${DATA_DIR}/server.db"
enable_mdns = true
EOF
        info "Created ${CONFIG_DIR}/server.toml"
    else
        warn "Config already exists, skipping: ${CONFIG_DIR}/server.toml"
    fi

    cp "${TMP}/screenguard-server.service" "${SYSTEMD_DIR}/screenguard-server.service"
    info "Installed systemd unit: screenguard-server.service"
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    cp "${TMP}/screenguard-agent" "${INSTALL_DIR}/screenguard-agent"
    info "Installed ${INSTALL_DIR}/screenguard-agent"

    mkdir -p "${DATA_DIR}"

    # Write agent config (only if it doesn't exist yet)
    if [[ ! -f "${CONFIG_DIR}/agent.toml" ]]; then
        if [[ -n $SERVER_URL ]]; then
            cat > "${CONFIG_DIR}/agent.toml" <<EOF
server_url = "${SERVER_URL}"
EOF
        else
            cat > "${CONFIG_DIR}/agent.toml" <<EOF
# server_url = "http://192.168.1.100:8080"
# Leave commented out to use mDNS auto-discovery.
EOF
        fi
        info "Created ${CONFIG_DIR}/agent.toml"
    else
        warn "Config already exists, skipping: ${CONFIG_DIR}/agent.toml"
    fi

    cp "${TMP}/screenguard-agent.service" "${SYSTEMD_DIR}/screenguard-agent.service"
    info "Installed systemd unit: screenguard-agent.service"
fi

# ── update config paths to /etc/screenguard ───────────────────────────────────
# Patch ExecStart EnvironmentFile to point at new config dir
if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    sed -i "s|/etc/screenguard/server.env|${CONFIG_DIR}/server.env|g" \
        "${SYSTEMD_DIR}/screenguard-server.service"
fi

# ── enable & start ────────────────────────────────────────────────────────────
header "Starting services"
systemctl daemon-reload

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    systemctl enable --now screenguard-server
    info "screenguard-server enabled and started"
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    systemctl enable --now screenguard-agent
    info "screenguard-agent enabled and started"
fi

# ── done ──────────────────────────────────────────────────────────────────────
header "Done!"

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    echo -e "  Server running on port ${SERVER_PORT}"
    echo -e "  Web UI:  cd webui && SERVER_URL=http://localhost:${SERVER_PORT} uv run --with flask --with requests python app.py"
    echo -e "  Logs:    journalctl -u screenguard-server -f"
    echo -e "  Config:  ${CONFIG_DIR}/server.toml"
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    echo -e "  Logs:    journalctl -u screenguard-agent -f"
    echo -e "  Config:  ${CONFIG_DIR}/agent.toml"
    echo -e "  Reset:   screenguard-agent --reset"
fi

echo
