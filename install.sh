#!/usr/bin/env bash
set -euo pipefail

# ── config ────────────────────────────────────────────────────────────────────
REPO="adambie/screenguard"
RELEASES_URL="https://github.com/${REPO}/releases/latest/download"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/screenguard"
DATA_DIR="/var/lib/screenguard"
WEBUI_DIR="/opt/screenguard/webui"
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

# Always read from /dev/tty so interactive prompts work when piped through curl.
ask() {
    local prompt="$1" varname="$2" default="${3:-}"
    [[ -n $default ]] && prompt+=" [${default}]"
    prompt+=": "
    local val
    read -rp "$prompt" val </dev/tty
    printf -v "$varname" '%s' "${val:-$default}"
}

confirm() {
    local prompt="$1" default="${2:-y}"
    local yn
    [[ $default == y ]] && prompt+=" [Y/n] " || prompt+=" [y/N] "
    read -rp "$prompt" yn </dev/tty
    yn="${yn:-$default}"
    [[ $yn =~ ^[Yy] ]]
}

# ── preflight ─────────────────────────────────────────────────────────────────
[[ $EUID -eq 0 ]] || error "Please run as root: sudo bash install.sh"
need_cmd curl
need_cmd systemctl

MODE="install"
for arg in "$@"; do
    case $arg in
        --update)    MODE="update"    ;;
        --uninstall) MODE="uninstall" ;;
        --help|-h)
            echo "Usage: sudo bash install.sh [--update | --uninstall]"
            echo "  (no flag)    Fresh install — interactive"
            echo "  --update     Download latest binaries, restart services"
            echo "  --uninstall  Stop services and remove all ScreenGuard files"
            exit 0
            ;;
    esac
done

# ── architecture ──────────────────────────────────────────────────────────────
ARCH=$(uname -m)
case $ARCH in
    x86_64)  BIN_ARCH="x86_64"  ;;
    aarch64) BIN_ARCH="aarch64" ;;
    armv7l)  error "armv7 is not yet supported. Only x86_64 and aarch64 are available." ;;
    *)       error "Unsupported architecture: $ARCH" ;;
esac

