#!/usr/bin/env bash
# Agentrix SaaS · deploy a commit to saas-node (made by deploy/saas/provision.sh apply).
#
#   deploy/saas/deploy.sh [COMMIT] [--books era|fake] [--shape v2|one|two] [--era] [--seed]
#                         [--models MODE] [--no-smoke]
#       Ships COMMIT (default HEAD) and builds it on the VM (release, --locked), then installs:
#         in SAAS_SHAPE v2 and two, /usr/local/bin/agentrix-saas and agentrix-saas.service (user
#           agentrix-saas, data in /var/lib/agentrix-saas, settings in
#           /etc/agentrix-saas/agentrix-saas.env, 0600, written from the repo's .env every deploy);
#         in SAAS_SHAPE one, agentrix-node (the platform with the money layer mounted, 8080/8079)
#           and agentrix-models (the gateway, 8090), each with its own 0600 settings file, after
#           agentrix-saas.service is stopped and disabled (never two processes on its data);
#         the AG Studio board (web/saas-board, built when it changed);
#         the home page's films from SAAS_FILMS_DIR (default .agentrix-local-w0/film-deploy, or
#           --films DIR) to /opt/agentrix-saas/film, when that folder is there;
#         mcp-erp (built with --features zavora when its commit changed);
#         Zavora ERA under compose project saas-era (ERA_* secrets from .env), rebuilt when ERA's
#           commit changed or with --era; seeded on the first deploy or with --seed;
#         Caddy for the hosts of SAAS_SHAPE (v2: Caddyfile.v2's four; one or two: spec v3's zone,
#           rendered from Caddyfile), once they answer with the VM's address;
#       then runs the smoke test. --books, --shape and --models (live|record|replay) override
#       SAAS_BOOKS, SAAS_SHAPE and AGENTRIX_MODELS_MODE from agentrix-saas.env and .env.
#   deploy/saas/deploy.sh --smoke         Each host answers over HTTPS with a valid certificate.
#   deploy/saas/deploy.sh --smoke-model   One small model call through the node's own gateway.
#   deploy/saas/deploy.sh --status        What is deployed, the services, disk and memory.
#   deploy/saas/deploy.sh --rollback      Puts back the previous agentrix-saas binary and restarts.
#   deploy/saas/deploy.sh --snapshot [--yes] [--wait]   Snapshots the VM (after submission).
#   deploy/saas/deploy.sh --owner-link    Writes the owner-console link to a local 0600 file.
#   deploy/saas/deploy.sh --break-lock    Removes a lock left by a deploy that died.
#   deploy/saas/deploy.sh --test-agents off|on|status   The test customer agents' off switch.
#   deploy/saas/deploy.sh --operator EMAIL ROLE [ROLE...]   Gives a platform account operator roles
#       (op.admin, op.sre, op.auditor...) in node's roles file, replacing that email's line. The
#       node re-reads the file every 5 minutes; then the person signs in on ops.<platform host>.
#
# It deploys committed trees only (git archive of COMMIT, of ERA's HEAD and of mcp-erp's HEAD),
# copied with rsync, never a working tree. Secrets go from .env to root-only files on the VM
# through SSH's stdin and are never printed. One deploy at a time: a lock on the VM
# (/run/lock/agentrix-saas-deploy) names its holder.
#
# Settings (environment, all optional): SAAS_ENV_FILE, SAAS_DEPLOY_STATE (as provision.sh),
# SAAS_PACKAGE (default saas-node) and SAAS_BIN (default agentrix-saas), ERA_SRC (default
# ~/Developer/projects/zavora-era), MCP_ERP_SRC (default ~/Developer/projects/mcp-servers/mcp-erp),
# SAAS_FILMS_DIR (default ~/Developer/hackathons/.agentrix-local-w0/film-deploy), and for alerts
# SAAS_TELEGRAM_FILE (default ~/.config/agentrix/telegram.json) and AGENTRIX_PLATFORM_ENV (the
# platform repo's .env, for RESEND_API_KEY and AGENTRIX_EMAIL_FROM).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
COMMON=$(git -C "$HERE" rev-parse --path-format=absolute --git-common-dir)
ENV_FILE=${SAAS_ENV_FILE:-$(dirname "$COMMON")/.env}
STATE=${SAAS_DEPLOY_STATE:-$HOME/.config/agentrix-saas/deploy}
PKG=${SAAS_PACKAGE:-saas-node}
BIN=${SAAS_BIN:-agentrix-saas}
ERA_SRC=${ERA_SRC:-$HOME/Developer/projects/zavora-era}
MCP_ERP_SRC=${MCP_ERP_SRC:-$HOME/Developer/projects/mcp-servers/mcp-erp}
FILMS_SRC=${SAAS_FILMS_DIR:-$HOME/Developer/hackathons/.agentrix-local-w0/film-deploy}
# Alerts (shape one): the Telegram bots file and Resend's key, as the platform's deploy carried them.
TELEGRAM_FILE=${SAAS_TELEGRAM_FILE:-$HOME/.config/agentrix/telegram.json}
TELEGRAM_TOKEN_FILE=${SAAS_TELEGRAM_TOKEN_FILE:-$HOME/.config/agentrix/telegram-bot.token}
PLATFORM_ENV=${AGENTRIX_PLATFORM_ENV:-$HOME/Developer/hackathons/agentrix-webagent-platform/.env}
# The films agentrix-home serves from AGENTRIX_HOME_FILM_DIR (crates/agentrix-home/src/film.rs).
FILMS=(hero.mp4 hero.webm film.mp4 film.webm classy.mp4 tana.mp4 salon.mp4)
# The hosts Caddy serves and the smoke test checks: shape_hosts SHAPE PLATFORM_HOST sets HOSTS.
# v2 is spec v2's four names; one and two are spec v3's judged build under the platform host
# (SAAS_PLATFORM_HOST), plus spec v2's names until they are retired.
V2_HOSTS=(money.agentrix.website kilele.money.agentrix.website pay.agentrix.website books.agentrix.website)
SAMPLES=(kilele-bakery-sample classy-perfumes-sample tana-valley-times-sample)
HOSTS=("${V2_HOSTS[@]}")
shape_hosts() {
  case $1 in
    v2) HOSTS=("${V2_HOSTS[@]}") ;;
    one) [ -n "${2:-}" ] || die "SAAS_SHAPE=one needs SAAS_PLATFORM_HOST"
       HOSTS=("$2" "ops.$2" "${SAMPLES[@]/%/.$2}" "pay.$2" "books.$2" "money.$2" "kilele.money.$2" "test-agent-amani.$2") ;;
    *) [ -n "${2:-}" ] || die "SAAS_SHAPE=$1 needs SAAS_PLATFORM_HOST"
       HOSTS=("$2" "ops.$2" "${SAMPLES[@]/%/.$2}" "pay.$2" "books.$2" "${V2_HOSTS[@]}") ;;
  esac
}
# A platform host (agentrix-node's), not the money layer's or ERA's.
platform_name() { case $1 in pay.*|books.*|money.*|kilele.money.*) return 1;; *) return 0;; esac; }
ERA_PATHS=(Cargo.toml Cargo.lock zavora-erp-core zavora-erp-api migrations zavora-erp-ui deploy/Caddyfile)
# The .env keys the node gets. Never VULTR_API_KEY (the account key stays on this Mac), never
# ERA_* (ERA's own file), never HEYGEN_* (the presenter pipeline runs here). The model keys go
# because the Model Gateway runs inside agentrix-saas, which fails at boot in live mode without.
CARRY='^(SAAS_|AGENTRIX_|PAYPAL_|KILELE_|ANTHROPIC_|VULTR_INFERENCE_|AG_STUDIO_|AG_GRID_)[A-Z0-9_]*='
LOCK=/run/lock/agentrix-saas-deploy

