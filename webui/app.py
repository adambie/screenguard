"""
Parental Controller – simple management web UI.

Usage:
    cd webui
    pip install -r requirements.txt
    SERVER_URL=http://localhost:8080 python app.py
"""

import json
import math
import os
import requests
from datetime import date, timedelta, datetime, timezone
from functools import wraps
from flask import (Flask, render_template, request, redirect, url_for,
                   session, flash, g, send_from_directory, jsonify)

app = Flask(__name__)
app.secret_key = os.environ.get("SECRET_KEY", "dev-secret-change-me")
app.permanent_session_lifetime = timedelta(days=30)

# ── i18n ───────────────────────────────────────────────────────────────────────

SUPPORTED_LANGS = ['en', 'es', 'fr', 'de', 'pt', 'pl']
_TRANSLATIONS: dict = {}

def _load_translations():
    trans_dir = os.path.join(os.path.dirname(__file__), 'translations')
    for lang in SUPPORTED_LANGS:
        path = os.path.join(trans_dir, f'{lang}.json')
        try:
            with open(path, encoding='utf-8') as f:
                _TRANSLATIONS[lang] = json.load(f)
        except Exception as e:
            print(f"Warning: could not load translation {lang}: {e}")
    if 'en' not in _TRANSLATIONS:
        _TRANSLATIONS['en'] = {}

_load_translations()

def _lang():
    return session.get('lang', 'en') if session else 'en'

def t(key, **kwargs):
    """Look up a translation key in the current language, with English fallback."""
    lang = _lang()
    trans = _TRANSLATIONS.get(lang, _TRANSLATIONS.get('en', {}))
    text = trans.get(key) or _TRANSLATIONS.get('en', {}).get(key, key)
    if kwargs:
        try:
            text = text.format(**kwargs)
        except Exception:
            pass
    return text

