"""
Parental Controller – simple management web UI.

Usage:
    cd webui
    pip install -r requirements.txt
    SERVER_URL=http://localhost:8080 python app.py
"""

import math
import os
import requests
from datetime import date, timedelta, datetime, timezone
from functools import wraps
from flask import (Flask, render_template, request, redirect, url_for,
                   session, flash, g)

app = Flask(__name__)
app.secret_key = os.environ.get("SECRET_KEY", "dev-secret-change-me")


@app.template_filter('ts_date')
def ts_date(ts):
    """Format a Unix timestamp as YYYY-MM-DD."""
    try:
        return datetime.fromtimestamp(int(ts), tz=timezone.utc).strftime('%Y-%m-%d')
    except Exception:
        return '—'


@app.template_filter('time_ago')
def time_ago(ts):
    """Format a Unix timestamp as 'just now', '5m ago', '2h ago', or 'May 3'."""
    if ts is None:
        return '—'
    try:
        dt = datetime.fromtimestamp(int(ts), tz=timezone.utc)
        diff = int((datetime.now(tz=timezone.utc) - dt).total_seconds())
        if diff < 60:
            return 'just now'
        if diff < 3600:
            return f'{diff // 60}m ago'
        if diff < 86400:
            return f'{diff // 3600}h ago'
        return dt.strftime('%b %-d')
    except Exception:
        return str(ts)


@app.template_filter('fmt_mins')
def fmt_mins(m):
    """Format minutes as '1h 30m', '45m', '0m', or '—' for None."""
    if m is None:
        return '—'
    try:
        m = int(m)
    except (ValueError, TypeError):
        return str(m)
    if m <= 0:
        return '0m'
    h, rem = divmod(m, 60)
    if h and rem:
        return f'{h}h {rem}m'
    if h:
        return f'{h}h'
    return f'{rem}m'

SERVER = os.environ.get("SERVER_URL", "http://localhost:8080").rstrip("/")
API = f"{SERVER}/api/v1"


# ── helpers ───────────────────────────────────────────────────────────────────

def api(method, path, **kwargs):
    token = session.get("token")
    headers = kwargs.pop("headers", {})
    if token:
        headers["Authorization"] = f"Bearer {token}"
    try:
        r = requests.request(method, f"{API}{path}", headers=headers,
                             timeout=5, **kwargs)
        return r
    except requests.ConnectionError:
        return None


def require_login(f):
    @wraps(f)
    def wrapper(*args, **kwargs):
        if "token" not in session:
            return redirect(url_for("login"))
        return f(*args, **kwargs)
    return wrapper


def days():
    return ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]


app.jinja_env.globals["days"] = days


# ── auth ──────────────────────────────────────────────────────────────────────

@app.route("/", methods=["GET"])
def index():
    if "token" in session:
        return redirect(url_for("dashboard"))
    return redirect(url_for("login"))


@app.route("/login", methods=["GET", "POST"])
def login():
    # Check if setup is needed.
    setup_needed = False
    r = api("GET", "/auth/status")
    if r is None:
        flash("Cannot reach server at " + SERVER, "danger")
    elif r.ok:
        setup_needed = r.json().get("setup_needed", False)

    if request.method == "POST":
        action = request.form.get("action")
        username = request.form["username"]
        password = request.form["password"]

        if action == "setup":
            r = api("POST", "/auth/setup", json={"username": username, "password": password})
            if r and r.status_code == 201:
                flash("Admin account created — please log in.", "success")
                return redirect(url_for("login"))
            else:
                flash(f"Setup failed: {r.json().get('error') if r else 'no response'}", "danger")
        else:
            r = api("POST", "/auth/login", json={"username": username, "password": password})
            if r and r.status_code == 200:
                data = r.json()
                session["token"] = data["token"]
                session["username"] = username
                return redirect(url_for("dashboard"))
            else:
                flash("Invalid credentials.", "danger")

    return render_template("login.html", setup_needed=setup_needed)


@app.route("/logout")
def logout():
    session.clear()
    return redirect(url_for("login"))


# ── dashboard ─────────────────────────────────────────────────────────────────

@app.route("/dashboard")
@require_login
def dashboard():
    r = api("GET", "/dashboard")
    data = r.json() if r and r.ok else {"profiles": [], "pending_agents": 0}
    return render_template("dashboard.html", data=data)


# ── agents ────────────────────────────────────────────────────────────────────

@app.route("/agents")
@require_login
def agents():
    r = api("GET", "/agents")
    agents_list = r.json().get("agents", []) if r and r.ok else []
    return render_template("agents.html", agents=agents_list)


