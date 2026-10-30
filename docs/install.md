# Installing and running ScreenGuard

The [README](../README.md#getting-started) covers the basic setup: one server, one or more children's computers on the same network. This page has the other options and the configuration reference.

- [Installer options](#installer-options) — update, uninstall
- [Running the server in Docker](#running-the-server-in-docker) — SQLite or Postgres
- [Firewall](#firewall)
- [Configuration](#configuration) — `server.toml`, `agent.toml`, environment variables
- [Web UI without the installer](#web-ui-without-the-installer)
- [Logs](#logs) and [re-pairing an agent](#re-pairing-an-agent)

For the hosted service instead of your own server, see [Cloud mode](cloud.md).

## Installer options

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash
```

The installer asks whether to install the **agent**, the **server** (optionally with the **web UI**, port 5000), or **both**, then configures and starts the systemd services. For an agent it asks how to reach the server: mDNS auto-discovery, a fixed URL, or a cloud account.

**Update** — downloads the latest binaries, replaces them and restarts the services; configs are not touched:

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash -s -- --update
```

Agents can also be updated from the web UI (Agents → Update); see [what the agent does](how-it-works.md#what-a-server-can-ask-the-agent-to-do).

**Uninstall** — you are asked separately whether to remove the config directory and the database:

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash -s -- --uninstall
```

## Running the server in Docker

Two compose files are included. The **agent always runs natively** on the child's computer — it works with the desktop session over D-Bus and cannot run in a container.

### SQLite (default) — `docker-compose.yml`

Uses host networking so mDNS auto-discovery works — agents on the LAN find the server without any configuration. The database lives in a named volume (`screenguard-data`) and survives restarts and image updates.

```bash
# 1. Get the compose file (images are pulled from ghcr.io — no clone or local build needed)
curl -fsSLO https://raw.githubusercontent.com/adambie/screenguard/main/docker-compose.yml

# 2. Edit docker-compose.yml and replace "change-me-in-production" with strong secrets

# 3. Start
docker compose up -d

# 4. Open the web UI: http://<server-ip>:5000
```

Images are published to GHCR for `linux/amd64` and `linux/arm64` on every release. To pin a release instead of tracking `latest`, change the image tags:

```yaml
image: ghcr.io/adambie/screenguard-server:v0.10.17
image: ghcr.io/adambie/screenguard-webui:v0.10.17
```

To build from source instead, use the commented `build:` lines in the compose file.

**Changing the server port:** with host networking a `ports:` mapping has no effect, so set `SCREENGUARD_SERVER_PORT` on the server (e.g. `"8089"`) and point the web UI's `SERVER_URL` at the same port (`http://127.0.0.1:8089`). Agents using mDNS pick up the new port automatically; agents with a fixed `server_url` need it updated.

### Postgres — `docker-compose.postgres.yml`

Runs a Postgres container next to the server. It uses bridge networking, so **mDNS auto-discovery does not work** — point each agent at the server:

```toml
# /etc/screenguard/agent.toml on each child's computer
server_url = "ws://<server-host-ip>:8080"
```

```bash
# Download docker-compose.postgres.yml, edit it and replace "change-me-in-production" with strong secrets
docker compose -f docker-compose.postgres.yml up -d
```

`DATABASE_URL` decides the database: a `postgres://` URL uses Postgres; a `sqlite:` path (or nothing) uses SQLite.

### Bridge networking (SQLite)

If host networking is not an option (rootless Podman, a non-Linux Docker host, strict isolation), `docker-compose.yml` has a commented-out bridge configuration. The same mDNS caveat applies: set the server address on every child's computer.

### Updating

```bash
docker compose pull
docker compose up -d
```

## Firewall

If the server and the agents run on **different machines**, the agents need to reach the server's port (default **8080**, TCP), and you need the web UI's port (default **5000**).

```bash
sudo ufw allow 8080/tcp                                   # ufw (Ubuntu/Debian)
sudo firewall-cmd --permanent --add-port=8080/tcp && sudo firewall-cmd --reload   # firewalld (Fedora/RHEL)
```

**mDNS auto-discovery** uses UDP multicast on port 5353 and works out of the box on most home networks. If agents don't find the server, use a fixed URL (the installer offers it) or allow mDNS/multicast between the machines.

## Configuration

### Server — `/etc/screenguard/server.toml`

```toml
listen_addr      = "0.0.0.0"
listen_port      = 8080
db_path          = "/var/lib/screenguard/server.db"  # SQLite path (ignored when database_url is set)
# database_url   = "postgres://user:pass@host:5432/screenguard"  # use Postgres instead of SQLite
enable_mdns      = true
jwt_expiry_hours = 24
```

### Agent — `/etc/screenguard/agent.toml`

```toml
# Leave commented out to use mDNS auto-discovery:
# server_url = "http://192.168.1.100:8080"

heartbeat_interval  = 10    # seconds
user_scan_interval  = 300   # seconds
cache_ttl_hours     = 48
min_uid             = 1000  # ignore system accounts below this UID
app_tracking        = true  # per-app usage reports; false = don't scan, store or send them
```

### Environment variables

| Variable | Description |
|---|---|
| `SCREENGUARD_SERVER_CONFIG` | Path to the server config file |
| `SCREENGUARD_AGENT_CONFIG` | Path to the agent config file |
| `SCREENGUARD_SERVER_DB` | SQLite database path (default: `/var/lib/screenguard/server.db`) |
| `DATABASE_URL` | Full database URL — overrides `db_path`. `postgres://…` for Postgres or `sqlite:/path` for SQLite |
| `SCREENGUARD_SERVER_JWT_SECRET` | JWT signing secret |
| `SCREENGUARD_SERVER_PORT` | Listen port — overrides `listen_port` (default: `8080`) |

## Web UI without the installer

The web UI is a small Flask app in `webui/`. The installer and the Docker images run it as a service; to run it by hand:

```bash
cd webui
SERVER_URL=http://localhost:8080 uv run --with flask --with requests python app.py
```

## Logs

```bash
journalctl -u screenguard-server -f
journalctl -u screenguard-webui -f
journalctl -u screenguard-agent -f
```

## Re-pairing an agent

To pair an agent again (e.g. after moving to a different server):

```bash
sudo screenguard-agent --reset
sudo systemctl restart screenguard-agent
```

To show the current pairing code again: `sudo screenguard-agent --show-pairing-code` (agents older than 0.10.17: `sudo journalctl -u screenguard-agent | grep 'PAIRING CODE' | tail -1`).