def days():
    """Return localized short day names Mon…Sun."""
    lang = _lang()
    trans = _TRANSLATIONS.get(lang, _TRANSLATIONS.get('en', {}))
    return trans.get('days', _TRANSLATIONS.get('en', {}).get('days',
           ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']))

app.jinja_env.globals['t'] = t
app.jinja_env.globals['days'] = days
app.jinja_env.globals['SUPPORTED_LANGS'] = SUPPORTED_LANGS

# ── template filters ───────────────────────────────────────────────────────────

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
            return t('agents.now')
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

COMMON_TIMEZONES = [
    ("Africa", ["Africa/Cairo", "Africa/Johannesburg", "Africa/Lagos", "Africa/Nairobi"]),
    ("America", ["America/Anchorage", "America/Argentina/Buenos_Aires", "America/Bogota",
                 "America/Chicago", "America/Denver", "America/Los_Angeles", "America/Mexico_City",
                 "America/New_York", "America/Phoenix", "America/Sao_Paulo", "America/Toronto",
                 "America/Vancouver"]),
    ("Asia", ["Asia/Bangkok", "Asia/Colombo", "Asia/Dubai", "Asia/Hong_Kong", "Asia/Jakarta",
              "Asia/Karachi", "Asia/Kolkata", "Asia/Kuala_Lumpur", "Asia/Seoul",
              "Asia/Shanghai", "Asia/Singapore", "Asia/Taipei", "Asia/Tokyo"]),
    ("Atlantic", ["Atlantic/Reykjavik"]),
    ("Australia", ["Australia/Adelaide", "Australia/Brisbane", "Australia/Melbourne",
                   "Australia/Perth", "Australia/Sydney"]),
    ("Europe", ["Europe/Amsterdam", "Europe/Athens", "Europe/Belgrade", "Europe/Berlin",
                "Europe/Brussels", "Europe/Bucharest", "Europe/Budapest", "Europe/Copenhagen",
                "Europe/Dublin", "Europe/Helsinki", "Europe/Istanbul", "Europe/Kiev",
                "Europe/Lisbon", "Europe/London", "Europe/Madrid", "Europe/Moscow",
                "Europe/Oslo", "Europe/Paris", "Europe/Prague", "Europe/Rome",
                "Europe/Sofia", "Europe/Stockholm", "Europe/Vienna", "Europe/Warsaw",
                "Europe/Zurich"]),
    ("Pacific", ["Pacific/Auckland", "Pacific/Fiji", "Pacific/Honolulu"]),
    ("UTC", ["UTC"]),
]


# ── helpers ───────────────────────────────────────────────────────────────────

def api(method, path, **kwargs):
    token = session.get("token")
    headers = kwargs.pop("headers", {})
    if token:
        headers["Authorization"] = f"Bearer {token}"
    try:
        r = requests.request(method, f"{API}{path}", headers=headers,
                             timeout=5, **kwargs)
        if r.status_code == 401:
            g.session_expired = True
        return r
    except requests.ConnectionError:
        return None


def require_login(f):
    @wraps(f)
    def wrapper(*args, **kwargs):
        if "token" not in session:
            return redirect(url_for("login"))
        g.session_expired = False
        result = f(*args, **kwargs)
        if g.session_expired:
            session.clear()
            flash(t("flash.session_expired"), "warning")
            return redirect(url_for("login"))
        return result
    return wrapper


# ── auth ──────────────────────────────────────────────────────────────────────

@app.route("/sw.js")
def service_worker():
    return send_from_directory(app.static_folder, "sw.js",
                               mimetype="application/javascript")


@app.route("/", methods=["GET"])
def index():
    if "token" in session:
        return redirect(url_for("dashboard"))
    return redirect(url_for("login"))


@app.route("/set-language", methods=["POST"])
def set_language():
    lang = request.form.get("lang", "en")
    if lang in SUPPORTED_LANGS:
        session["lang"] = lang
    next_url = request.form.get("next") or url_for("dashboard")
    return redirect(next_url)


@app.route("/login", methods=["GET", "POST"])
def login():
    # Check if setup is needed.
    setup_needed = False
    r = api("GET", "/auth/status")
    if r is None:
        flash(t("flash.server_unreachable", server=SERVER), "danger")
    elif r.ok:
        setup_needed = r.json().get("setup_needed", False)

    if request.method == "POST":
        action = request.form.get("action")
        username = request.form["username"]
        password = request.form["password"]

        if action == "setup":
            r = api("POST", "/auth/setup", json={"username": username, "password": password})
            if r and r.status_code == 201:
                flash(t("flash.admin_created"), "success")
                return redirect(url_for("login"))
            else:
                flash(t("flash.setup_failed", error=r.json().get('error') if r else 'no response'), "danger")
        else:
            r = api("POST", "/auth/login", json={"username": username, "password": password})
            if r and r.status_code == 200:
                data = r.json()
                if request.form.get("keep_signed_in"):
                    session.permanent = True
                session["token"] = data["token"]
                session["username"] = username
                return redirect(url_for("dashboard"))
            else:
                flash(t("flash.invalid_credentials"), "danger")

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
        flash(t("flash.agent_accepted"), "success")
    else:
        flash(t("flash.adjustment_failed", error=r.json().get('error') if r else 'no response'), "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/delete", methods=["POST"])
@require_login
def delete_agent(agent_id):
    r = api("DELETE", f"/agents/{agent_id}")
    if r and r.ok:
        flash(t("flash.agent_deleted"), "success")
    else:
        flash(t("flash.lock_failed"), "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/undo-delete", methods=["POST"])
@require_login
def undo_delete_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/undo-delete")
    flash(t("flash.deletion_cancelled") if (r and r.ok) else t("flash.cancel_failed"),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/force-delete", methods=["POST"])
@require_login
def force_delete_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/force-delete")
    flash(t("flash.agent_removed") if (r and r.ok) else t("flash.force_delete_failed"),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("agents"))


@app.route("/agents/<agent_id>/logs")
@require_login
def agent_logs(agent_id):
    r = api("GET", f"/agents/{agent_id}/logs")
    if r and r.ok:
        return jsonify(r.json())
    error = r.json().get("error", "Unknown error") if r else "Request failed"
    return jsonify({"error": error}), (r.status_code if r else 500)


@app.route("/agents/<agent_id>/update", methods=["POST"])
@require_login
def update_agent(agent_id):
    r = api("POST", f"/agents/{agent_id}/update")
    if r and r.ok:
        flash(t("flash.agent_update_triggered"), "success")
    else:
        error = r.json().get("error", "") if r else ""
        flash(t("flash.agent_update_failed") + (f": {error}" if error else ""), "danger")
    return redirect(url_for("agent_detail", agent_id=agent_id))


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
        flash(t("flash.user_linked") if profile_id else t("flash.user_unlinked"), "success")
    else:
        flash(t("flash.user_link_failed"), "danger")
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
        flash(t("flash.name_required"), "warning")
        return redirect(url_for("dashboard"))
    r = api("POST", "/profiles", json={"display_name": name})
    if r and r.ok:
        flash(t("flash.profile_created", name=name), "success")
    else:
        flash(t("flash.profile_create_failed"), "danger")
    return redirect(url_for("dashboard"))


@app.route("/profiles/<profile_id>")
@require_login
def profile_detail(profile_id):
    r = api("GET", f"/profiles/{profile_id}")
    if not r or not r.ok:
        flash(t("flash.profile_not_found"), "danger")
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
    week_start = today_date - timedelta(days=today_date.weekday()) + timedelta(weeks=week_offset)
    week_end = week_start + timedelta(days=6)

    r3 = api("GET", f"/profiles/{profile_id}/usage",
             params={"from": str(week_start), "to": str(week_end)})
    usage = r3.json().get("usage", []) if r3 and r3.ok else []

    usage_by_date = {u['date']: u for u in usage}
    max_used = max((u.get('used_minutes') or 0 for u in usage), default=0)
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
    flash(t("flash.profile_deleted") if (r and r.ok) else t("flash.profile_delete_failed"),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("dashboard"))


@app.route("/profiles/<profile_id>/language", methods=["POST"])
@require_login
def set_profile_language(profile_id):
    lang = request.form.get("language", "en")
    if lang not in SUPPORTED_LANGS:
        lang = "en"
    r = api("PATCH", f"/profiles/{profile_id}", json={"language": lang})
    flash(t("flash.language_saved") if (r and r.ok) else t("flash.lock_failed"),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/schedules", methods=["POST"])
@require_login
def update_schedules(profile_id):
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
    flash(t("flash.schedules_saved", version=r.json().get('config_version', '?')) if (r and r.ok)
          else t("flash.schedules_failed"), "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/limits", methods=["POST"])
@require_login
def update_limits(profile_id):
    limits = []
    for dow in range(7):
        val = request.form.get(f"limit_{dow}", "").strip()
        if val:
            try:
                limits.append({"day_of_week": dow, "allowed_minutes": int(val)})
            except ValueError:
                pass
    r = api("PUT", f"/profiles/{profile_id}/daily-limits", json={"limits": limits})
    flash(t("flash.limits_saved", version=r.json().get('config_version', '?')) if (r and r.ok)
          else t("flash.limits_failed"), "success" if (r and r.ok) else "danger")
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
        flash(t("flash.invalid_minutes"), "warning")
        return redirect(url_for("profile_detail", profile_id=profile_id))
    r = api("POST", f"/profiles/{profile_id}/adjustments",
            json={"target_date": target, "adjustment_minutes": minutes, "reason": reason})
    flash(t("flash.adjustment_added") if (r and r.ok)
          else t("flash.adjustment_failed", error=r.json() if r else 'no response'),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/notify", methods=["POST"])
@require_login
def notify_profile(profile_id):
    message = request.form.get("message", "").strip()
    if not message:
        flash(t("flash.message_empty"), "warning")
        return redirect(url_for("profile_detail", profile_id=profile_id))
    r = api("POST", f"/profiles/{profile_id}/notify", json={"body": message})
    if r and r.ok:
        flash(t("flash.message_sent"), "success")
    else:
        flash(t("flash.message_failed"), "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


@app.route("/profiles/<profile_id>/lock-now", methods=["POST"])
@require_login
def lock_now(profile_id):
    r = api("POST", f"/profiles/{profile_id}/lock-now")
    flash(t("flash.locked") if (r and r.ok) else t("flash.lock_failed"),
          "success" if (r and r.ok) else "danger")
    return redirect(url_for("profile_detail", profile_id=profile_id))


# ── settings ──────────────────────────────────────────────────────────────────

@app.route("/settings", methods=["GET", "POST"])
@require_login
def settings():
    if request.method == "POST":
        timezone = request.form.get("timezone", "").strip()
        if timezone:
            r = api("PATCH", "/auth/me", json={"timezone": timezone})
            if r and r.ok:
                flash(t("flash.settings_saved"), "success")
            else:
                flash(t("flash.settings_error"), "danger")
        return redirect(url_for("settings"))
    r = api("GET", "/auth/me")
    me = r.json() if r and r.ok else {}
    return render_template("settings.html", me=me, timezones=COMMON_TIMEZONES)


# ── run ───────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    port = int(os.environ.get("UI_PORT", 5000))
    app.run(host="0.0.0.0", port=port, debug=True)