@app.route("/agents/<agent_id>")
@require_login
def agent_detail(agent_id):
    r = api("GET", f"/agents/{agent_id}")
    agent = r.json() if r and r.ok else {}
    r2 = api("GET", f"/agents/{agent_id}/users")
    users = r2.json().get("users", []) if r2 and r2.ok else []
    r3 = api("GET", "/profiles")
    profiles = r3.json().get("profiles", []) if r3 and r3.ok else []
    return render_template("agent.html", agent=agent, users=users, profiles=profiles)


@app.route("/agents/<agent_id>/accept", methods=["POST"])
@require_login
def accept_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/accept")
    if r and r.ok:
        flash("Agent accepted — it will reconnect shortly.", "success")
    else:
        flash(f"Failed: {r.json().get('error') if r else 'no response'}", "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/delete", methods=["POST"])
@require_login
def delete_agent(agent_id):
    r = api("DELETE", f"/agents/{agent_id}")
    if r and r.ok:
        flash("Agent queued for deletion — it will be unlinked when it next connects.", "success")
    else:
        flash("Delete failed.", "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/undo-delete", methods=["POST"])
@require_login
def undo_delete_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/undo-delete")
    flash("Deletion cancelled." if (r and r.ok) else "Failed to cancel deletion.",
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/force-delete", methods=["POST"])
@require_login
def force_delete_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/force-delete")
    flash("Agent removed." if (r and r.ok) else "Force delete failed.",
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/rename", methods=["POST"])
@require_login
def rename_agent(agent_id):
    name = request.form.get("display_name", "").strip()
    if name:
        api("PATCH", f"/agents/{agent_id}", json={"display_name": name})
    return redirect(url_for("agent_detail", agent_id=agent_id))


@app.route("/agent-users/<user_id>/link", methods=["POST"])
@require_login
def link_agent_user(user_id):
    profile_id = request.form.get("profile_id") or None
    status = "managed" if profile_id else "unmanaged"
    r = api("PATCH", f"/agent-users/{user_id}",
            json={"profile_id": profile_id, "status": status})
    agent_id = request.form.get("agent_id")
    if r and r.ok:
        flash("User linked to profile." if profile_id else "User unlinked.", "success")
    else:
        flash("Failed to update user.", "danger")
    return redirect(url_for("agent_detail", agent_id=agent_id))


# ── profiles ──────────────────────────────────────────────────────────────────

@app.route("/profiles")
@require_login
def profiles():
    return redirect(url_for("dashboard"))


@app.route("/profiles/create", methods=["POST"])
@require_login
def create_profile():
    name = request.form.get("display_name", "").strip()
    if not name:
        flash("Name is required.", "warning")
        return redirect(url_for("dashboard"))
    r = api("POST", "/profiles", json={"display_name": name})
    if r and r.ok:
        flash(f"Profile '{name}' created.", "success")
    else:
        flash("Failed to create profile.", "danger")
    return redirect(url_for("dashboard"))


@app.route("/profiles/<profile_id>")
@require_login
def profile_detail(profile_id):
    r = api("GET", f"/profiles/{profile_id}")
    if not r or not r.ok:
        flash("Profile not found.", "danger")
        return redirect(url_for("dashboard"))
    data = r.json()

    r2 = api("GET", f"/profiles/{profile_id}/status")
    status = r2.json().get("profile") if r2 and r2.ok else {}

    r_agents = api("GET", "/agents")
    agents_by_id = {a["id"]: a for a in (r_agents.json().get("agents", []) if r_agents and r_agents.ok else [])}
    for u in data.get("agent_users", []):
        u["agent_hostname"] = agents_by_id.get(str(u.get("agent_id", "")), {}).get("hostname", "?")

    today_date = date.today()
    week_offset = int(request.args.get('week', 0))
    # Monday of the selected week
    week_start = today_date - timedelta(days=today_date.weekday()) + timedelta(weeks=week_offset)
    week_end = week_start + timedelta(days=6)

    r3 = api("GET", f"/profiles/{profile_id}/usage",
             params={"from": str(week_start), "to": str(week_end)})
    usage = r3.json().get("usage", []) if r3 and r3.ok else []

    usage_by_date = {u['date']: u for u in usage}
    max_used = max((u.get('used_minutes') or 0 for u in usage), default=0)
    # Round up to the next 15-minute interval, minimum 15m, so bars never touch the top
    chart_max = max(math.ceil(max_used / 15) * 15, 15)

    week_bars = []
    for i in range(7):
        day = week_start + timedelta(days=i)
        u = usage_by_date.get(str(day), {})
        used = u.get('used_minutes') or 0
        limit = u.get('limit_minutes')
        adj = u.get('adjustments_minutes') or 0
        eff_limit = (limit + adj) if limit is not None else None
        pct = min(int(used / chart_max * 100), 100)
        over = eff_limit is not None and used > eff_limit
        warn = eff_limit is not None and eff_limit > 0 and used / eff_limit >= 0.75
        week_bars.append({
            'date': str(day),
            'day_name': day.strftime('%a'),
            'is_today': day == today_date,
            'is_future': day > today_date,
            'used': used,
            'limit': limit,
            'eff_limit': eff_limit,
            'pct': pct,
            'over': over,
            'warn': warn,
        })

    return render_template("profile.html",
                           profile=data["profile"],
                           schedules=data.get("schedules", []),
                           limits=data.get("daily_limits", []),
                           agent_users=data.get("agent_users", []),
                           status=status,
                           week_bars=week_bars,
                           week_max=chart_max,
                           week_offset=week_offset,
                           week_start=week_start,
                           week_end=week_end,
                           today=str(today_date))


@app.route("/profiles/<profile_id>/rename", methods=["POST"])
@require_login
def rename_profile(profile_id):
    name = request.form.get("display_name", "").strip()
    if name:
        api("PATCH", f"/profiles/{profile_id}", json={"display_name": name})
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/delete", methods=["POST"])
@require_login
def delete_profile(profile_id):
    r = api("DELETE", f"/profiles/{profile_id}")
    flash("Profile deleted." if (r and r.ok) else "Delete failed.", "success" if (r and r.ok) else "danger")
    return redirect(url_for("dashboard"))


@app.route("/profiles/<profile_id>/schedules", methods=["POST"])
@require_login
def update_schedules(profile_id):
    """Rebuild the schedules list from posted form data."""
    entries = []
    i = 0
    while True:
        dow = request.form.get(f"dow_{i}")
        start = request.form.get(f"start_{i}")
        end = request.form.get(f"end_{i}")
        if dow is None:
            break
        if start and end:
            entries.append({"day_of_week": int(dow), "start_time": start, "end_time": end})
        i += 1
    r = api("PUT", f"/profiles/{profile_id}/schedules", json={"schedules": entries})
    flash(f"Schedules saved (config v{r.json().get('config_version','?')})." if (r and r.ok)
          else "Failed to save schedules.", "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/limits", methods=["POST"])
@require_login
def update_limits(profile_id):
    """Rebuild the daily limits from posted form data."""
    limits = []
    for dow in range(7):
        val = request.form.get(f"limit_{dow}", "").strip()
        if val:
            try:
                limits.append({"day_of_week": dow, "allowed_minutes": int(val)})
            except ValueError:
                pass
    r = api("PUT", f"/profiles/{profile_id}/daily-limits", json={"limits": limits})
    flash(f"Limits saved (config v{r.json().get('config_version','?')})." if (r and r.ok)
          else "Failed to save limits.", "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/adjust", methods=["POST"])
@require_login
def add_adjustment(profile_id):
    target = request.form.get("target_date") or str(date.today())
    minutes = request.form.get("minutes", "0")
    reason = request.form.get("reason", "").strip() or None
    try:
        minutes = int(minutes)
    except ValueError:
        flash("Invalid minutes value.", "warning")
        return redirect(url_for("profile_detail", profile_id=profile_id))
    r = api("POST", f"/profiles/{profile_id}/adjustments",
            json={"target_date": target, "adjustment_minutes": minutes, "reason": reason})
    flash(f"Adjustment added." if (r and r.ok) else f"Failed: {r.json() if r else 'no response'}",
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/notify", methods=["POST"])
@require_login
def notify_profile(profile_id):
    message = request.form.get("message", "").strip()
    if not message:
        flash("Message cannot be empty.", "warning")
        return redirect(url_for("profile_detail", profile_id=profile_id))
    r = api("POST", f"/profiles/{profile_id}/notify",
            json={"body": message})
    if r and r.ok:
        data = r.json()
        flash(f"Message sent: {data.get('message', 'ok')}", "success")
    else:
        flash("Failed to send message.", "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/lock-now", methods=["POST"])
@require_login
def lock_now(profile_id):
    r = api("POST", f"/profiles/{profile_id}/lock-now")
    flash("Today's allowance zeroed out." if (r and r.ok) else "Failed.", "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


# ── run ───────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    port = int(os.environ.get("UI_PORT", 5000))
    app.run(host="0.0.0.0", port=port, debug=True)