# ════════════════════════════════════════════════════════════════════════════
# UNINSTALL
# ════════════════════════════════════════════════════════════════════════════
if [[ $MODE == uninstall ]]; then
    header "ScreenGuard uninstaller"

    AGENT_INSTALLED=0; SERVER_INSTALLED=0; WEBUI_INSTALLED=0
    [[ -f "${INSTALL_DIR}/screenguard-agent"  ]] && AGENT_INSTALLED=1
    [[ -f "${INSTALL_DIR}/screenguard-server" ]] && SERVER_INSTALLED=1
    [[ -f "${WEBUI_DIR}/app.py"               ]] && WEBUI_INSTALLED=1

    if [[ $AGENT_INSTALLED -eq 0 && $SERVER_INSTALLED -eq 0 && $WEBUI_INSTALLED -eq 0 ]]; then
        warn "No ScreenGuard components found. Nothing to uninstall."
        exit 0
    fi

    header "This will remove:"
    [[ $SERVER_INSTALLED -eq 1 ]] && echo "  • screenguard-server binary and systemd unit"
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        echo "  • screenguard-agent binary and systemd unit"
        echo "  • screenguard-tray binary and XDG autostart entry"
    fi
    [[ $WEBUI_INSTALLED -eq 1 ]] && echo "  • screenguard-webui (${WEBUI_DIR}) and systemd unit"
    echo "  • Systemd units in ${SYSTEMD_DIR}/"
    echo
    echo "  Config and data directories will be removed only if you confirm separately."
    echo

    confirm "Proceed with uninstall?" n || { echo "Aborted."; exit 0; }

    header "Stopping and disabling services"
    for svc in screenguard-server screenguard-agent screenguard-webui; do
        if systemctl is-active --quiet "$svc" 2>/dev/null; then
            systemctl stop "$svc"
            info "Stopped $svc"
        fi
        if systemctl is-enabled --quiet "$svc" 2>/dev/null; then
            systemctl disable "$svc"
            info "Disabled $svc"
        fi
    done

    header "Removing files"
    for bin in screenguard-server screenguard-agent screenguard-tray; do
        if [[ -f "${INSTALL_DIR}/${bin}" ]]; then
            rm -f "${INSTALL_DIR}/${bin}"
            info "Removed ${INSTALL_DIR}/${bin}"
        fi
    done
    for unit in screenguard-server.service screenguard-agent.service screenguard-webui.service; do
        if [[ -f "${SYSTEMD_DIR}/${unit}" ]]; then
            rm -f "${SYSTEMD_DIR}/${unit}"
            info "Removed ${SYSTEMD_DIR}/${unit}"
        fi
    done
    if [[ $WEBUI_INSTALLED -eq 1 && -d "${WEBUI_DIR}" ]]; then
        rm -rf "${WEBUI_DIR}"
        info "Removed ${WEBUI_DIR}"
    fi
    if [[ -f "/etc/xdg/autostart/screenguard-tray.desktop" ]]; then
        rm -f "/etc/xdg/autostart/screenguard-tray.desktop"
        info "Removed /etc/xdg/autostart/screenguard-tray.desktop"
    fi
    if [[ -f "/etc/dbus-1/system.d/screenguard-dbus.conf" ]]; then
        rm -f "/etc/dbus-1/system.d/screenguard-dbus.conf"
        info "Removed D-Bus policy /etc/dbus-1/system.d/screenguard-dbus.conf"
        systemctl reload dbus 2>/dev/null || true
    fi
    systemctl daemon-reload

    echo
    if confirm "Also remove config directory ${CONFIG_DIR}/ (agent.toml, server.toml)?" n; then
        rm -rf "${CONFIG_DIR}"
        info "Removed ${CONFIG_DIR}"
    fi
    if [[ -d "${DATA_DIR}" ]]; then
        if confirm "Also remove data directory ${DATA_DIR}/ (database — this deletes all profiles, schedules, usage history)?" n; then
            rm -rf "${DATA_DIR}"
            info "Removed ${DATA_DIR}"
        fi
    fi

    header "Done — ScreenGuard has been removed."
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# UPDATE
# ════════════════════════════════════════════════════════════════════════════
if [[ $MODE == update ]]; then
    header "ScreenGuard updater"
    info "Architecture: ${ARCH}"

    AGENT_INSTALLED=0; SERVER_INSTALLED=0; WEBUI_INSTALLED=0
    [[ -f "${INSTALL_DIR}/screenguard-agent"  ]] && AGENT_INSTALLED=1
    [[ -f "${INSTALL_DIR}/screenguard-server" ]] && SERVER_INSTALLED=1
    [[ -f "${WEBUI_DIR}/app.py"               ]] && WEBUI_INSTALLED=1

    if [[ $AGENT_INSTALLED -eq 0 && $SERVER_INSTALLED -eq 0 ]]; then
        error "No ScreenGuard binaries found in ${INSTALL_DIR}. Run without --update to do a fresh install."
    fi

    header "Will update:"
    [[ $SERVER_INSTALLED -eq 1 ]] && echo "  • screenguard-server"
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        echo "  • screenguard-agent"
        echo "  • screenguard-tray"
    fi
    [[ $WEBUI_INSTALLED -eq 1 ]] && echo "  • screenguard-webui"
    echo "  • systemd service units"
    echo "  Configs in ${CONFIG_DIR}/ will NOT be touched."
    echo
    confirm "Proceed?" || { echo "Aborted."; exit 0; }

    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT

    header "Downloading latest binaries"
    if [[ $SERVER_INSTALLED -eq 1 ]]; then
        info "Downloading screenguard-server..."
        download "${RELEASES_URL}/screenguard-server-${BIN_ARCH}" "${TMP}/screenguard-server"
        chmod +x "${TMP}/screenguard-server"
        download "${RELEASES_URL}/screenguard-server.service" "${TMP}/screenguard-server.service"
    fi
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        info "Downloading screenguard-agent..."
        download "${RELEASES_URL}/screenguard-agent-${BIN_ARCH}" "${TMP}/screenguard-agent"
        chmod +x "${TMP}/screenguard-agent"
        download "${RELEASES_URL}/screenguard-agent.service" "${TMP}/screenguard-agent.service"
        info "Downloading screenguard-tray..."
        download "${RELEASES_URL}/screenguard-tray-${BIN_ARCH}" "${TMP}/screenguard-tray"
        chmod +x "${TMP}/screenguard-tray"
        download "${RELEASES_URL}/screenguard-tray.desktop" "${TMP}/screenguard-tray.desktop"
        download "${RELEASES_URL}/screenguard-dbus.conf" "${TMP}/screenguard-dbus.conf"
    fi
    if [[ $WEBUI_INSTALLED -eq 1 ]]; then
        info "Downloading screenguard-webui..."
        download "${RELEASES_URL}/screenguard-webui.tar.gz" "${TMP}/screenguard-webui.tar.gz"
        download "${RELEASES_URL}/screenguard-webui.service" "${TMP}/screenguard-webui.service"
    fi

    header "Stopping services"
    if [[ $SERVER_INSTALLED -eq 1 ]]; then
        systemctl stop screenguard-server 2>/dev/null && info "Stopped screenguard-server" || true
    fi
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        systemctl stop screenguard-agent 2>/dev/null && info "Stopped screenguard-agent" || true
    fi
    if [[ $WEBUI_INSTALLED -eq 1 ]]; then
        systemctl stop screenguard-webui 2>/dev/null && info "Stopped screenguard-webui" || true
    fi

    header "Installing"
    if [[ $SERVER_INSTALLED -eq 1 ]]; then
        cp "${TMP}/screenguard-server" "${INSTALL_DIR}/screenguard-server"
        cp "${TMP}/screenguard-server.service" "${SYSTEMD_DIR}/screenguard-server.service"
        info "Updated screenguard-server"
    fi
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        cp "${TMP}/screenguard-agent" "${INSTALL_DIR}/screenguard-agent"
        cp "${TMP}/screenguard-agent.service" "${SYSTEMD_DIR}/screenguard-agent.service"
        info "Updated screenguard-agent"
        cp "${TMP}/screenguard-tray" "${INSTALL_DIR}/screenguard-tray"
        cp "${TMP}/screenguard-tray.desktop" "/etc/xdg/autostart/screenguard-tray.desktop"
        cp "${TMP}/screenguard-dbus.conf" "/etc/dbus-1/system.d/screenguard-dbus.conf"
        systemctl reload dbus 2>/dev/null || true
        info "Updated screenguard-tray"
    fi
    if [[ $WEBUI_INSTALLED -eq 1 ]]; then
        tar -xzf "${TMP}/screenguard-webui.tar.gz" -C "${WEBUI_DIR}"
        "${WEBUI_DIR}/venv/bin/pip" install --quiet -r "${WEBUI_DIR}/requirements.txt"
        cp "${TMP}/screenguard-webui.service" "${SYSTEMD_DIR}/screenguard-webui.service"
        info "Updated screenguard-webui"
    fi

    header "Restarting services"
    systemctl daemon-reload
    if [[ $SERVER_INSTALLED -eq 1 ]]; then
        systemctl restart screenguard-server
        info "screenguard-server restarted"
    fi
    if [[ $AGENT_INSTALLED -eq 1 ]]; then
        systemctl restart screenguard-agent
        info "screenguard-agent restarted"
    fi
    if [[ $WEBUI_INSTALLED -eq 1 ]]; then
        systemctl restart screenguard-webui
        info "screenguard-webui restarted"
    fi

    header "Done — ScreenGuard updated to latest release."
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# INSTALL (fresh)
# ════════════════════════════════════════════════════════════════════════════
header "ScreenGuard installer"
echo "  GitHub: https://github.com/${REPO}"
echo
info "Architecture: ${ARCH}"

