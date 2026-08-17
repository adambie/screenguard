# Web Content Filtering — Technical Design & Implementation Plan

## Architecture summary

DNS queries from managed UIDs are intercepted via nftables DNAT and forwarded to a per-UID UDP/TCP proxy running inside the agent process. The proxy checks each queried domain against the profile's blocklist and returns NXDOMAIN for matches; all other queries are forwarded to the real system resolver. A secondary nftables filter blocks known DoH provider IPs to prevent encrypted DNS bypass.

```
managed user (UID 1000)
    │  DNS query (UDP/TCP port 53)
    ▼
nftables DNAT (meta skuid 1000 → redirect to :5354)
    │
    ▼
agent DNS proxy (127.0.0.1:5354, knows it serves UID 1000)
    ├── domain in blocklist?  →  return NXDOMAIN
    └── not blocked?          →  forward to upstream resolver → return response

nftables filter (separate chain)
    └── meta skuid 1000, ip daddr @doh_servers → drop   (DoH bypass prevention)
```

The agent runs as root (UID 0). nftables rules only redirect/filter non-root managed UIDs, so the proxy's own upstream forwarding is not caught in the redirect loop.

---

## Capability check

Feature availability is determined entirely by the presence of the `nft` binary. No other runtime dependency is introduced.

- **Install script**: warns if `nft` is absent, prints install hint.
- **Agent startup**: re-checks on every start, stores result in SQLite.
- **Server**: receives capability via `agent_hello`, stores it, exposes it via REST.
- **Web UI / mobile**: reads capability from agent detail API, shows badge.

---

## nftables rule structure

```
table inet screenguard {

    # DNS redirect per managed UID (NAT hook, created per user)
    chain dns_redirect {
        type nat hook output priority -100; policy accept;
        # one rule per managed UID:
        meta skuid 1000 udp dport 53 redirect to :5354
        meta skuid 1000 tcp dport 53 redirect to :5354
        meta skuid 1001 udp dport 53 redirect to :5355
        meta skuid 1001 tcp dport 53 redirect to :5355
    }

    # DoH provider block (filter hook)
    chain doh_block {
        type filter hook output priority 0; policy accept;
        # one rule per managed UID:
        meta skuid 1000 ip  daddr @doh_ipv4 drop
        meta skuid 1000 ip6 daddr @doh_ipv6 drop
    }

    set doh_ipv4 {
        type ipv4_addr;
        # Cloudflare, Google, Quad9, NextDNS, AdGuard
        elements = {
            1.1.1.1, 1.0.0.1,
            8.8.8.8, 8.8.4.4,
            9.9.9.9, 149.112.112.112,
            45.90.28.0, 45.90.30.0,
            94.140.14.14, 94.140.15.15
        }
    }

    set doh_ipv6 {
        type ipv6_addr;
        elements = {
            2606:4700:4700::1111, 2606:4700:4700::1001,
            2001:4860:4860::8888, 2001:4860:4860::8844,
            2620:fe::fe, 2620:fe::9
        }
    }
}
```

Port assignment for per-UID DNS proxy: base port 5354, each managed UID gets `5354 + index` (index = insertion order into managed user list, stable for the session).

Agent creates the table and chains on startup (idempotent `nft -f`), tears down the entire `inet screenguard` table on clean exit. Systemd `ExecStopPost` should also run a cleanup command in case of crash.

---

## DNS proxy

- Implemented as a tokio task inside the agent process — no separate binary.
- Each managed UID with web filtering enabled gets one proxy instance (its own UDP + TCP listener on its assigned port).
- Domain matching: strip trailing dot, check if queried name equals blocked domain OR ends with `.{blocked_domain}` (covers all subdomains).
- Upstream resolver: read from `/etc/resolv.conf` at proxy startup, skip loopback addresses to avoid forwarding loops with systemd-resolved.
- TCP DNS note: DNS falls back to TCP for responses over 512 bytes (EDNS0 can raise this). Both transports must be handled.
- NXDOMAIN response: a well-formed DNS response packet with RCODE=3, no answers.

---

## Common crate changes (`crates/common`)

- Add `blocked_domains: Vec<String>` to `UserConfig`.
- Add `capabilities: Vec<String>` to `AgentHello` message payload.

---

## Server changes (`crates/server`)