die() { printf 'deploy: %s\n' "$*" >&2; exit 1; }
say() { printf '== %s\n' "$*"; }
tilde() { case $1 in "$HOME"*) printf '~%s' "${1#"$HOME"}";; *) printf '%s' "$1";; esac; }

# --- arguments ---------------------------------------------------------------------------
MODE=deploy COMMIT=HEAD ERA_BUILD=0 SEED=0 MODELS="" BOOKS="" SHAPE="" SMOKE=1 PASS=() TA_CMD="" OP_EMAIL="" OP_ROLES=""
while [ $# -gt 0 ]; do
  case $1 in
    --era) ERA_BUILD=1 ;;
    --seed) SEED=1 ;;
    --models) [ $# -ge 2 ] || die "--models needs live, record or replay"; MODELS=$2; shift ;;
    --books) [ $# -ge 2 ] || die "--books needs era or fake"; BOOKS=$2; shift ;;
    --shape) [ $# -ge 2 ] || die "--shape needs v2, one or two"; SHAPE=$2; shift ;;
    --smoke-model) MODE=smoke-model ;;
    --test-agents) [ $# -ge 2 ] || die "--test-agents needs off, on or status"; MODE=test-agents; TA_CMD=$2; shift ;;
    # The rest of the line is the email and its roles.
    --operator) [ $# -ge 3 ] || die "--operator needs an email and at least one role"
      MODE=operator; OP_EMAIL=$2; shift 2; OP_ROLES=$*; set -- end ;;
    --no-smoke) SMOKE=0 ;;
    --films) [ $# -ge 2 ] || die "--films needs a folder"; FILMS_SRC=$2; shift ;;
    --smoke) MODE=smoke ;;
    --status) MODE=status ;;
    --rollback) MODE=rollback ;;
    --snapshot) MODE=snapshot ;;
    --owner-link) MODE=owner-link ;;
    --break-lock) MODE=break-lock ;;
    --yes|--wait) PASS+=("$1") ;;
    -h|--help) sed -n '2,/^set -euo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) COMMIT=$1 ;;
  esac
  shift
done
case $MODELS in ""|live|record|replay) ;; *) die "--models takes live, record or replay";; esac
case $BOOKS in ""|era|fake) ;; *) die "--books takes era or fake";; esac
case $SHAPE in ""|v2|one|two) ;; *) die "--shape takes v2, one or two";; esac

# Snapshots go through the Vultr API, which provision.sh owns.
[ "$MODE" = snapshot ] && exec "$HERE/provision.sh" snapshot ${PASS[@]+"${PASS[@]}"}

# --- the VM ------------------------------------------------------------------------------
[ -f "$STATE/inventory.env" ] && [ -f "$STATE/ssh_config" ] || die "no saas-node yet: run deploy/saas/provision.sh apply first ($(tilde "$STATE"))"
inv() { grep -E "^$1=" "$STATE/inventory.env" | tail -n 1 | cut -d= -f2-; }
NODE_IP=$(inv NODE_IP)
[ -n "$NODE_IP" ] || die "inventory.env has no NODE_IP"
SSH=(ssh -F "$STATE/ssh_config" -o BatchMode=yes -o ControlMaster=auto -o "ControlPath=/tmp/saas-ssh-%C" -o ControlPersist=120)
R=("${SSH[@]}" saas-node)
RSH="ssh -F $STATE/ssh_config -o BatchMode=yes -o ControlMaster=auto -o ControlPath=/tmp/saas-ssh-%C -o ControlPersist=120"
case $STATE in *' '*) die "SAAS_DEPLOY_STATE must not contain spaces (rsync's -e)";; esac

smoke() {
  say "smoke: each host over HTTPS at $NODE_IP (certificate checked)"
  local h path code fail=0 want platform=1 note
  "${R[@]}" 'systemctl is-active --quiet agentrix-node' || platform=0
  for h in "${HOSTS[@]}"; do
    note=""
    case $h in books.*) path=/health; want=2;; pay.*) path=/; want=234;; *) path=/; want=23;; esac
    if [ $platform = 0 ] && platform_name "$h"; then want=2345; note="  (agentrix-node is not running: TLS only)"; fi
    code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 --resolve "$h:443:$NODE_IP" "https://$h$path" 2>/dev/null || true)
    if [ -n "$code" ] && [ "$code" != 000 ] && [[ $want == *"${code:0:1}"* ]]; then
      printf '   ok    https://%s%s  %s%s\n' "$h" "$path" "$code" "$note"
    else
      printf '   FAIL  https://%s%s  %s\n' "$h" "$path" "${code:-no answer}"; fail=1
    fi
  done
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 --resolve "${HOSTS[0]}:80:$NODE_IP" "http://${HOSTS[0]}/" 2>/dev/null || true)
  printf '   %-5s http://%s/ redirects  %s\n' "$( [ "$code" = 308 ] && echo ok || echo FAIL)" "${HOSTS[0]}" "${code:-no answer}"
  [ "$code" = 308 ] || fail=1
  [ $fail = 0 ] || die "smoke test failed"
  say "smoke: all ${#HOSTS[@]} hosts answer"
}

# The four names must answer with the VM's address at Vultr's name server, and have no AAAA
# (the wildcard's AAAA points at agx-control; an explicit A record hides it).
dns_ready() {
  local h a aaaa bad=0
  for h in "${HOSTS[@]}"; do
    a=$(dig +short A "$h" @ns1.vultr.com 2>/dev/null | tail -n 1)
    aaaa=$(dig +short AAAA "$h" @ns1.vultr.com 2>/dev/null | tail -n 1)
    if [ "$a" != "$NODE_IP" ] || [ -n "$aaaa" ]; then
      printf '   %s: A %s%s (want A %s, no AAAA)\n' "$h" "${a:-none}" "${aaaa:+, AAAA $aaaa}" "$NODE_IP"; bad=1
    fi
  done
  return $bad
}

# A setting deployed now, from the VM's settings file (empty when there is none yet).
deployed() { "${R[@]}" "sudo -n cat /etc/agentrix-saas/agentrix-node.env /etc/agentrix-saas/agentrix-saas.env 2>/dev/null | sed -n 's/^$1=//p' | tail -n 1" || true; }