# ── what to install ───────────────────────────────────────────────────────────
header "What would you like to install?"
echo "  1) Agent only   — managed machine (child's computer)"
echo "  2) Server only  — management server"
echo "  3) Both         — server + agent on this machine"
echo
while true; do
    ask "Enter choice [1-3]" choice
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
        ask "Enter choice [1-2]" disc
        case $disc in
            1) SERVER_URL=""; break ;;
            2)
                ask "Server URL (e.g. http://192.168.1.100:8080)" SERVER_URL
                [[ -n $SERVER_URL ]] && break || warn "URL cannot be empty"
                ;;
            *) warn "Please enter 1 or 2" ;;
        esac
    done
elif [[ ${INSTALL_AGENT:-0} -eq 1 && ${INSTALL_SERVER:-0} -eq 1 ]]; then
    SERVER_URL="http://127.0.0.1:8080"
fi

# ── server: port ──────────────────────────────────────────────────────────────
SERVER_PORT=8080
if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    header "Server configuration"
    ask "Listen port" SERVER_PORT "8080"
fi

# ── web UI ────────────────────────────────────────────────────────────────────
INSTALL_WEBUI=0
WEBUI_PORT=5000
if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    echo
    if confirm "Install web UI? (requires Python 3)" y; then
        need_cmd python3
        INSTALL_WEBUI=1
        ask "Web UI listen port" WEBUI_PORT "5000"
    fi