### Database

New table:
```sql
CREATE TABLE blocked_domains (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id  UUID NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    domain      TEXT NOT NULL,
    enabled     BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (profile_id, domain)
);
```

Seed default domains when a new profile is created (all `enabled = false`).

Agent capability storage — extend `agents` table:
```sql
ALTER TABLE agents ADD COLUMN web_filter_available BOOLEAN;
```

### API

New endpoints:
- `GET  /api/v1/profiles/:id/blocked-domains` — list all entries with enabled flag
- `PUT  /api/v1/profiles/:id/blocked-domains` — full replace (same pattern as schedules)
- `PATCH /api/v1/profiles/:id/blocked-domains/:domain_id` — toggle enabled / update domain

`config_push` payload already flows through `UserConfig`; adding `blocked_domains` field (enabled entries only) is sufficient.

`GET /agents/:id` response — add `web_filter_available: bool | null` field.

Bump `config_versions` for a profile whenever its blocked domain list changes.

---

## Agent changes (`crates/agent`)

### SQLite schema additions

```sql
CREATE TABLE agent_capabilities (
    capability  TEXT PRIMARY KEY,
    available   INTEGER NOT NULL DEFAULT 0,
    checked_at  INTEGER NOT NULL
);

CREATE TABLE cached_blocked_domains (
    local_uid   INTEGER NOT NULL,
    domain      TEXT NOT NULL,
    PRIMARY KEY (local_uid, domain)
);
```

### New modules

- `nftables.rs` — manages all nft CLI invocations: create/delete table, add/remove per-UID chains and rules, manage DoH sets.
- `dns_proxy.rs` — per-UID DNS proxy task: UDP + TCP listeners, domain matching, upstream forwarding, NXDOMAIN generation.
- `web_filter.rs` — orchestrates the above: reads capability flag, manages proxy task lifecycle, reacts to config changes.

### Startup flow additions

1. Check `nft` binary presence → write to `agent_capabilities`.
2. If available: create nftables table (idempotent).
3. Report capability in `agent_hello` (`"capabilities": ["web_filter"]` or `[]`).
4. On `config_push`: if web filter available, update `cached_blocked_domains`, apply/refresh proxy and nftables rules per UID.

### Shutdown / crash cleanup

- `ExecStopPost=/usr/bin/nft delete table inet screenguard` in systemd unit.
- Agent also attempts cleanup on clean shutdown signal.

---

## Web UI changes (`webui/`)

- Agent list page: show web filter availability indicator per agent.
- Agent detail page: web filter capability line.
- Profile detail page: new **Blocked Sites** tab.
  - Toggle rows for each domain (name + domain + enabled switch).
  - "Add domain" input field.
  - Delete button per custom entry (default entries can be disabled but not deleted, or allowed to be deleted — TBD).

---

## Mobile app changes (`mobile/`)

- Agent detail screen: add capability badge (same info as web UI).
- Profile detail screen: new Blocked Sites section with toggle list and add/remove.

---

## Default blocked domains (seeded on profile creation)

```
youtube.com, tiktok.com, instagram.com, facebook.com,
twitter.com, x.com, twitch.tv, discord.com,
reddit.com, snapchat.com, roblox.com
```

All seeded as `enabled = false`.

---

## TODO checklist

Items are ordered roughly by dependency. Sign off each item as it is completed.

**Testing policy:** every item that produces runnable code must be tested before being signed off. Unit tests for pure logic (domain matching, NXDOMAIN generation, config parsing). Integration tests where a real component is involved (DB queries, nftables CLI invocations, DNS proxy round-trips). Manual verification for anything that touches the live system (actual session blocking, web UI flows, mobile flows). Do not mark an item complete based on "it compiles" — run it.

### Stage 1 — Common crate & server foundation ✅