# One small model call through the gateway inside agentrix-saas (its loopback address is in the
# journal since the node last started), as the Receivables agent's turns call it.
smoke_model() {
  say "smoke: one model call through the node's own gateway"
  "${R[@]}" 'sudo -n bash -s' <<'EOF'
set -euo pipefail
if systemctl is-active --quiet agentrix-models; then
  addr=127.0.0.1:8090
else
  since=$(systemctl show -p ActiveEnterTimestamp --value agentrix-saas)
  addr=$(journalctl -u agentrix-saas --since "$since" -o cat --no-pager | sed 's/\x1b\[[0-9;]*m//g' \
    | grep 'model gateway inside this process' | grep -oE '127\.0\.0\.1:[0-9]+' | tail -n 1 || true)
  [ -n "$addr" ] || { echo "no 'model gateway inside this process' line since the node started ($since)" >&2; exit 1; }
fi
out=$(curl -sS -m 120 -w '\n%{http_code}' "http://$addr/v1/chat/completions" -H 'content-type: application/json' \
  -H 'x-agentrix-site: saas-agentrix' -H 'x-agentrix-purpose: collect' -H 'x-agentrix-webagent: receivables' \
  -H 'x-agentrix-job: saas-company' -d '{"messages":[{"role":"user","content":"Reply with exactly one word: ready"}],"max_tokens":16}')
code=${out##*$'\n'}; body=${out%$'\n'*}
echo "   gateway $addr answered HTTP $code"
jq -r '"   model \(.model // "?"); reply \"\((.choices[0].message.content // "") | .[0:60])\"; tokens in \(.usage.prompt_tokens // "?"), out \(.usage.completion_tokens // "?")"' <<<"$body" 2>/dev/null \
  || printf '   %s\n' "${body:0:300}"
[ "${code:0:1}" = 2 ]
EOF
  say "smoke: the model answered through the node"
}

# The test customer agents' off switch: the off file present means paused (they check it before
# every visit).
test_agents() {
  local off; off=$(deployed TEST_AGENTS_OFF_FILE); off=${off:-/var/lib/agentrix-saas/test-agents/off}
  case $1 in
    off) "${R[@]}" "sudo -n install -m 0640 -o agentrix-saas -g agentrix-saas /dev/null $off && echo 'test agents: paused (off file present)'" ;;
    on) "${R[@]}" "sudo -n rm -f $off && echo 'test agents: running (off file removed)'" ;;
    status) "${R[@]}" "printf 'test agents: service %s; %s\n' \"\$(systemctl is-active agentrix-test-agents)\" \"\$(sudo -n test -e $off && echo 'paused (off file present)' || echo 'not paused')\"" ;;
    *) die "--test-agents takes off, on or status" ;;
  esac
}

# Node's operator roles file (agentrix-operator applies it at start and every 5 minutes): one
# person per line, `email role role...`. A line applies only to an email that already has a
# platform account. Deploys make the file once and never overwrite it; this replaces one line.
OPS_ROLES_FILE=/var/lib/agentrix-node/agentrix-operator/operators.txt
operator_grant() {
  local email=$1 roles=$2 r
  [[ $email =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] || die "--operator: '$email' isn't an email address"
  for r in $roles; do
    case $r in
      op.admin|op.sre|op.release|op.finance|op.support|op.partners|op.security|op.auditor|director) ;;
      *) die "--operator: '$r' isn't a role (op.admin op.sre op.release op.finance op.support op.partners op.security op.auditor director)" ;;
    esac
  done
  roles=$(tr -s ' ' '\n' <<<"$roles" | awk 'NF && !seen[$0]++' | tr '\n' ' '); roles=${roles% }
  # Both values are checked above (no spaces or quotes in the email, roles from a fixed list).
  "${R[@]}" "sudo -n env OPS_EMAIL=$email OPS_ROLES='$roles' OPS_FILE=$OPS_ROLES_FILE bash -s" <<'EOF'
set -euo pipefail
install -d -m 0750 -o agentrix-saas -g agentrix-saas "$(dirname "$OPS_FILE")"
[ -f "$OPS_FILE" ] || install -m 0600 -o agentrix-saas -g agentrix-saas /dev/null "$OPS_FILE"
tmp=$(mktemp)
awk -v e="$OPS_EMAIL" 'tolower($1) != tolower(e)' "$OPS_FILE" > "$tmp"
printf '%s %s\n' "$OPS_EMAIL" "$OPS_ROLES" >> "$tmp"
install -m 0600 -o agentrix-saas -g agentrix-saas "$tmp" "$OPS_FILE"
rm -f "$tmp"
printf 'roles file %s: %s person(s); %s now has %s.\n' "$OPS_FILE" "$(grep -cvE '^[[:space:]]*(#|$)' "$OPS_FILE")" "$OPS_EMAIL" "$OPS_ROLES"
echo "The node applies it within 5 minutes (journal: \"operator roles updated from the roles file\")."
EOF
}

case $MODE in
  test-agents) test_agents "$TA_CMD"; exit 0 ;;
  operator) operator_grant "$OP_EMAIL" "$OP_ROLES"; exit 0 ;;
  smoke) s=$(deployed SAAS_SHAPE); shape_hosts "${s:-v2}" "$(deployed SAAS_PLATFORM_HOST)"; smoke; exit 0 ;;
  smoke-model) smoke_model; exit 0 ;;
  status)
    # shellcheck disable=SC2016 # expands on the VM
    "${R[@]}" 'printf "deployed: "; cat /opt/agentrix-saas/DEPLOYED 2>/dev/null || echo nothing; printf "lock:     %s\n" "$(cat '"$LOCK"'/holder 2>/dev/null || echo free)"
      for s in agentrix-saas caddy docker; do printf "%-14s %s\n" "$s" "$(systemctl is-active $s)"; done
      sudo -n docker compose -p saas-era ps --format "table {{.Service}}\t{{.State}}\t{{.Status}}" 2>/dev/null
      df -h / | tail -n 1; free -m | sed -n 2p'
    exit 0 ;;
  break-lock)
    h=$("${R[@]}" "cat $LOCK/holder 2>/dev/null" || true)
    [ -n "$h" ] || { echo "the lock is free"; exit 0; }
    "${R[@]}" "sudo -n rm -rf $LOCK"; echo "removed the lock held by: $h"; exit 0 ;;
  owner-link)
    mkdir -p "$STATE" && chmod 700 "$STATE"
    console=money.agentrix.website
    [ "$(deployed SAAS_SHAPE)" != one ] || console=money.$(deployed SAAS_PLATFORM_HOST)
    ( umask 077; "${R[@]}" "sudo -n cat /etc/agentrix-saas/owner.env /etc/agentrix-saas/agentrix-saas.env /etc/agentrix-saas/agentrix-node.env 2>/dev/null | sed -n 's/^SAAS_OWNER_TOKEN=//p' | tail -n 1" \
      | sed "s|^|https://$console/console?key=|" > "$STATE/owner-link" )
    [ -s "$STATE/owner-link" ] || die "no owner token on the VM yet (deploy first)"
    echo "owner-console link written to $(tilde "$STATE/owner-link") (0600); it is not printed here"
    exit 0 ;;
esac

# --- deploy and rollback hold the lock ---------------------------------------------------
SHA=""
if [ "$MODE" = deploy ]; then
  SHA=$(git -C "$REPO" rev-parse --verify --quiet "$COMMIT^{commit}") || die "$COMMIT is not a commit"
  git -C "$REPO" cat-file -e "$SHA:deploy/saas/agentrix-saas.service" 2>/dev/null \
    || die "${SHA:0:12} has no deploy/saas/ (merge the deploy branch first): the unit, settings and Caddyfile come from the commit"
  git -C "$REPO" grep -q "^name = \"$PKG\"" "$SHA" -- 'crates/*/Cargo.toml' \
    || die "${SHA:0:12} has no package $PKG (SAAS_PACKAGE); saas-node lands from the company slice"
  [ -f "$ENV_FILE" ] || die "no .env at $(tilde "$ENV_FILE")"