fi

# ── confirm ───────────────────────────────────────────────────────────────────
header "Summary"
[[ ${INSTALL_SERVER:-0} -eq 1 ]] && echo "  • Install server  (port ${SERVER_PORT})"
[[ $INSTALL_WEBUI -eq 1 ]] && echo "  • Install web UI  (port ${WEBUI_PORT})"
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
    info "Downloading screenguard-tray..."
    download "${RELEASES_URL}/screenguard-tray-${BIN_ARCH}" "${TMP}/screenguard-tray"
    chmod +x "${TMP}/screenguard-tray"
    download "${RELEASES_URL}/screenguard-tray.desktop" "${TMP}/screenguard-tray.desktop"
    download "${RELEASES_URL}/screenguard-dbus.conf" "${TMP}/screenguard-dbus.conf"
fi

if [[ $INSTALL_WEBUI -eq 1 ]]; then
    info "Downloading screenguard-webui..."
    download "${RELEASES_URL}/screenguard-webui.tar.gz" "${TMP}/screenguard-webui.tar.gz"
    download "${RELEASES_URL}/screenguard-webui.service" "${TMP}/screenguard-webui.service"
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

    if [[ $INSTALL_WEBUI -eq 1 ]]; then
        mkdir -p "${WEBUI_DIR}"
        tar -xzf "${TMP}/screenguard-webui.tar.gz" -C "${WEBUI_DIR}"
        info "Extracted web UI to ${WEBUI_DIR}"
        python3 -m venv "${WEBUI_DIR}/venv"
        "${WEBUI_DIR}/venv/bin/pip" install --quiet -r "${WEBUI_DIR}/requirements.txt"
        info "Installed Python dependencies"
        if [[ ! -f "${CONFIG_DIR}/webui.env" ]]; then
            WEBUI_SECRET=$(python3 -c "import secrets; print(secrets.token_hex(32))")
            cat > "${CONFIG_DIR}/webui.env" <<EOF
