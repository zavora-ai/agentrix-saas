#!/usr/bin/env bash
# Agentrix SaaS · run the local Zavora ERA (hosted) demo instance for both companies' books.
#
#   deploy/saas/era/era.sh up        Postgres (compose project saas-era), Redis, ERA's API on :18180
#   deploy/saas/era/era.sh seed      both tenants, idempotent (seed.py)
#   deploy/saas/era/era.sh restart   restart ERA's API (after seed changes ENTITY_ID)
#   deploy/saas/era/era.sh status    health and tenants
#   deploy/saas/era/era.sh logs      tail ERA's log
#   deploy/saas/era/era.sh down      stop all three; the database volume stays
#   deploy/saas/era/era.sh reset --yes   drop saas-era's database and seed both tenants afresh
#                                    (new tenant ids; the logins in tenants.json are kept)
#
# ERA's code stays in its own repo (ERA_SRC, default ~/Developer/projects/zavora-era) and is
# never edited or copied here: this script builds it into this entry's cache with --locked and
# runs the binary. Secrets come from the repo's .env (ERA_DATABASE_PASSWORD, ERA_JWT_ACCESS_SECRET,
# ERA_JWT_REFRESH_SECRET), read line by line, never sourced or printed. State (the tenants' logins,
# logs, pids) lives in ERA_STATE_DIR, default ~/.config/agentrix-saas/era, mode 0700.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
common=$(git -C "$here" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
envfile=${SAAS_ENV_FILE:-${common:+$(dirname "$common")/.env}}
state=${ERA_STATE_DIR:-$HOME/.config/agentrix-saas/era}
era_src=${ERA_SRC:-$HOME/Developer/projects/zavora-era}
era_target=${ERA_TARGET_DIR:-$HOME/Developer/hackathons/.agentrix-saas-target/era}
era_bin=${ERA_BIN:-$era_target/debug/zavora-erp-api}
api_port=${ERA_API_PORT:-18180}
pg_port=${ERA_PG_PORT:-15433}
redis_port=${ERA_REDIS_PORT:-16380}
compose=(docker compose -p saas-era -f "$here/docker-compose.yml")
export ERA_PG_PORT=$pg_port ERA_REDIS_PORT=$redis_port

# One value from the .env, read line by line. A value starting with '#' counts as empty.
env_value() {
  [ -n "$envfile" ] && [ -f "$envfile" ] || { echo "no .env at ${envfile:-?} (set SAAS_ENV_FILE)" >&2; return 1; }
  awk -v k="$1" 'index($0, k "=") == 1 { v = substr($0, length(k) + 2); sub(/^["\047]/, "", v); sub(/["\047]$/, "", v); if (substr(v, 1, 1) == "#") v = ""; print v; exit }' "$envfile"
}

need() {
  local v
  v=$(env_value "$1")
  [ -n "$v" ] || { echo "$1 is empty in $envfile" >&2; exit 1; }
  printf '%s' "$v"
}

running() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

health() { curl -fsS --max-time 2 "http://127.0.0.1:$api_port/health" >/dev/null 2>&1; }

up() {
  mkdir -p "$state" && chmod 700 "$state"
  ERA_DATABASE_PASSWORD=$(need ERA_DATABASE_PASSWORD)
  export ERA_DATABASE_PASSWORD
  "${compose[@]}" up -d postgres >/dev/null
  for _ in $(seq 1 60); do
    "${compose[@]}" exec -T postgres pg_isready -U zavora -d zavora_era >/dev/null 2>&1 && break
    sleep 1
  done
  echo "postgres  127.0.0.1:$pg_port (compose saas-era)"

  if running "$state/redis.pid"; then
    echo "redis     127.0.0.1:$redis_port (local redis-server, running)"
  elif docker image inspect "${ERA_REDIS_IMAGE:-redis:7-alpine}" >/dev/null 2>&1; then
    "${compose[@]}" --profile redis up -d redis >/dev/null
    echo "redis     127.0.0.1:$redis_port (compose saas-era)"
  else
    command -v redis-server >/dev/null || { echo "no redis image and no redis-server" >&2; exit 1; }
    redis-server --port "$redis_port" --bind 127.0.0.1 --daemonize yes --dir "$state" \
      --pidfile "$state/redis.pid" --logfile "$state/redis.log" --save "" --appendonly no
    sleep 1
    echo "redis     127.0.0.1:$redis_port (local redis-server)"
  fi

  if [ ! -x "$era_bin" ]; then
    echo "building ERA's API into $era_target (first time takes a few minutes)"
    CARGO_TARGET_DIR=$era_target CARGO_PROFILE_DEV_DEBUG=0 \
      cargo build --locked --manifest-path "$era_src/Cargo.toml" -p zavora-erp-api --bin zavora-erp-api
  fi

  if running "$state/era-api.pid" && health; then
    echo "era api   http://127.0.0.1:$api_port (running)"
    return
  fi
  # ENTITY_ID is the tenant ERA's customer portal serves (CRM tickets come in through it).
  # seed.py sets it to Agentrix Ltd's tenant; until then a fixed placeholder keeps boots stable.
  [ -s "$state/entity_id" ] || uuidgen | tr 'A-Z' 'a-z' > "$state/entity_id"
  local jwt_access jwt_refresh
  jwt_access=$(need ERA_JWT_ACCESS_SECRET)
  jwt_refresh=$(need ERA_JWT_REFRESH_SECRET)
  (
    cd "$state" # ERA reads a .env from its working directory; this one has none.
    DATABASE_URL="postgres://zavora:$ERA_DATABASE_PASSWORD@127.0.0.1:$pg_port/zavora_era" \
    REDIS_URL="redis://127.0.0.1:$redis_port" \
    BIND_ADDR="127.0.0.1:$api_port" \
    JWT_ACCESS_SECRET=$jwt_access \
    JWT_REFRESH_SECRET=$jwt_refresh \
    ENTITY_ID=$(cat "$state/entity_id") \
    APP_ENV=${ERA_APP_ENV:-development} \
    LOGIN_RATE_LIMIT=${ERA_LOGIN_RATE_LIMIT:-120} \
    RUST_LOG=${ERA_RUST_LOG:-info} \
    nohup "$era_bin" >>"$state/era-api.log" 2>&1 &
    echo $! > "$state/era-api.pid"
  )
  for _ in $(seq 1 90); do
    health && { echo "era api   http://127.0.0.1:$api_port"; return; }
    running "$state/era-api.pid" || { echo "ERA's API exited; see $state/era-api.log" >&2; tail -20 "$state/era-api.log" >&2; exit 1; }
    sleep 1
  done
  echo "ERA's API did not answer /health in 90 s; see $state/era-api.log" >&2
  exit 1
}

stop_api() {
  if running "$state/era-api.pid"; then
    kill "$(cat "$state/era-api.pid")"
    for _ in $(seq 1 20); do running "$state/era-api.pid" || break; sleep 0.5; done
  fi
  rm -f "$state/era-api.pid"
}

down() {
  stop_api
  if running "$state/redis.pid"; then kill "$(cat "$state/redis.pid")"; fi
  rm -f "$state/redis.pid"
  ERA_DATABASE_PASSWORD=unused "${compose[@]}" --profile redis stop >/dev/null 2>&1 || true
  echo "stopped (the saas-era database volume is kept)"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  restart) stop_api; up ;;
  reset)
    [ "${2:-}" = "--yes" ] || { echo "reset drops the saas-era database (both companies' books); run: era.sh reset --yes" >&2; exit 2; }
    down
    ERA_DATABASE_PASSWORD=unused "${compose[@]}" --profile redis down -v >/dev/null 2>&1 || true
    rm -f "$state/entity_id"
    up
    python3 -I "$here/seed.py" --url "http://127.0.0.1:$api_port" --state "$state" --env "$envfile"
    stop_api; up
    ;;
  seed) python3 -I "$here/seed.py" --url "http://127.0.0.1:$api_port" --state "$state" --env "$envfile" ;;
  status)
    health && echo "era api   up on :$api_port" || echo "era api   down"
    if [ -f "$state/tenants.json" ]; then
      python3 -I -c 'import json, sys
for k, v in json.load(open(sys.argv[1])).items():
    print("tenant    %s: %s, entity %s" % (k, v.get("name"), v.get("entity_id")))' "$state/tenants.json"
    fi
    ;;
  logs) tail -n "${2:-50}" -f "$state/era-api.log" ;;
  *) sed -n '2,11p' "$0"; exit 2 ;;
esac
