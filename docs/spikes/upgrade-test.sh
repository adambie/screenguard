#!/bin/bash
# Upgrade test (docs/feature-app-usage-limits.md, step 1.12): an old server
# release creates and populates the DB, then the new server migrates it.
# Seeds a paired agent, a managed user and screen-time usage, and checks they
# survive. Localhost only, mDNS off. Needs sqlite3, jq, curl; docker for postgres.
#
# Usage: upgrade-test.sh sqlite|postgres OLD_SERVER_BIN NEW_SERVER_BIN [WORKDIR]
# Old binary, e.g.: git worktree add /tmp/sg-old v0.10.17 &&
#   (cd /tmp/sg-old && cargo build --release -p server)
set -euo pipefail
MODE=$1; OLD=$2; NEW=$3; PORT=18087
W=${4:-$(mktemp -d)}/upgrade-$MODE; rm -rf "$W"; mkdir -p "$W"
cat > "$W/server.toml" <<TOML
listen_addr = "127.0.0.1"
listen_port = $PORT
enable_mdns = false
jwt_secret = "upgrade-test-secret-upgrade-test-secret"
db_path = "$W/server.db"
TOML
if [[ $MODE == postgres ]]; then
  docker rm -f sg-upgrade-pg >/dev/null 2>&1 || true
  docker run -d --name sg-upgrade-pg -e POSTGRES_PASSWORD=pw -e POSTGRES_DB=sg -p 127.0.0.1:15433:5432 postgres:16 >/dev/null
  for i in $(seq 30); do docker exec sg-upgrade-pg pg_isready -U postgres -d sg >/dev/null 2>&1 && break; sleep 1; done
  sleep 2
  export DATABASE_URL="postgres://postgres:pw@127.0.0.1:15433/sg"
  sql() { docker exec -i sg-upgrade-pg psql -qtAX -U postgres -d sg -c "$1"; }
else
  sql() { sqlite3 "$W/server.db" "$1"; }
fi
export SCREENGUARD_SERVER_CONFIG="$W/server.toml" RUST_LOG=info
start() { "$1" >"$W/$2.log" 2>&1 & PID=$!; for i in $(seq 50); do curl -fs "localhost:$PORT/api/v1/version" >/dev/null 2>&1 && return; sleep 0.2; done; echo "server $2 did not start"; cat "$W/$2.log"; exit 1; }
stop() { kill $PID; wait $PID 2>/dev/null || true; }

echo "== $MODE: old server (0.10.17)"
start "$OLD" old
curl -fs "localhost:$PORT/api/v1/version"; echo
curl -fs -X POST "localhost:$PORT/api/v1/auth/setup" -H 'content-type: application/json' -d '{"username":"admin","password":"secret-pass-123"}' >/dev/null
echo "schema before: $(sql 'SELECT version FROM _schema_version')"
NOW=$(date +%s)
sql "INSERT INTO agents (id,machine_id,display_name,hostname,timezone,status,auth_token_hash,token_verified,agent_version,paired_at,last_seen_at,created_at,web_filter_available) VALUES ('11111111-1111-1111-1111-111111111111','mach-1','Kid PC','kid-pc','UTC','paired','tokhash',1,'0.10.17',$NOW,$NOW,$NOW,1)"
sql "INSERT INTO user_profiles (id,display_name,language,created_at,updated_at) VALUES ('22222222-2222-2222-2222-222222222222','Kid','pl',$NOW,$NOW)"
sql "INSERT INTO agent_users (id,agent_id,profile_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at) VALUES ('33333333-3333-3333-3333-333333333333','11111111-1111-1111-1111-111111111111','22222222-2222-2222-2222-222222222222',1001,'kid','Kid','managed',$NOW,$NOW)"
sql "INSERT INTO daily_usage (agent_user_id,date,used_seconds,reported_at) VALUES ('33333333-3333-3333-3333-333333333333','2026-10-19',3600,$NOW)"
stop

echo "== $MODE: new server"
start "$NEW" new
grep -E "migration|ERROR|error" "$W/new.log" || true
echo "schema after: $(sql 'SELECT version FROM _schema_version')"
echo "agent: $(sql "SELECT status||' '||auth_token_hash||' '||token_verified||' '||COALESCE(CAST(app_usage_available AS TEXT),'null') FROM agents")"
echo "usage: $(sql "SELECT used_seconds FROM daily_usage")"
echo "app tables: $(sql "SELECT COUNT(*) FROM app_usage") $(sql "SELECT COUNT(*) FROM app_catalog")"
TOKEN=$(curl -fs -X POST "localhost:$PORT/api/v1/auth/login" -H 'content-type: application/json' -d '{"username":"admin","password":"secret-pass-123"}' | jq -r .token)
curl -fs "localhost:$PORT/api/v1/agents" -H "authorization: Bearer $TOKEN" | jq -c '.agents[] | {display_name,status,web_filter_available,app_usage_available}'
stop
echo "== $MODE: restart new server (idempotent)"
start "$NEW" new2; echo "schema: $(sql 'SELECT version FROM _schema_version')"; stop
[[ $MODE == postgres ]] && docker rm -f sg-upgrade-pg >/dev/null
echo "== $MODE OK"