fi
"${R[@]}" true || die "no SSH to saas-node at $NODE_IP (if this Mac's address changed: deploy/saas/provision.sh ssh-allow --yes)"
ME="$(whoami)@$(hostname -s) pid $$ $MODE ${SHA:0:12} since $(date -u +%Y-%m-%dT%H:%M:%SZ)"
"${R[@]}" "sudo -n mkdir $LOCK 2>/dev/null && printf '%s\n' '$ME' | sudo -n tee $LOCK/holder >/dev/null" \
  || die "another deploy holds the lock: $("${R[@]}" "cat $LOCK/holder 2>/dev/null" || true). If it died: deploy/saas/deploy.sh --break-lock"
TMP=$(mktemp -d)
cleanup() { rm -rf "$TMP"; "${R[@]}" "sudo -n rm -rf $LOCK" 2>/dev/null || true; }
trap cleanup EXIT

# restart_node UNIT PORT HOST: (re)starts UNIT and waits until it answers HOST on PORT.
restart_node() {
  "${R[@]}" "sudo -n env UNIT=$1 PORT=$2 HOSTH=$3 bash -s" <<'EOF'
set -euo pipefail
systemctl daemon-reload
systemctl enable "$UNIT" >/dev/null 2>&1
systemctl restart "$UNIT"
for _ in $(seq 1 90); do
  c=$(curl -s -o /dev/null -m 2 -w '%{http_code}' -H "Host: $HOSTH" "http://127.0.0.1:$PORT/" || true)
  case $c in [234]*) echo "   $UNIT answers on 127.0.0.1:$PORT for $HOSTH ($c)"; exit 0;; esac
  sleep 1
done
echo "$UNIT did not answer on 127.0.0.1:$PORT in 90 s" >&2
journalctl -u "$UNIT" -n 40 --no-pager >&2
exit 1
EOF
}

if [ "$MODE" = rollback ]; then
  [ "$(deployed SAAS_SHAPE)" != one ] || die "rollback puts back agentrix-saas only; in shape one, deploy the previous commit"
  say "rollback: the previous agentrix-saas binary"
  "${R[@]}" 'sudo -n sh -c "[ -x /usr/local/bin/agentrix-saas.prev ] || { echo no previous binary >&2; exit 1; }; mv /usr/local/bin/agentrix-saas /usr/local/bin/agentrix-saas.rolled-back && mv /usr/local/bin/agentrix-saas.prev /usr/local/bin/agentrix-saas && sed -i \"1s/^/rolled back from: /\" /opt/agentrix-saas/DEPLOYED"'
  restart_node agentrix-saas 18190 money.agentrix.website
  s=$(deployed SAAS_SHAPE); shape_hosts "${s:-v2}" "$(deployed SAAS_PLATFORM_HOST)"
  smoke
  exit 0
fi

# --- settings and secrets ----------------------------------------------------------------