SERVER_URL=http://127.0.0.1:${SERVER_PORT}
SECRET_KEY=${WEBUI_SECRET}
UI_PORT=${WEBUI_PORT}
EOF
            chmod 600 "${CONFIG_DIR}/webui.env"
            info "Created ${CONFIG_DIR}/webui.env"
        else
            warn "Config already exists, skipping: ${CONFIG_DIR}/webui.env"
        fi
        cp "${TMP}/screenguard-webui.service" "${SYSTEMD_DIR}/screenguard-webui.service"
        info "Installed systemd unit: screenguard-webui.service"
    fi
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    cp "${TMP}/screenguard-agent" "${INSTALL_DIR}/screenguard-agent"
    info "Installed ${INSTALL_DIR}/screenguard-agent"
    cp "${TMP}/screenguard-tray" "${INSTALL_DIR}/screenguard-tray"
    info "Installed ${INSTALL_DIR}/screenguard-tray"
    mkdir -p /etc/xdg/autostart
    cp "${TMP}/screenguard-tray.desktop" "/etc/xdg/autostart/screenguard-tray.desktop"
    info "Installed /etc/xdg/autostart/screenguard-tray.desktop"
    cp "${TMP}/screenguard-dbus.conf" "/etc/dbus-1/system.d/screenguard-dbus.conf"
    systemctl reload dbus 2>/dev/null || true
    info "Installed D-Bus policy /etc/dbus-1/system.d/screenguard-dbus.conf"

    mkdir -p "${DATA_DIR}"

    if [[ ! -f "${CONFIG_DIR}/agent.toml" ]]; then
        # Derive web UI URL: same host as server, web UI port.
        if [[ -n $SERVER_URL ]]; then
            _server_host=$(echo "$SERVER_URL" | sed 's|^[a-z]*://||;s|:.*||;s|/.*||')
            _webui_port=${WEBUI_PORT:-5000}
            cat > "${CONFIG_DIR}/agent.toml" <<EOF
server_url = "${SERVER_URL}"
webui_url = "http://${_server_host}:${_webui_port}"
EOF
        else
            cat > "${CONFIG_DIR}/agent.toml" <<EOF
# server_url = "http://192.168.1.100:8080"
# Leave commented out to use mDNS auto-discovery.
# webui_url = "http://192.168.1.100:5000"
EOF
        fi
        info "Created ${CONFIG_DIR}/agent.toml"
    else
        warn "Config already exists, skipping: ${CONFIG_DIR}/agent.toml"
    fi

    cp "${TMP}/screenguard-agent.service" "${SYSTEMD_DIR}/screenguard-agent.service"
    info "Installed systemd unit: screenguard-agent.service"
fi

# ── enable & start ────────────────────────────────────────────────────────────
header "Starting services"
systemctl daemon-reload

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    systemctl enable --now screenguard-server
    info "screenguard-server enabled and started"
fi

if [[ $INSTALL_WEBUI -eq 1 ]]; then
    systemctl enable --now screenguard-webui
    info "screenguard-webui enabled and started"
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    systemctl enable --now screenguard-agent
    info "screenguard-agent enabled and started"
fi

# ── done ──────────────────────────────────────────────────────────────────────
header "Done!"

if [[ ${INSTALL_SERVER:-0} -eq 1 ]]; then
    echo -e "  Server running on port ${SERVER_PORT}"
    if [[ $INSTALL_WEBUI -eq 1 ]]; then
        LOCAL_IP=$(hostname -I 2>/dev/null | awk '{print $1}')
        echo -e "  Web UI:  http://${LOCAL_IP:-localhost}:${WEBUI_PORT}"
        echo -e "  Web UI logs:  journalctl -u screenguard-webui -f"
    fi
    echo -e "  Logs:    journalctl -u screenguard-server -f"
    echo -e "  Config:  ${CONFIG_DIR}/server.toml"
    echo
    if command -v ufw &>/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
        warn "ufw firewall is active. Agents on other machines will not reach the server until you run:"
        warn "  sudo ufw allow ${SERVER_PORT}/tcp"
        [[ $INSTALL_WEBUI -eq 1 ]] && warn "  sudo ufw allow ${WEBUI_PORT}/tcp"
    fi
fi

if [[ ${INSTALL_AGENT:-0} -eq 1 ]]; then
    echo -e "  Logs:    journalctl -u screenguard-agent -f"
    echo -e "  Config:  ${CONFIG_DIR}/agent.toml"
    echo -e "  Reset:   screenguard-agent --reset"
fi

echo
echo -e "  To update later:    sudo bash install.sh --update"
echo -e "  To uninstall later: sudo bash install.sh --uninstall"
echo
