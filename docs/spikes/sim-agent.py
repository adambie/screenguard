#!/usr/bin/env python3
"""Compatibility check for docs/feature-app-usage-limits.md (step 1.20).

A simulated agent that speaks the real WebSocket protocol against a running
server: pair with a pairing code (accepted through the admin API), reconnect
with the token, send hello + user list, then `app_usage_sync` followed by a
heartbeat. Against an old server this shows the unknown message is ignored
without a disconnect (the heartbeat still gets its `remaining_update`); against a
new server the app usage appears in the reports API.

Run against local test servers only (localhost, mDNS off). Needs `websockets`
and `requests`.

Usage: sim-agent.py http://127.0.0.1:PORT ADMIN_USER ADMIN_PASSWORD
"""
import asyncio
import json
import sys
from datetime import date, datetime, timezone

import requests
import websockets

BASE, USER, PASSWORD = sys.argv[1], sys.argv[2], sys.argv[3]
API = f"{BASE}/api/v1"
WS = BASE.replace("http", "ws", 1) + "/ws"
CODE = "ABC123"
MACHINE = "sim-agent-machine"


def msg(kind, payload):
    return json.dumps({"type": kind, "timestamp": datetime.now(timezone.utc).isoformat(), "payload": payload})


async def recv_type(ws, kind, timeout=10):
    async with asyncio.timeout(timeout):
        while True:
            m = json.loads(await ws.recv())
            if m["type"] == kind:
                return m["payload"]


def admin():
    s = requests.Session()
    tok = s.post(f"{API}/auth/login", json={"username": USER, "password": PASSWORD}).json()["token"]
    s.headers["authorization"] = f"Bearer {tok}"
    return s


async def main():
    s = admin()
    print("server version:", requests.get(f"{API}/version").json())

    # 1. pairing
    async with websockets.connect(WS) as ws:
        await ws.send(msg("pairing_request", {"machine_id": MACHINE, "hostname": "sim", "pairing_code": CODE}))
        for _ in range(50):
            agents = s.get(f"{API}/agents").json()["agents"]
            pending = [a for a in agents if a["machine_id"] == MACHINE and a.get("pairing_requested")]
            if pending:
                break
            await asyncio.sleep(0.1)
        agent_id = pending[0]["id"]
        r = s.post(f"{API}/agents/{agent_id}/accept", json={"pairing_code": CODE})
        assert r.ok, r.text
        token = (await recv_type(ws, "pairing_accepted"))["auth_token"]
    print("paired:", agent_id)

    # 2. normal connection
    headers = {"Authorization": f"Bearer {token}"}
    async with websockets.connect(WS, additional_headers=headers) as ws:
        await ws.send(msg("agent_hello", {
            "machine_id": MACHINE, "hostname": "sim", "timezone": "UTC", "agent_version": "0.10.18",
            "last_config_version": 0, "capabilities": ["app_usage"],
        }))
        await ws.send(msg("user_list_update", {
            "users": [{"local_uid": 1001, "username": "kid", "display_name": "Kid"}], "removed_uids": [],
        }))
        await asyncio.sleep(0.5)
        users = s.get(f"{API}/agents/{agent_id}/users").json()
        users = users.get("users", users) if isinstance(users, dict) else users
        au = next(u for u in users if u["local_uid"] == 1001)
        profile = s.post(f"{API}/profiles", json={"display_name": "Sim kid"}).json()
        profile_id = profile.get("id") or profile["profile"]["id"]
        assert s.patch(f"{API}/agent-users/{au['id']}", json={"profile_id": profile_id, "status": "managed"}).ok

        today = date.today().isoformat()
        entries = [
            {"local_uid": 1001, "date": today, "app_id": "desktop:firefox", "display_name": "Firefox",
             "icon": "firefox", "seconds_total": 600},
            {"local_uid": 1001, "date": today, "app_id": "steam:570", "display_name": "Dota 2", "seconds_total": 1200},
        ]
        await ws.send(msg("app_usage_sync", {"entries": entries}))
        entries[0]["seconds_total"] = 660  # grows
        await ws.send(msg("app_usage_sync", {"entries": entries}))
        await ws.send(msg("app_usage_sync", {"entries": entries}))  # resend: no double count
        await ws.send(msg("heartbeat", {"users": [
            {"local_uid": 1001, "active_seconds_since_last": 10, "idle": False, "session_count": 1}]}))
        remaining = await recv_type(ws, "remaining_update")
        print("still connected after app_usage_sync — remaining_update:", remaining["users"][:1])

    agent = s.get(f"{API}/agents/{agent_id}").json()
    print("agent app_usage_available:", agent.get("app_usage_available", "<not in this server's API>"))
    r = s.get(f"{API}/profiles/{profile_id}/app-usage")
    if r.status_code == 404:
        print("reports API: not on this server (old version) — OK")
    else:
        body = r.json()
        print("reports API:", [(a["app_id"], a["seconds"]) for a in body["apps"]], "total", body["total_seconds"])
        assert [(a["app_id"], a["seconds"]) for a in body["apps"]] == [("steam:570", 1200), ("desktop:firefox", 660)]
    print("OK")


asyncio.run(main())