# Carried .env lines, KEY=value only; a value starting with '#' (or empty) is left out. Values go
# verbatim into systemd's and compose's env files, so quotes, '$', '\' or spaces are refused.
env_lines() {
  awk -v re="$1" '$0 ~ re {
    k = substr($0, 1, index($0, "=") - 1); v = substr($0, index($0, "=") + 1)
    sub(/^["\047]/, "", v); sub(/["\047]$/, "", v)
    if (v == "" || substr(v, 1, 1) == "#") next
    if (v ~ /[$`"\047\\[:space:]]/) { print "deploy: " k " in .env holds a quote, $, \\ or a space; env files take it verbatim" > "/dev/stderr"; exit 3 }
    print k "=" v }' "$ENV_FILE"
}

assemble_env() {
  local base shape plat
  base=$(
    printf '# /etc/agentrix-saas/agentrix-saas.env: written by deploy/saas/deploy.sh at %s for %s.\n' "$(date -u +%Y-%m-%dT%H:%MZ)" "${SHA:0:12}"
    printf '# From the repo .env (the keys the node uses), then deploy/saas/agentrix-saas.env. Do not edit here.\n'
    env_lines "$CARRY"
    grep -vE '^[[:space:]]*(#|$)' "$TMP/src/deploy/saas/agentrix-saas.env"
    [ -z "$MODELS" ] || printf 'AGENTRIX_MODELS_MODE=%s\n' "$MODELS"
    [ -z "$BOOKS" ] || printf 'SAAS_BOOKS=%s\n' "$BOOKS"
    [ -z "$SHAPE" ] || printf 'SAAS_SHAPE=%s\n' "$SHAPE"
  )
  printf '%s\n' "$base"
  shape=$(sed -n 's/^SAAS_SHAPE=//p' <<<"$base" | tail -n 1)
  plat=$(sed -n 's/^SAAS_PLATFORM_HOST=//p' <<<"$base" | tail -n 1)
  case ${shape:-v2} in
    two)
      [ -n "$plat" ] || die "SAAS_SHAPE=$shape needs SAAS_PLATFORM_HOST"
      printf '# Spec v3: the node answers its agent door and books under the platform host.\n'
      printf 'SAAS_AGENT_DOOR_HOST=pay.%s\nSAAS_BOOKS_HOST=books.%s\n' "$plat" "$plat" ;;
  esac
}

era_secrets() {
  local k out; out=$(env_lines '^ERA_(DATABASE_PASSWORD|JWT_ACCESS_SECRET|JWT_REFRESH_SECRET)=')
  for k in ERA_DATABASE_PASSWORD ERA_JWT_ACCESS_SECRET ERA_JWT_REFRESH_SECRET; do
    grep -q "^$k=" <<<"$out" || die "$k is empty in $(tilde "$ENV_FILE")"
  done
  printf '%s\n' "$out"
}

# put_root FILE: stdin becomes FILE on the VM, root-only, replaced atomically.
put_root() { "${R[@]}" "sudo -n sh -c 'umask 077 && cat > $1.new && mv $1.new $1'"; }

# --- ship --------------------------------------------------------------------------------

# export_tree REPO REV DEST [PATHSPEC...]: a committed tree, never the working tree.
export_tree() {
  local repo=$1 rev=$2 dest=$3; shift 3
  mkdir -p "$dest"
  git -C "$repo" archive --format=tar "$rev" -- "$@" | tar -x -C "$dest"
}

# rsync without -t: files whose content is unchanged keep their time on the VM, so cargo
# rebuilds only what changed. node_modules and dist under the board are the VM's own.
push() { rsync -rlpz --checksum --delete --exclude=/web/saas-board/node_modules/ --exclude=/web/saas-board/dist/ -e "$RSH" "$1/" "saas-node:$2/"; }

say "ship ${SHA:0:12} ($(git -C "$REPO" log -1 --format=%s "$SHA" | cut -c1-70))"
export_tree "$REPO" "$SHA" "$TMP/src" . ':(exclude)specs/submission'
assemble_env > "$TMP/agentrix-saas.env"
SHAPE_NOW=$(sed -n 's/^SAAS_SHAPE=//p' "$TMP/agentrix-saas.env" | tail -n 1)
SHAPE_NOW=${SHAPE_NOW:-v2}
PLATFORM_NOW=$(sed -n 's/^SAAS_PLATFORM_HOST=//p' "$TMP/agentrix-saas.env" | tail -n 1)
shape_hosts "$SHAPE_NOW" "$PLATFORM_NOW"
case $SHAPE_NOW in v2) ERA_PUBLIC=https://books.agentrix.website;; *) ERA_PUBLIC=https://books.$PLATFORM_NOW;; esac
SETTINGS=agentrix-saas.env
if [ "$SHAPE_NOW" = one ]; then
  for pkg in agentrix-node agentrix-models; do
    git -C "$REPO" grep -q "^name = \"$pkg\"" "$SHA" -- 'crates/*/Cargo.toml' || die "${SHA:0:12} has no package $pkg"
  done
  SETTINGS=agentrix-node.env
  # The node and the money layer: no model provider key, and no SAAS_*_HOST, because the
  # module derives every host from AGENTRIX_ORIGIN (an explicit one would override it).
  {
    printf '# /etc/agentrix-saas/agentrix-node.env: written by deploy/saas/deploy.sh at %s for %s. Do not edit here.\n' "$(date -u +%Y-%m-%dT%H:%MZ)" "${SHA:0:12}"
    grep -vE '^(#|ANTHROPIC_|VULTR_INFERENCE_|SAAS_HOST=|SAAS_KILELE_HOST=|SAAS_AGENT_DOOR_HOST=|SAAS_BOOKS_HOST=|SAAS_ADDR=)' "$TMP/agentrix-saas.env"
    printf 'AGENTRIX_ORIGIN=https://%s\nAGENTRIX_LISTEN=127.0.0.1:8080\nAGENTRIX_INTERNAL_LISTEN=127.0.0.1:8079\n' "$PLATFORM_NOW"
    printf 'AGENTRIX_DATA_DIR=/var/lib/agentrix-node\nAGENTRIX_MODELS_URL=http://127.0.0.1:8090\nSAAS_MODULE=1\n'
    # Email alerts through Resend, from the platform's .env (never printed).
    if [ -f "$PLATFORM_ENV" ]; then ( ENV_FILE=$PLATFORM_ENV; env_lines '^(RESEND_API_KEY|AGENTRIX_EMAIL_FROM)=' ); fi
    # The model providers' Payout emails for Agentrix's model-spend bills: two sandbox personal
    # accounts (slice J). The node makes those vendors at boot, before the founder's mandate.
    b1=$(sed -n 's/^PAYPAL_BUYER_EMAIL_1=//p' "$TMP/agentrix-saas.env" | tail -n 1)
    b2=$(sed -n 's/^PAYPAL_BUYER_EMAIL_2=//p' "$TMP/agentrix-saas.env" | tail -n 1)
    [ -z "$b1" ] || printf 'SAAS_PAYEE_ANTHROPIC=%s\n' "$b1"
    [ -z "$b2" ] || printf 'SAAS_PAYEE_VULTR=%s\n' "$b2"
    # The live run's Telegram alerts: the bot's token from its file (never printed). The chat
    # (SAAS_ALERT_TELEGRAM_CHAT) and the email address (SAAS_ALERT_EMAIL) come from .env's SAAS_*.
    if [ -s "$TELEGRAM_TOKEN_FILE" ]; then
      tok=$(head -n 1 "$TELEGRAM_TOKEN_FILE" | tr -d '[:space:]')
      [[ $tok =~ ^[0-9]+:[A-Za-z0-9_-]+$ ]] || die "$(tilde "$TELEGRAM_TOKEN_FILE") doesn't hold a bot token"
      printf 'SAAS_ALERT_TELEGRAM_TOKEN=%s\n' "$tok"
    fi
  } > "$TMP/agentrix-node.env"
  # The test customer agents: their own settings, the Vultr key they call directly, the fast
  # model, and the sandbox PayPal app the sample sites take payments with.
  {
    printf '# /etc/agentrix-saas/agentrix-test-agents.env: written by deploy/saas/deploy.sh at %s for %s. Do not edit here.\n' "$(date -u +%Y-%m-%dT%H:%MZ)" "${SHA:0:12}"
    grep -E '^(TEST_AGENTS_|VULTR_INFERENCE_|AGENTRIX_TIER_FAST_VULTR=|PAYPAL_ENV=|PAYPAL_CLIENT_ID=|PAYPAL_CLIENT_SECRET=|RUST_LOG=)' "$TMP/agentrix-saas.env"
    printf 'TEST_AGENTS_PLATFORM_HOST=%s\n' "$PLATFORM_NOW"
  } > "$TMP/agentrix-test-agents.env"
  # The gateway: the provider keys, the routing, and the caps (the fleet's is SAAS_DAILY_MODEL_CAP_USD).
  {
    printf '# /etc/agentrix-saas/agentrix-models.env: written by deploy/saas/deploy.sh at %s for %s. Do not edit here.\n' "$(date -u +%Y-%m-%dT%H:%MZ)" "${SHA:0:12}"
    grep -E '^(ANTHROPIC_|VULTR_INFERENCE_|AGENTRIX_(PROVIDER|TIER_|ROUTE_|EMBED|RERANK|SCREEN_|VECTOR_STORE|STT|TTS|IMAGE|MODELS_MODE|MODELS_RECORDINGS|SITE_DAILY_CAP_USD|SAMPLES_DAILY_CAP_USD))' "$TMP/agentrix-saas.env"
    fleet=$(sed -n 's/^SAAS_DAILY_MODEL_CAP_USD=//p' "$TMP/agentrix-saas.env" | tail -n 1)
    [ -z "$fleet" ] || printf 'AGENTRIX_FLEET_DAILY_CAP_USD=%s\n' "$fleet"
    printf 'AGENTRIX_CONTROL_DB=/var/lib/agentrix-node/control.db\nAGENTRIX_MODELS_LISTEN=127.0.0.1:8090\n'
  } > "$TMP/agentrix-models.env"
fi
say "settings ($SETTINGS): $(grep -vE '^#' "$TMP/$SETTINGS" | cut -d= -f1 | sort -u | tr '\n' ' ')"
[ "$SHAPE_NOW" != one ] || say "settings (agentrix-models.env): $(grep -vE '^#' "$TMP/agentrix-models.env" | cut -d= -f1 | sort -u | tr '\n' ' ')"
[ "$SHAPE_NOW" != one ] || say "settings (agentrix-test-agents.env): $(grep -vE '^#' "$TMP/agentrix-test-agents.env" | cut -d= -f1 | sort -u | tr '\n' ' ')"
push "$TMP/src" /opt/agentrix-saas/src

ERA_SHA=$(git -C "$ERA_SRC" rev-parse --verify HEAD)
MCP_SHA=$(git -C "$MCP_ERP_SRC" rev-parse --verify HEAD)
[ -z "$(git -C "$ERA_SRC" status --porcelain -- "${ERA_PATHS[@]}")" ] || say "note: ERA has uncommitted changes; its HEAD ${ERA_SHA:0:12} is what ships"
[ -z "$(git -C "$MCP_ERP_SRC" status --porcelain)" ] || say "note: mcp-erp has uncommitted changes; its HEAD ${MCP_SHA:0:12} is what ships"

# --- mcp-erp -----------------------------------------------------------------------------
if [ "$("${R[@]}" 'cat /opt/mcp-erp/built.commit 2>/dev/null' || true)" != "$MCP_SHA" ]; then
  say "mcp-erp ${MCP_SHA:0:12}: ship and build (--features zavora)"
  export_tree "$MCP_ERP_SRC" "$MCP_SHA" "$TMP/mcp-erp" .
  rsync -rlpz --checksum --delete -e "$RSH" "$TMP/mcp-erp/" saas-node:/opt/mcp-erp/src/
  "${R[@]}" "cd /opt/mcp-erp/src && . ~/.cargo/env && CARGO_TARGET_DIR=/opt/mcp-erp/target cargo build --release --locked --features zavora --bin mcp-erp"
  "${R[@]}" "sudo -n sh -c 'install -d -m 0755 /usr/local/lib/mcp-erp && install -m 0755 /opt/mcp-erp/target/release/mcp-erp /usr/local/lib/mcp-erp/mcp-erp && install -m 0644 /opt/mcp-erp/src/mcp-server.toml /usr/local/lib/mcp-erp/mcp-server.toml' && echo $MCP_SHA > /opt/mcp-erp/built.commit"
else
  say "mcp-erp ${MCP_SHA:0:12}: already built"
fi

# --- build -------------------------------------------------------------------------------
if [ "$SHAPE_NOW" = one ]; then
  say "build agentrix-node, agentrix-models and the agentrix-saas command on the VM: release, --locked"
  "${R[@]}" "cd /opt/agentrix-saas/src && . ~/.cargo/env && CARGO_TARGET_DIR=/opt/agentrix-saas/target cargo build --release --locked -p agentrix-node -p agentrix-models -p $PKG -p saas-test-agents --bin agentrix-node --bin agentrix-models --bin $BIN --bin agentrix-test-agents"
else
  say "build $PKG ($BIN) on the VM: release, --locked (the first build takes a while)"
  "${R[@]}" "cd /opt/agentrix-saas/src && . ~/.cargo/env && CARGO_TARGET_DIR=/opt/agentrix-saas/target cargo build --release --locked -p $PKG --bin $BIN"
fi

BOARD_TREE=$(git -C "$REPO" rev-parse "$SHA:web/saas-board")
if [ "$("${R[@]}" 'cat /opt/agentrix-saas/board.tree 2>/dev/null' || true)" != "$BOARD_TREE" ]; then
  say "board: npm ci and build (web/saas-board changed)"
  "${R[@]}" "cd /opt/agentrix-saas/src/web/saas-board && npm ci --no-audit --no-fund --loglevel=error && npm run build && rsync -a --delete dist/ /opt/agentrix-saas/board/ && echo $BOARD_TREE > /opt/agentrix-saas/board.tree"
else
  say "board: unchanged"
fi

# --- films -------------------------------------------------------------------------------
if [ -d "$FILMS_SRC" ]; then
  have=()
  for f in "${FILMS[@]}"; do [ -f "$FILMS_SRC/$f" ] && have+=("$f"); done
  if [ ${#have[@]} -gt 0 ]; then
    say "films: ${have[*]} from $(tilde "$FILMS_SRC") to /opt/agentrix-saas/film"
    "${R[@]}" 'install -d -m 0755 /opt/agentrix-saas/film'
    rsync -rtz --checksum --delete "${FILMS[@]/#/--include=/}" --exclude='*' -e "$RSH" "$FILMS_SRC/" saas-node:/opt/agentrix-saas/film/
  else
    say "films: none of ${FILMS[*]} in $(tilde "$FILMS_SRC"); the home page shows its posters"
  fi
else
  say "films: no folder at $(tilde "$FILMS_SRC"); the home page shows its posters"
fi

# --- settings on the VM ------------------------------------------------------------------
if [ "$SHAPE_NOW" = one ]; then
  say "settings: /etc/agentrix-saas/agentrix-node.env, agentrix-models.env and era-secrets.env (root, 0600; not printed)"
  put_root /etc/agentrix-saas/agentrix-node.env < "$TMP/agentrix-node.env"
  put_root /etc/agentrix-saas/agentrix-models.env < "$TMP/agentrix-models.env"
  # The agents' clock starts when they (re)start, so a fixed end (TEST_AGENTS_END, UTC) becomes
  # the hours left now, a minute or two before the restart below; without one, TEST_AGENTS_HOURS.
  ta_end=$(sed -n 's/^TEST_AGENTS_END=//p' "$TMP/agentrix-test-agents.env" | tail -n 1)
  ta_hours=$(sed -n 's/^TEST_AGENTS_HOURS=//p' "$TMP/agentrix-test-agents.env" | tail -n 1)
  if [ -n "$ta_end" ]; then
    end_s=$(TZ=UTC date -j -f '%Y-%m-%dT%H:%M:%SZ' "$ta_end" +%s 2>/dev/null || date -u -d "$ta_end" +%s 2>/dev/null) \
      || die "TEST_AGENTS_END=$ta_end isn't a UTC time such as 2026-11-12T09:00:00Z"
    ta_hours=$(awk -v s=$(( end_s - $(date -u +%s) )) 'BEGIN { h = s / 3600; if (h < 0.02) h = 0.02; printf "%.2f", h }')
    say "test agents: run until $ta_end, TEST_AGENTS_HOURS=$ta_hours from this deploy"
  else
    say "test agents: TEST_AGENTS_HOURS=${ta_hours:-12} from this deploy (no TEST_AGENTS_END)"
  fi
  grep -vE '^TEST_AGENTS_(END|HOURS)=' "$TMP/agentrix-test-agents.env" \
    | { cat; [ -z "$ta_hours" ] || printf 'TEST_AGENTS_HOURS=%s\n' "$ta_hours"; } \
    | put_root /etc/agentrix-saas/agentrix-test-agents.env
  # The node's unit loads it as a credential and won't start without the file: none means off.
  if [ -s "$TELEGRAM_FILE" ] && jq -e '.bots | type == "array"' "$TELEGRAM_FILE" >/dev/null 2>&1; then
    put_root /etc/agentrix-saas/telegram.json < "$TELEGRAM_FILE"
    say "alerts: telegram.json with $(jq '.bots | length' "$TELEGRAM_FILE") bot(s) (root, 0600; not printed)"
  else
    printf '{"bots":[]}\n' | put_root /etc/agentrix-saas/telegram.json
    say "alerts: no telegram.json at $(tilde "$TELEGRAM_FILE"); Telegram off"
  fi
  for k in SAAS_ALERT_TELEGRAM_TOKEN SAAS_ALERT_TELEGRAM_CHAT SAAS_ALERT_EMAIL; do
    if grep -q "^$k=" "$TMP/agentrix-node.env"; then say "alerts: $k set (not printed)"; else say "alerts: $k not set"; fi
  done
  if grep -q '^RESEND_API_KEY=' "$TMP/agentrix-node.env"; then say "alerts: Resend's key from $(tilde "$PLATFORM_ENV") (not printed)"; else say "alerts: no RESEND_API_KEY; email off"; fi
else
  say "settings: /etc/agentrix-saas/agentrix-saas.env and era-secrets.env (root, 0600; not printed)"
  put_root /etc/agentrix-saas/agentrix-saas.env < "$TMP/agentrix-saas.env"
fi
era_secrets | put_root /etc/agentrix-saas/era-secrets.env

# --- proofs ------------------------------------------------------------------------------
# The commit's specs/saas/PROOFS.md and proofs/ to SAAS_PROOFS_DIR, where the node may fill them.
"${R[@]}" 'sudo -n bash -s' <<'EOF'
set -euo pipefail
src=/opt/agentrix-saas/src/specs/saas
dst=/var/lib/agentrix-saas/proofs
[ -f "$src/PROOFS.md" ] && [ -d "$src/proofs" ] || { echo "   proofs: none in this commit"; exit 0; }
install -d -m 0750 -o agentrix-saas -g agentrix-saas /var/lib/agentrix-saas "$dst"
rsync -a --delete "$src/proofs/" "$dst/proofs/"
install -m 0640 "$src/PROOFS.md" "$dst/PROOFS.md"
chown -R agentrix-saas:agentrix-saas "$dst"
echo "   proofs: PROOFS.md and proofs/ in $dst"
EOF

# --- ERA ---------------------------------------------------------------------------------
# era_up build|nobuild: era.env from the secrets, the tenant ERA's portal serves, and a
# notification key made once on the VM; then compose up and a health wait.
era_up() {
  "${R[@]}" "sudo -n env BUILD=$1 ERA_PUBLIC_URL=$ERA_PUBLIC bash -s" <<'EOF'
set -euo pipefail
cd /opt/agentrix-saas/src/deploy/saas
st=/var/lib/agentrix-saas/era
install -d -m 0700 -o agentrix-saas -g agentrix-saas "$st"
[ -s "$st/entity_id" ] || uuidgen | tr '[:upper:]' '[:lower:]' > "$st/entity_id"
[ -s /etc/agentrix-saas/era-notif.key ] || (umask 077; openssl rand -base64 32 > /etc/agentrix-saas/era-notif.key)
umask 077
{ cat /etc/agentrix-saas/era-secrets.env
  echo "ERA_ENTITY_ID=$(cat "$st/entity_id")"
  echo "ERA_NOTIF_ENC_KEY=$(cat /etc/agentrix-saas/era-notif.key)"
  echo "ERA_SRC=/opt/zavora-era"
  echo "ERA_PUBLIC_URL=$ERA_PUBLIC_URL"
} > /etc/agentrix-saas/era.env
compose=(docker compose -p saas-era --env-file /etc/agentrix-saas/era.env -f era/docker-compose.yml -f era.compose.yml --profile redis)
[ "$BUILD" = build ] && "${compose[@]}" build --pull
"${compose[@]}" up -d --remove-orphans
for _ in $(seq 1 120); do
  curl -fsS -m 2 http://127.0.0.1:18180/health >/dev/null 2>&1 && { echo "   ERA's API is healthy on 127.0.0.1:18180"; exit 0; }
  sleep 2
done
echo "ERA's API did not answer /health in 4 minutes" >&2
"${compose[@]}" logs --tail 40 api >&2
exit 1
EOF
}

if [ $ERA_BUILD = 1 ] || [ "$("${R[@]}" 'cat /opt/agentrix-saas/era.commit 2>/dev/null' || true)" != "$ERA_SHA" ]; then
  say "ERA ${ERA_SHA:0:12}: rsync its committed sources to /opt/zavora-era and build (compose saas-era)"
  export_tree "$ERA_SRC" "$ERA_SHA" "$TMP/era" "${ERA_PATHS[@]}"
  rsync -rlpz --checksum --delete -e "$RSH" "$TMP/era/" saas-node:/opt/zavora-era/
  era_up build
  "${R[@]}" "echo $ERA_SHA > /opt/agentrix-saas/era.commit"
else
  say "ERA ${ERA_SHA:0:12}: already built; compose up"
  era_up nobuild
fi

if [ $SEED = 1 ] || ! "${R[@]}" 'sudo -n test -s /var/lib/agentrix-saas/era/tenants.json'; then
  say "ERA: seed both tenants (idempotent; logins stay on the VM)"
  "${R[@]}" "sudo -n env SETTINGS=$SETTINGS bash -s" <<'EOF'
set -euo pipefail
st=/var/lib/agentrix-saas/era
python3 -I /opt/agentrix-saas/src/deploy/saas/era/seed.py --url http://127.0.0.1:18180 --state "$st" --env "/etc/agentrix-saas/$SETTINGS" \
  | sed 's/^/   /'
chown -R agentrix-saas:agentrix-saas "$st"; chmod 700 "$st"; chmod 600 "$st/tenants.json"
EOF
  say "ERA: restart its API with Agentrix Ltd's tenant as the portal's"
  era_up nobuild
fi

# --- Caddy -------------------------------------------------------------------------------
# Before the node starts: its agent doors read the companies' published key directories over
# https on their public hosts, so the certificates must already be there.
say "caddy: shape $SHAPE_NOW${PLATFORM_NOW:+, platform host $PLATFORM_NOW}, ${#HOSTS[@]} named hosts"
if dns_ready; then
  "${R[@]}" "sudo -n env SHAPE=$SHAPE_NOW PLAT=$PLATFORM_NOW bash -s" <<'EOF'
set -euo pipefail
src=/opt/agentrix-saas/src/deploy/saas
case $SHAPE in
  v2)  install -m 0644 "$src/Caddyfile.v2" /etc/caddy/Caddyfile.new ;;
  one) sed -e "s/@@HOST@@/$PLAT/g" -e 's/@@PLATFORM@@/127.0.0.1:8080/g' -e 's/@@MONEY@@/127.0.0.1:8080/g' -e 's/@@ASK@@/127.0.0.1:8079/g' -e 's/@@TESTAGENTS@@/127.0.0.1:18195/g' "$src/Caddyfile" > /etc/caddy/Caddyfile.new ;;
  two) sed -e "s/@@HOST@@/$PLAT/g" -e 's/@@PLATFORM@@/127.0.0.1:8080/g' -e 's/@@MONEY@@/127.0.0.1:18190/g' -e 's/@@ASK@@/127.0.0.1:8079/g' -e 's/@@TESTAGENTS@@/127.0.0.1:18195/g' "$src/Caddyfile" > /etc/caddy/Caddyfile.new ;;
esac
chmod 0644 /etc/caddy/Caddyfile.new
! grep -q '@@' /etc/caddy/Caddyfile.new || { echo "unfilled @@ token in the Caddyfile" >&2; exit 1; }
caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile.new >/dev/null 2>&1 \
  || { caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile.new >&2; exit 1; }
if ! cmp -s /etc/caddy/Caddyfile.new /etc/caddy/Caddyfile; then
  mv /etc/caddy/Caddyfile.new /etc/caddy/Caddyfile
  systemctl reload caddy
  echo "   Caddyfile installed and reloaded"
else
  rm -f /etc/caddy/Caddyfile.new
  echo "   Caddyfile unchanged"
fi
EOF
  for _ in $(seq 1 24); do
    curl -s -o /dev/null --max-time 10 --resolve "books.agentrix.website:443:$NODE_IP" "https://books.agentrix.website/health" && break
    sleep 5
  done
else
  say "caddy: NOT installed: the records above do not answer with $NODE_IP yet (Let's Encrypt would fail)"
  SMOKE=0
fi

# --- install the node --------------------------------------------------------------------
if [ "$SHAPE_NOW" = one ]; then
  say "install agentrix-node and agentrix-models; agentrix-saas.service stops for good"
  "${R[@]}" 'sudo -n bash -s' <<'EOF'
set -euo pipefail
rel=/opt/agentrix-saas/target/release
src=/opt/agentrix-saas/src/deploy/saas
for b in agentrix-node agentrix-models agentrix-test-agents; do
  [ -x "$rel/$b" ] || { echo "no $rel/$b" >&2; exit 1; }
  if [ -x "/usr/local/bin/$b" ] && ! cmp -s "$rel/$b" "/usr/local/bin/$b"; then cp -p "/usr/local/bin/$b" "/usr/local/bin/$b.prev"; fi
  install -m 0755 "$rel/$b" "/usr/local/bin/$b"
  install -m 0644 "$src/$b.service" "/etc/systemd/system/$b.service"
done
install -m 0644 "$src/agentrix-saas.service" /etc/systemd/system/agentrix-saas.service
# The agentrix-saas command (badge verify, passport tools); its service stays off in this shape.
[ ! -x "$rel/agentrix-saas" ] || install -m 0755 "$rel/agentrix-saas" /usr/local/bin/agentrix-saas
grep -qs '^SAAS_OWNER_TOKEN=' /etc/agentrix-saas/owner.env || (umask 077; printf 'SAAS_OWNER_TOKEN=%s\n' "$(openssl rand -hex 24)" > /etc/agentrix-saas/owner.env)
install -d -m 0750 -o agentrix-saas -g agentrix-saas /var/lib/agentrix-saas /var/lib/agentrix-saas/keys /var/lib/agentrix-node /var/lib/agentrix-saas/test-agents
# Node's operator roles file: made once, empty but for its header, and never overwritten here
# (deploy.sh --operator adds a person).
install -d -m 0750 -o agentrix-saas -g agentrix-saas /var/lib/agentrix-node/agentrix-operator
ops=/var/lib/agentrix-node/agentrix-operator/operators.txt
[ -f $ops ] || { printf '%s\n' "# Node's operator roles, re-read every 5 minutes: one person per line, 'email role role...'." \
  "# Roles: op.admin op.sre op.release op.finance op.support op.partners op.security op.auditor director." \
  "# Only an email with a platform account signs in. Edit with deploy/saas/deploy.sh --operator." > $ops
  chown agentrix-saas:agentrix-saas $ops; chmod 0600 $ops; }
# The test agents' control token (bearer for /control/stop and /control/start), made once here.
grep -qs '^TEST_AGENTS_CONTROL_TOKEN=' /etc/agentrix-saas/test-agents-control.env || (umask 077; printf 'TEST_AGENTS_CONTROL_TOKEN=%s\n' "$(openssl rand -hex 24)" > /etc/agentrix-saas/test-agents-control.env)
# Never two processes running the companies' kernels on /var/lib/agentrix-saas.
systemctl disable --now agentrix-saas >/dev/null 2>&1 || true
rm -f /etc/agentrix-saas/agentrix-saas.env
systemctl daemon-reload
EOF
  restart_node agentrix-node 8080 "$PLATFORM_NOW"
  "${R[@]}" 'sudo -n bash -s' <<'EOF'
set -euo pipefail
systemctl enable agentrix-models >/dev/null 2>&1
systemctl restart agentrix-models
for _ in $(seq 1 60); do
  c=$(curl -s -o /dev/null -m 2 -w '%{http_code}' http://127.0.0.1:8090/ || true)
  [ "$c" != 000 ] && { echo "   agentrix-models answers on 127.0.0.1:8090 ($c)"; exit 0; }
  sleep 1
done
echo "agentrix-models did not answer on 127.0.0.1:8090 in 60 s" >&2
journalctl -u agentrix-models -n 30 --no-pager >&2
exit 1
EOF
  "${R[@]}" 'sudo -n bash -s' <<'EOF'
set -euo pipefail
systemctl enable agentrix-test-agents >/dev/null 2>&1
systemctl restart agentrix-test-agents
for _ in $(seq 1 60); do
  c=$(curl -s -o /dev/null -m 2 -w '%{http_code}' -H 'Host: test-agents.x' http://127.0.0.1:18195/ || true)
  [ "$c" != 000 ] && { echo "   agentrix-test-agents answers on 127.0.0.1:18195 ($(test -e /var/lib/agentrix-saas/test-agents/off && echo paused || echo running))"; exit 0; }
  sleep 1
done
echo "agentrix-test-agents did not answer on 127.0.0.1:18195 in 60 s" >&2
journalctl -u agentrix-test-agents -n 30 --no-pager >&2
exit 1
EOF
else
say "install /usr/local/bin/agentrix-saas and agentrix-saas.service"
"${R[@]}" "sudo -n env BIN=$BIN bash -s" <<'EOF'
set -euo pipefail
# Back from shape one: agentrix-node and the gateway stop first.
systemctl disable --now agentrix-test-agents agentrix-models agentrix-node >/dev/null 2>&1 || true
rm -f /etc/agentrix-saas/agentrix-node.env /etc/agentrix-saas/agentrix-models.env /etc/agentrix-saas/agentrix-test-agents.env
new=/opt/agentrix-saas/target/release/$BIN
[ -x "$new" ] || { echo "no $new" >&2; exit 1; }
if [ -x /usr/local/bin/agentrix-saas ] && ! cmp -s "$new" /usr/local/bin/agentrix-saas; then
  cp -p /usr/local/bin/agentrix-saas /usr/local/bin/agentrix-saas.prev
fi
install -m 0755 "$new" /usr/local/bin/agentrix-saas
install -m 0644 /opt/agentrix-saas/src/deploy/saas/agentrix-saas.service /etc/systemd/system/agentrix-saas.service
grep -qs '^SAAS_OWNER_TOKEN=' /etc/agentrix-saas/owner.env || (umask 077; printf 'SAAS_OWNER_TOKEN=%s\n' "$(openssl rand -hex 24)" > /etc/agentrix-saas/owner.env)
install -d -m 0750 -o agentrix-saas -g agentrix-saas /var/lib/agentrix-saas /var/lib/agentrix-saas/keys
EOF
restart_node agentrix-saas 18190 money.agentrix.website
fi

"${R[@]}" "printf '%s\nera %s\nmcp-erp %s\n%s\n' '$SHA' '$ERA_SHA' '$MCP_SHA' 'deployed $(date -u +%Y-%m-%dT%H:%M:%SZ)' > /opt/agentrix-saas/DEPLOYED"
say "deployed ${SHA:0:12} (ERA ${ERA_SHA:0:12}, mcp-erp ${MCP_SHA:0:12})"

if [ $SMOKE = 1 ]; then
  for _ in $(seq 1 12); do
    curl -s -o /dev/null --max-time 10 --resolve "${HOSTS[0]}:443:$NODE_IP" "https://${HOSTS[0]}/" && break
    sleep 5
  done
  smoke
fi
