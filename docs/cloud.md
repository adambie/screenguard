# Cloud mode

> The hosted service is young — its endpoint and this enrolment flow may still change. Self-hosting on your own network is unaffected.

There is a hosted ScreenGuard server at **[screenguard.cc](https://screenguard.cc)** — a multi-account instance, so you can use ScreenGuard without running a server of your own. Each account is isolated: its own profiles, computers and usage history. The [mobile app](https://github.com/adambie/screenguard-mobile) signs in to the same account, so you can change a limit or lock a computer while you are out — no VPN back home, no port forwarding.

The agent is the same binary as for self-hosting and sends and accepts exactly the same things ([what the agent does](how-it-works.md#what-the-agent-does-on-your-machine)); only the address it connects to changes (`wss://api.screenguard.cc/ws`).

## At install time

Create an account at screenguard.cc, then on the child's computer:

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh \
  | sudo bash -s -- --cloud-account=you@example.com
```

The interactive installer offers the same: choose **Agent only**, then **Cloud account** when it asks how to reach the server. Either way it writes `/etc/screenguard/agent.toml` with `cloud_account` set (an existing `agent.toml` is not modified — add the line by hand, see below). The computer shows up as *pending* under your account; approve it there and it is managed.

## On an already-installed agent

```toml
# /etc/screenguard/agent.toml
cloud_account = "you@example.com"
# cloud_url  = "wss://api.screenguard.cc/ws"   # override the endpoint if needed
```

```bash
sudo systemctl restart screenguard-agent
```

You can also pass it once on the command line (`screenguard-agent --cloud-account you@example.com`), but the systemd unit carries no flags, so without the config line the agent falls back to its saved binding on the next restart — `agent.toml` is the supported way.

## Good to know

- The account is the **email of your cloud account**, not a server address. Use `--server-url wss://host/ws` (or `cloud_url`) for a different or self-run cloud endpoint.
- With a cloud account set, mDNS discovery and `server_url` are ignored.
- **Starting with a _different_ cloud account re-pairs from scratch.** If the agent was paired (to a LAN server or another account) and you start it with a different `--cloud-account`, it clears the old pairing *and* all downloaded rules and usage counters, then pairs anew. **The computer is unmanaged until the new account approves it**, and today's screen-time counter starts at zero. Restarting with the *same* account, or with none, does **not** wipe anything. To leave cloud mode or move to a new server, run `screenguard-agent --reset`.
- An unknown or mistyped account gives no error — the agent just stays *pending*. This is intentional (it stops the service being used to test which emails have accounts).
