# Web Content Filtering — Feature Overview

## What it does

ScreenGuard agents can now block access to specific websites for managed users. Blocked domains return no result — the browser shows a standard "can't connect" error. Filtering works system-wide across all browsers and apps, not just a single browser.

## How it works

The agent intercepts DNS queries made by the managed user and returns a negative response for any domain on the blocklist. Blocking applies only to the managed user's sessions — the admin account and other unmanaged users are not affected. Blocking persists while the agent is running and survives server disconnects (the blocklist is cached locally).

## Requirements

Web filtering requires the `nft` command-line tool (`nftables` package) to be present on the managed machine. The installer will check for it and warn if it is missing. The feature can be enabled later by installing nftables and restarting the agent — no reinstall needed.

If `nft` is not present, all other ScreenGuard functionality continues to work normally. Web filtering is simply unavailable for that agent.

## Domain matching

Blocking `youtube.com` automatically covers all subdomains: `www.youtube.com`, `m.youtube.com`, `music.youtube.com`, and so on. You only need to enter the root domain.

## Managing the blocklist

The blocklist is configured per profile, the same way schedules and daily limits are. Changes take effect immediately on all online agents linked to that profile.

Each new profile starts with a default list of commonly blocked sites. Every entry can be individually enabled, disabled, or removed. Custom domains can be added at any time.

### Default blocked sites

| Site | Domain |
|------|--------|
| YouTube | youtube.com |
| TikTok | tiktok.com |
| Instagram | instagram.com |
| Facebook | facebook.com |
| Twitter / X | twitter.com, x.com |
| Twitch | twitch.tv |
| Discord | discord.com |
| Reddit | reddit.com |
| Snapchat | snapchat.com |
| Roblox | roblox.com |

All entries are **disabled by default** — enabling them is a deliberate admin action.

## Web UI

- Profile detail page: new **Blocked Sites** tab with toggle list and custom domain field.
- Agent list and agent detail page: shows whether web filtering is available or unavailable for each agent.

## Mobile app

- Agent detail: capability badge (filtering available / unavailable).
- Profile detail: Blocked Sites section, same functionality as the web UI.

## Known limitations

- Filtering works at the domain level only — individual URLs or paths cannot be blocked.
- A user running a VPN can bypass filtering. This is a known limitation of DNS-based approaches.
- The browser shows a generic connection error when a site is blocked, not a ScreenGuard-branded message.
- DNS over HTTPS (DoH) is blocked for managed users to prevent bypass via encrypted DNS. Known DoH provider IPs (Cloudflare, Google, Quad9, NextDNS) are blocked at the network level.