- [x] Add `blocked_domains: Vec<String>` to `UserConfig` in `crates/common/src/models.rs`
- [x] Add `capabilities: Vec<String>` to `AgentHello` in `crates/common/src/messages.rs`
- [x] Add `blocked_domains` table + migration v6 to server (`crates/server/src/db.rs`)
- [x] Add `web_filter_available` column to `agents` table (schema + migration v6)
- [x] Seed default blocked domains on profile creation (all disabled)
- [x] Implement `GET /profiles/:id/blocked-domains`
- [x] Implement `PUT /profiles/:id/blocked-domains` (full replace)
- [x] Implement `POST /profiles/:id/blocked-domains` (add single domain)
- [x] Implement `PATCH /profiles/:id/blocked-domains/:domain_id` (toggle enabled)
- [x] Implement `DELETE /profiles/:id/blocked-domains/:domain_id`
- [x] Extend `config_push` to include enabled blocked domains per user
- [x] Extend `GET /agents/:id` and `GET /agents` responses with `web_filter_available`
- [x] Store `web_filter_available` from `agent_hello` capabilities in DB
- [x] Bump config version when blocked domain list changes
- [x] Tests: 6 new DB tests (seed, set/get, enabled-only, patch, delete, config_push propagation)

### Stage 2 — Agent: capability check & SQLite ✅

- [x] Add `agent_capabilities` and `cached_blocked_domains` tables to agent SQLite schema + migration (`crates/agent/src/db.rs`)
- [x] On startup: detect `nft` binary via `Command::new("nft").arg("--version")`, write result to `agent_capabilities` (`crates/agent/src/main.rs`)
- [x] Include `capabilities` in `agent_hello` based on stored capability (`crates/agent/src/heartbeat.rs`)
- [x] On `config_push`: parse and cache `blocked_domains` per UID into `cached_blocked_domains` (`apply_config_push` in `crates/agent/src/db.rs`)
- [x] DB methods: `save_capability`, `get_capability`, `save_cached_blocked_domains`, `get_cached_blocked_domains`
- [x] Tests: capability save/get, blocked domain caching via config_push, domain replacement, direct save/get

### Stage 3 — Agent: nftables module ✅

- [x] Implement `nftables.rs`: create/delete `inet screenguard` table via `nft` CLI
- [x] Implement per-UID DNS redirect rules (UDP + TCP port 53 DNAT)
- [x] Implement DoH block rules with static IPv4 + IPv6 sets
- [x] Implement add/remove UID rules when users are added/removed (full re-setup via `setup()`)
- [x] Add `ExecStopPost` nft cleanup to agent systemd unit

### Stage 4 — Agent: DNS proxy module ✅

- [x] Implement `dns_proxy.rs`: tokio UDP listener per UID on assigned port
- [x] Implement TCP DNS support in proxy
- [x] Implement domain blocklist matching (exact + subdomain suffix)
- [x] Implement NXDOMAIN response generation
- [x] Implement upstream forwarding (read `/etc/resolv.conf`, skip loopback)
- [x] Implement proxy task lifecycle (start/stop per UID on config change)

### Stage 5 — Agent: orchestration ✅

- [x] Implement `web_filter.rs`: wire up nftables + dns_proxy on config push
- [x] Handle config updates: update blocklist in running proxy without restart
- [x] Handle UID removed: tear down proxy task and nftables rules for that UID
- [x] Verify clean shutdown and crash cleanup via systemd ExecStopPost

### Stage 6 — Install script ✅

- [x] Check for `nft` binary after installing agent binary
- [x] Print warning and install hint if missing (`apt install nftables` / `dnf install nftables`)
- [x] Do not fail installation — just warn

### Stage 7 — Web UI ✅

- [x] Agent list: add web filter availability indicator (badge in version column)
- [x] Agent detail: add web filter capability line in info card
- [x] Profile detail: add Blocked Sites card with toggle list
- [x] Profile detail: add custom domain input
- [x] Profile detail: delete/remove domain

### Stage 8 — Mobile app ✅

- [x] Agent detail: add web filter capability badge
- [x] Profile detail: add Blocked Sites section with toggle list
- [x] Profile detail: add custom domain input and delete

### Stage 9 — Testing & validation

- [ ] Verify DNS intercept blocks queried domain for managed UID
- [ ] Verify subdomain matching works (`www.youtube.com` blocked when `youtube.com` in list)
- [ ] Verify unmanaged UIDs and root are not affected
- [ ] Verify DoH is blocked for managed UID (test against 1.1.1.1:443)
- [ ] Verify config push updates running proxy without restarting agent
- [ ] Verify offline enforcement uses cached blocklist
- [ ] Verify nftables table is cleaned up on agent stop and crash
- [ ] Verify feature is fully disabled and silent when `nft` is absent
