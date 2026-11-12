#!/usr/bin/env bash
# Agentrix SaaS: this is the WebAgent Platform's three-VM (agx-*) deploy, kept only for reference.
# It must never run from this repository: it would reach the Vultr entry's servers and domain.
echo "$(basename "$0"): disabled in agentrix-saas (see specs/saas/README.md for this entry's deploy)" >&2; exit 2
# Agentrix · deploy a commit to the running stack (task S1-12; R14.1, R14.3).
#
#   deploy/deploy.sh [COMMIT] [--secrets] [--email] [--films DIR] [--quick] [--no-gauntlet]
#       Ships COMMIT (default HEAD) to control, builds it there (release, --locked), installs
#       node and models on control and the runner and egress on the sandbox, syncs the units,
#       settings and Caddyfile, restarts, runs the stack smoke test, then re-runs the gauntlet
#       on every live site (R13.2). --secrets also (re)writes control's provider keys from ./.env,
#       and the Telegram bot file from $AGENTRIX_TELEGRAM_FILE (default
#       ~/.config/agentrix/telegram.json) when there is one. --email writes RESEND_API_KEY (and
#       RESEND_WEBHOOK_SECRET, AGENTRIX_EMAIL_FROM when set) from ./.env, which switches real
#       email on. --films DIR uploads the home page's film (*.mp4, *.webm in DIR).
#       --quick skips the slower rooms smoke test; --no-gauntlet skips the gauntlet re-run.
#   deploy/deploy.sh --rollback     Puts back the binaries from before the last deploy, restarts,
#                                   and runs the smoke test. Units and settings are not rolled back.
#   deploy/deploy.sh --status       Which commit is deployed, and whether a deploy holds the lock.
#   deploy/deploy.sh --break-lock   Removes a lock left by a deploy that died.
#
# It deploys a committed tree (git archive, and the settings, units and Caddyfile from the same
# commit), never the working tree. One deploy at a time: a lock
# on control (/run/lock/agentrix-deploy) names who holds it, so two sessions cannot collide.
# Provider keys go from ./.env into root-only files on control through stdin and are never
# printed; the sandbox and fixture hold no secrets (N6). Needs deploy/provision.sh apply first
# (the VMs, deploy/.state/ssh_config and inventory.env). The fixture is deployed by its own lane.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

die() { printf 'deploy: %s\n' "$*" >&2; exit 1; }
say() { printf '== %s\n' "$*"; }
STATE=deploy/.state
[ -f "$STATE/ssh_config" ] && [ -f "$STATE/inventory.env" ] || die "run deploy/provision.sh apply first (no $STATE/ssh_config or inventory.env)"
# The inventory is parsed, never sourced.
inv() { grep -E "^$1=" "$STATE/inventory.env" | tail -n 1 | cut -d= -f2-; }
PLATFORM=$(inv PLATFORM_FQDN)
FIXTURE=$(inv FIXTURE_FQDN)
RUNNER=$(inv RUNNER_ADDR)
CONTROL_VPC_IP=$(inv CONTROL_VPC_IP)
[ -n "$PLATFORM" ] && [ -n "$RUNNER" ] && [ -n "$CONTROL_VPC_IP" ] || die "inventory.env lacks PLATFORM_FQDN, RUNNER_ADDR or CONTROL_VPC_IP"

# One SSH connection per host for the whole run: dozens of fresh connections through the jump
# host can time out in the banner exchange.
SSH_OPTS=(-F "$STATE/ssh_config" -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30
  -o ControlMaster=auto -o "ControlPath=/tmp/agx-ssh-%C" -o ControlPersist=120)
C=(ssh "${SSH_OPTS[@]}" agx-control)
S=(ssh "${SSH_OPTS[@]}" agx-sandbox)
SRC=/opt/agentrix/src
TARGET=/opt/agentrix/target           # outside src, so each deploy builds incrementally
REL=$TARGET/release
LOCK=/run/lock/agentrix-deploy
PACKAGES=(agentrix-node agentrix-models agentrix-sandbox agentrix-egress agentrix-gauntlet)
CONTROL_BINS=(agentrix-node agentrix-models agentrix-gauntlet)
SANDBOX_BINS=(agentrix-runner agentrix-egress)

# --- arguments ---------------------------------------------------------------------------
MODE=deploy COMMIT=HEAD SECRETS=0 EMAIL=0 CONCEPTS=0 FILMS="" QUICK=() GAUNTLET=1
while [ $# -gt 0 ]; do
  case $1 in
    --secrets) SECRETS=1 ;;
    --email) EMAIL=1 ;;
    --concepts) CONCEPTS=1 ;;
    --films) [ $# -ge 2 ] || die "--films needs a folder"; FILMS=$2; shift ;;
    --quick) QUICK=(--quick) ;;
    --no-gauntlet) GAUNTLET=0 ;;
    --rollback) MODE=rollback ;;
    --status) MODE=status ;;
    --break-lock) MODE=break-lock ;;
    -h|--help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) COMMIT=$1 ;;
  esac
  shift
done

deployed() { "${C[@]}" 'cat /opt/agentrix/bin/DEPLOYED 2>/dev/null || echo "nothing recorded"'; }
holder() { "${C[@]}" "cat $LOCK/holder 2>/dev/null" || true; }

case $MODE in
  status)
    printf 'deployed: %s\n' "$(deployed)"
    h=$(holder); printf 'lock:     %s\n' "${h:-free}"
    exit 0 ;;
  break-lock)
    h=$(holder); [ -n "$h" ] || { echo "the lock is free"; exit 0; }
    "${C[@]}" "rm -rf $LOCK"; echo "removed the lock held by: $h"; exit 0 ;;
esac

# --- the lock ----------------------------------------------------------------------------
SHA=""
if [ "$MODE" = deploy ]; then
  SHA=$(git rev-parse --verify --quiet "$COMMIT^{commit}") || die "$COMMIT is not a commit"
fi
ME="$(whoami)@$(hostname -s) pid $$ $MODE ${SHA:0:12} since $(date -u +%Y-%m-%dT%H:%M:%SZ)"
if ! "${C[@]}" "mkdir $LOCK 2>/dev/null && printf '%s\n' '$ME' > $LOCK/holder"; then
  die "another deploy holds the lock: $(holder). If it died, run deploy/deploy.sh --break-lock"
fi
TMP=$(mktemp -d)
# The one-time key control uses to copy the sandbox's programs over the VPC: gone from both hosts.
remove_push_key() {
  [ -n "${PUSH_KEY:-}" ] || return 0
  "${S[@]}" "sed -i '/agx-deploy-push/d' ~/.ssh/authorized_keys" 2>/dev/null || true
  "${C[@]}" "rm -f /tmp/agx-push /tmp/agx-push.pub" 2>/dev/null || true
  PUSH_KEY=""
}
cleanup() {
  remove_push_key
  rm -rf "$TMP"; "${C[@]}" "rm -rf $LOCK" 2>/dev/null || true
}
trap cleanup EXIT

# Gives group agentrix write on node's database and its -wal/-shm (the gateway, agentrix-models,
# writes usage rows there). SQLite creates files 0644, which the units' UMask=0007 makes 0640,
# and -wal/-shm copy the database's mode. Prints "changed" if it changed anything.
read -r -d '' DB_MODES <<'EOF' || true
sudo -n sh -c 'c=; for f in /var/lib/agentrix/control.db /var/lib/agentrix/control.db-wal /var/lib/agentrix/control.db-shm; do [ -e "$f" ] || continue; [ "$(stat -c %a:%G "$f")" = 660:agentrix ] && continue; chgrp agentrix "$f" && chmod 0660 "$f" && c=changed; done; echo "$c"'
EOF

# Dependencies first, so a Move that node resumes when it starts finds them up: the runner on
# the sandbox, then the gateway, then node.
restart_all() {
  say "restart the runner on the sandbox"
  # The runner needs control's job key, which node creates on its first start: on a new sandbox
  # it waits, and the job key step below starts it.
  "${S[@]}" "sudo -n sh -c 'systemctl daemon-reload; systemctl enable agentrix-runner >/dev/null 2>&1; if [ -s /etc/agentrix/control-job.pub ]; then systemctl restart agentrix-runner; sleep 2; systemctl is-active agentrix-runner; else systemctl stop agentrix-runner; echo \"waiting for the control job key (first install); started after node\"; fi'"
  say "restart models, then node, on control"
  "${C[@]}" "sudo -n systemctl daemon-reload && sudo -n systemctl enable agentrix-node agentrix-models >/dev/null 2>&1"
  "${C[@]}" "$DB_MODES" >/dev/null
  # A new control has no database yet: node creates it on its first start, and the gateway
  # exits until it can open it. So on a first install node starts first.
  if [ "$("${C[@]}" 'sudo -n test -e /var/lib/agentrix/control.db && echo yes || echo no')" = no ]; then
    say "first start: node creates the database before the gateway opens it"
    "${C[@]}" "sudo -n systemctl restart agentrix-node && for i in \$(seq 1 60); do sudo -n test -e /var/lib/agentrix/control.db && exit 0; sleep 1; done; echo 'node made no database in 60 s' >&2; sudo -n journalctl -u agentrix-node -n 20 --no-pager >&2; exit 1"
    "${C[@]}" "$DB_MODES" >/dev/null
  fi
  "${C[@]}" "sudo -n systemctl restart agentrix-models && for i in \$(seq 1 30); do c=\$(curl -s -o /dev/null -m 2 -w '%{http_code}' http://127.0.0.1:8090/); [ \"\$c\" != 000 ] && exit 0; sleep 1; done; echo 'models did not answer in 30 s' >&2; sudo -n journalctl -u agentrix-models -n 20 --no-pager >&2; exit 1"
  "${C[@]}" "sudo -n systemctl restart agentrix-node"
  "${C[@]}" "for i in \$(seq 1 30); do c=\$(curl -s -o /dev/null -m 2 -w '%{http_code}' -H 'Host: $PLATFORM' http://127.0.0.1:8080/healthz); [ \"\$c\" = 200 ] && exit 0; sleep 1; done; echo 'node did not answer /healthz in 30 s' >&2; sudo -n journalctl -u agentrix-node -n 20 --no-pager >&2; exit 1"
  # On a fresh install node has just created the database: fix its modes, and restart models,
  # since a connection opened while the file was read-only stays read-only.
  if [ "$("${C[@]}" "$DB_MODES")" = changed ]; then
    echo "database modes fixed; restarting models"; "${C[@]}" "sudo -n systemctl restart agentrix-models && sleep 2"
  fi
  "${C[@]}" "systemctl is-active agentrix-models agentrix-node"
}

# R13.2: the gauntlet score is recomputed from receipts on every deploy. For each live site
# (a site_pointer), it re-verifies the receipts of the site's latest Move and of the assessment
# before it (attacks 1-9), fires attack 10 at the live site again, and records a gauntlet.run
# receipt. A score below 10/10 is reported loudly; nothing is rolled back (the owner decides).
GAUNTLET_LOW=0
run_gauntlet() {
  say "gauntlet on each live site (R13.2)"
  local sites site domain since host out score digest
  sites=$("${C[@]}" 'sudo -n python3 -I -' <<'EOF'
import datetime, sqlite3
c = sqlite3.connect("file:/var/lib/agentrix/control.db?mode=ro", uri=True, timeout=10)
rows = c.execute("""SELECT p.site_id, s.domain,
  (SELECT m.created_at FROM moves m WHERE m.site_id = p.site_id ORDER BY m.created_at DESC LIMIT 1)
  FROM site_pointer p JOIN sites s USING (site_id) ORDER BY p.site_id""").fetchall()
for site, domain, move_at in rows:
    if not move_at:
        print(site, domain, "-")
        continue
    # Attacks 1 and 2 are proven by the assessment's crawl, which ran before the Move started:
    # the window opens at the latest assessment at or before the Move.
    (asmt_at,) = c.execute("SELECT MAX(created_at) FROM assessments WHERE site_id = ? AND created_at <= ?",
                           (site, move_at)).fetchone()
    started = asmt_at or move_at
    t = datetime.datetime.fromisoformat(started.replace("Z", "+00:00"))
    t = t if t.tzinfo else t.replace(tzinfo=datetime.timezone.utc)
    print(site, domain, t.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
EOF
  ) || { echo "could not read the live sites; gauntlet skipped" >&2; GAUNTLET_LOW=1; return 0; }
  [ -n "$sites" ] || { echo "no live site yet; gauntlet skipped"; return 0; }
  while read -r site domain since; do
    if ! [[ $site =~ ^[A-Za-z0-9_-]+$ && $domain =~ ^[A-Za-z0-9.-]+$ ]]; then
      echo "skipped a site with an unexpected id or domain: $site $domain" >&2; GAUNTLET_LOW=1; continue
    fi
    [ "$since" = - ] && { echo "$site ($domain): no Move on record; skipped"; continue; }
    # Attack 10 goes to the live site at its platform address, {domain-with-dashes}.<platform>,
    # which the wildcard record points at control (the owner's own domain may point elsewhere).
    host="$(printf '%s' "${domain%.}" | tr 'A-Z.' 'a-z-').$PLATFORM"
    out=$("${C[@]}" "sudo -n /opt/agentrix/bin/agentrix-gauntlet --platform http://127.0.0.1:8080 --fixture https://$FIXTURE --site $site --site-url https://$host --since $since --token-file /etc/agentrix/secrets/gauntlet.token --wait 0 --record --json --require 0" 2> "$TMP/gauntlet-$site.log") || true
    sed 's/^/  /' "$TMP/gauntlet-$site.log"
    score=$(printf '%s' "$out" | jq -r '"\(.score)/\(.out_of)"' 2>/dev/null || echo "?/?")
    digest=$(grep -o 'gauntlet.run receipt [^ ]*' "$TMP/gauntlet-$site.log" | awk '{print $3}' | tail -n 1)
    printf 'gauntlet  %s  %s  gauntlet.run %s\n' "$host" "$score" "${digest:-not recorded}"
    if [ "$score" != 10/10 ] || [ -z "$digest" ]; then
      GAUNTLET_LOW=1
      printf '\n!!! GAUNTLET %s ON %s (since %s): not 10/10 or not recorded. Nothing was rolled back;\n!!! the owner decides. deploy/deploy.sh --rollback puts back the previous binaries.\n\n' "$score" "$host" "$since"
    fi
  done <<< "$sites"
}

if [ "$MODE" = rollback ]; then
  say "roll back to the binaries before the last deploy"
  "${C[@]}" 'test -d /opt/agentrix/bin.prev' || die "control has no previous binaries (/opt/agentrix/bin.prev)"
  "${S[@]}" 'test -d /opt/agentrix/bin.prev' || die "the sandbox has no previous binaries (/opt/agentrix/bin.prev)"
  swap='sudo -n sh -c "set -e; rm -rf /opt/agentrix/bin.next; mv /opt/agentrix/bin /opt/agentrix/bin.next; mv /opt/agentrix/bin.prev /opt/agentrix/bin; mv /opt/agentrix/bin.next /opt/agentrix/bin.prev"'
  "${S[@]}" "sudo -n systemctl stop agentrix-runner; $swap"
  "${C[@]}" "$swap"
  restart_all
  printf 'deployed: %s\n' "$(deployed)"
else
  # --- ship and build --------------------------------------------------------------------
  say "ship $(git log -1 --format='%h %s' "$SHA") to control (changed files only)"
  "${C[@]}" "sudo -n install -d -o deploy -g deploy $SRC $TARGET"
  mkdir "$TMP/src" && git archive "$SHA" | tar -x -C "$TMP/src"
  if command -v rsync >/dev/null && "${C[@]}" 'command -v rsync >/dev/null'; then
    # Only changed files cross this machine's link, compressed; unchanged files keep their dates
    # on control, so cargo rebuilds only what changed. (The whole tree is about 29 MB.)
    rsync -rlpcz --delete --exclude=/.commit -e "$(printf '%q ' ssh "${SSH_OPTS[@]}")" "$TMP/src/" "agx-control:$SRC/"
  else
    "${C[@]}" "find $SRC -mindepth 1 -delete"
    git archive "$SHA" | "${C[@]}" "tar -x -C $SRC"
  fi
  "${C[@]}" "printf '%s\n' $SHA > $SRC/.commit"

  # systemd will not start a unit whose LoadCredential= file is missing, so check them all on
  # control before building or restarting anything.
  say "credential files the node unit needs"
  missing=""
  while IFS= read -r path; do
    "${C[@]}" "sudo -n test -s '$path'" || missing="$missing $path"
  done < <(git show "$SHA:deploy/systemd/agentrix-node.service" | sed -n 's/^LoadCredential=[^:]*:\(.*\)$/\1/p' | grep -v -E '/(gauntlet\.token|objects\.json|paypal\.json|telegram\.json)$')
  # These four are exempt: the host setup below makes them when missing (empty or no bot = off).
  [ -z "$missing" ] || die "missing on control:$missing. node would not start, so nothing was changed. Write it first (deploy/README.md, Secrets)."

  say "host setup on control (idempotent)"
  "${C[@]}" 'sudo -n sh -s' <<'EOF'
set -e
# Build tools for the release build (webauthn-rs links OpenSSL).
dpkg -s build-essential pkg-config libssl-dev clang >/dev/null 2>&1 || {
  apt-get update -q >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -q build-essential pkg-config libssl-dev clang >/dev/null; }
# The Model Gateway's own user: in group agentrix (the shared database), not able to read node's keys.
id agentrix-models >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /var/lib/agentrix --shell /usr/sbin/nologin --gid agentrix agentrix-models
chown agentrix:agentrix /var/lib/agentrix && chmod 2770 /var/lib/agentrix
install -d -m 0700 /etc/agentrix/secrets
test -s /etc/agentrix/secrets/gauntlet.token || (umask 077; openssl rand -hex 32 > /etc/agentrix/secrets/gauntlet.token)
# Object Storage is optional (R14.4, a Should). Node's unit loads objects.json as a credential,
# so it must exist: empty means off, and releases and evidence stay on control's disk.
test -e /etc/agentrix/secrets/objects.json || install -m 0600 /dev/null /etc/agentrix/secrets/objects.json
if test -s /etc/agentrix/secrets/objects.json; then echo "Object Storage: configured (objects.json)"; else echo "Object Storage: off (objects.json is empty; see deploy/README.md, Secrets)"; fi
# PayPal sandbox keys (platform W6, agentrix-effects) are optional the same way: empty means off.
test -e /etc/agentrix/secrets/paypal.json || install -m 0600 /dev/null /etc/agentrix/secrets/paypal.json
if test -s /etc/agentrix/secrets/paypal.json; then echo "PayPal: configured (paypal.json)"; else echo "PayPal: off (paypal.json is empty)"; fi
# The Telegram bot file (platform W13, agentrix-channels): a file that lists no bot means off.
test -e /etc/agentrix/secrets/telegram.json || (umask 077; printf '{"bots":[]}\n' > /etc/agentrix/secrets/telegram.json)
if grep -q '"token"' /etc/agentrix/secrets/telegram.json; then echo "Telegram: configured (telegram.json)"; else echo "Telegram: off (telegram.json lists no bot)"; fi
# The home page's film (AGENTRIX_HOME_FILM_DIR in node.env); --films fills it.
install -d -o agentrix -g agentrix -m 2750 /var/lib/agentrix/film
# Vultr's Ubuntu image enables ufw with only 22 open; both ufw and nftables must accept.
if command -v ufw >/dev/null && ufw status | grep -q 'Status: active'; then
  ufw allow 80/tcp >/dev/null; ufw allow 443/tcp >/dev/null
fi
EOF

  say "build on control (release, --locked)"
  pkgs=(); for p in "${PACKAGES[@]}"; do pkgs+=(-p "$p"); done
  "${C[@]}" "cd $SRC && export PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=$TARGET && cargo build --release --locked -q ${pkgs[*]}"

  # --- control: settings, units, Caddy, secrets ------------------------------------------
  say "settings, units and Caddyfile on control"
  # From the commit being deployed, like the binaries; never from the working tree. The runner's
  # address comes from the inventory, so a sandbox that provision.sh wake brought back at a new
  # VPC address needs no commit.
  for f in node.env models.env; do
    git show "$SHA:deploy/env/$f" | sed "s|^AGENTRIX_RUNNER=.*|AGENTRIX_RUNNER=http://$RUNNER|" |
      "${C[@]}" "sudo -n install -m 0644 /dev/stdin /etc/agentrix/$f"
  done
  for u in agentrix-node.service agentrix-models.service; do
    git show "$SHA:deploy/systemd/$u" | "${C[@]}" "sudo -n install -m 0644 /dev/stdin /etc/systemd/system/$u"
  done
  git show "$SHA:deploy/caddy/control.Caddyfile" > "$TMP/control.Caddyfile"
  deploy/provision.sh render-file "$TMP/control.Caddyfile" > "$TMP/Caddyfile"
  "${C[@]}" "sudo -n install -m 0644 /dev/stdin /etc/agentrix/Caddyfile.new" < "$TMP/Caddyfile"
  "${C[@]}" 'sudo -n sh -s' <<'EOF'
set -e
caddy validate --adapter caddyfile --config /etc/agentrix/Caddyfile.new >/dev/null 2>&1 || {
  echo "the rendered Caddyfile does not validate; Caddy left as it was:" >&2
  caddy validate --adapter caddyfile --config /etc/agentrix/Caddyfile.new >&2 || true; exit 1; }
if cmp -s /etc/agentrix/Caddyfile.new /etc/caddy/Caddyfile; then
  rm -f /etc/agentrix/Caddyfile.new; echo "Caddyfile unchanged"
else
  install -m 0644 /etc/agentrix/Caddyfile.new /etc/caddy/Caddyfile && mv /etc/agentrix/Caddyfile.new /etc/agentrix/Caddyfile
  systemctl reload caddy && echo "Caddyfile changed; Caddy reloaded"
fi
EOF

  if [ "$SECRETS" = 1 ]; then
    say "provider keys from ./.env into /etc/agentrix/secrets/models.env (not printed)"
    [ -f .env ] || die "no ./.env with the provider keys"
    python3 -I - <<'EOF' | "${C[@]}" 'sudo -n sh -c "umask 077; cat > /etc/agentrix/secrets/models.env"'
import re, pathlib
env = pathlib.Path(".env").read_text()
for name in ("ANTHROPIC_API_KEY", "VULTR_INFERENCE_API_KEY"):
    m = re.search(rf"(?m)^{name}=(.*)$", env)
    if m and m.group(1).strip():
        print(f"{name}={m.group(1).strip()}")
EOF
    tg=${AGENTRIX_TELEGRAM_FILE:-$HOME/.config/agentrix/telegram.json}
    if [ -s "$tg" ]; then
      say "Telegram bot file from $tg into /etc/agentrix/secrets/telegram.json (not printed)"
      python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert isinstance(d.get("bots"), list)' "$tg" ||
        die "$tg isn't the expected JSON ({\"bots\":[{\"site\":…,\"token\":…}]})"
      "${C[@]}" 'sudo -n sh -c "umask 077; cat > /etc/agentrix/secrets/telegram.json"' < "$tg"
    fi
  fi

  # node's secret environment, rewritten whole: the email keys (--email) and the Inference key the
  # concept stage's Zavora CLI uses (--concepts). Pass both flags when both are wanted.
  if [ "$EMAIL" = 1 ] || [ "$CONCEPTS" = 1 ]; then
    say "keys from ./.env into /etc/agentrix/secrets/node.env (not printed):$([ "$EMAIL" = 1 ] && echo ' email')$([ "$CONCEPTS" = 1 ] && echo ' concept stage')"
    [ -f .env ] || die "no ./.env"
    EMAIL=$EMAIL CONCEPTS=$CONCEPTS python3 -I - <<'EOF' | "${C[@]}" 'sudo -n sh -c "umask 077; cat > /etc/agentrix/secrets/node.env"'
import os, re, pathlib, sys
env = pathlib.Path(".env").read_text()
def put(name):
    m = re.search(rf"(?m)^\s*(?:export\s+)?{name}\s*=\s*\"?([^\"\n]*)\"?\s*$", env)
    if m and m.group(1).strip():
        print(f"{name}={m.group(1).strip()}")
        return True
    return False
if os.environ["EMAIL"] == "1":
    if not [put(n) for n in ("RESEND_API_KEY", "RESEND_WEBHOOK_SECRET", "AGENTRIX_EMAIL_FROM")][0]:
        sys.exit("no RESEND_API_KEY in ./.env")
if os.environ["CONCEPTS"] == "1" and not put("VULTR_INFERENCE_API_KEY"):
    sys.exit("no VULTR_INFERENCE_API_KEY in ./.env")
EOF
  fi
  # The concept stage's Zavora CLI: built on control from the Vultr-provider branch into
  # /opt/agentrix/zavora/target (a long release build, done once), installed beside it, outside
  # /opt/agentrix/bin, which deploys replace.
  if [ "$CONCEPTS" = 1 ]; then
    say "Zavora CLI for the concept stage into /opt/agentrix/zavora/bin"
    "${C[@]}" 'sudo -n sh -c "set -e; z=/opt/agentrix/zavora; [ -x \$z/target/release/zavora-cli ] || { echo \"no Zavora build in \$z/target on control\" >&2; exit 1; }; install -d -m 0755 \$z/bin; install -m 0755 \$z/target/release/zavora-cli \$z/bin/zavora-cli.new; mv \$z/bin/zavora-cli.new \$z/bin/zavora-cli; \$z/bin/zavora-cli --version"' ||
      die "the concept stage needs Zavora CLI built on control"
  fi

  if [ -n "$FILMS" ]; then
    say "the home page's film from $FILMS into /var/lib/agentrix/film"
    ls "$FILMS"/*.mp4 >/dev/null 2>&1 || die "$FILMS holds no .mp4"
    (cd "$FILMS" && COPYFILE_DISABLE=1 tar -c --no-xattrs $(ls *.mp4 *.webm 2>/dev/null)) | "${C[@]}" 'sudo -n sh -c "set -e; d=/var/lib/agentrix; rm -rf \$d/film.new \$d/film.old; install -d -o agentrix -g agentrix -m 2750 \$d/film.new; tar -x -C \$d/film.new --no-same-owner --no-same-permissions; chown -R agentrix:agentrix \$d/film.new; chmod 0640 \$d/film.new/*; mv \$d/film \$d/film.old; mv \$d/film.new \$d/film; rm -rf \$d/film.old; ls \$d/film | tr \"\\n\" \" \"; echo"'
  fi

  # --- install on control, then the sandbox ---------------------------------------------
  say "install on control (the previous binaries kept in /opt/agentrix/bin.prev)"
  "${C[@]}" "sudo -n sh -c 'set -e; rm -rf /opt/agentrix/bin.prev; cp -a /opt/agentrix/bin /opt/agentrix/bin.prev; for b in ${CONTROL_BINS[*]}; do install -m 0755 $REL/\$b /opt/agentrix/bin/\$b; done; printf \"%s %s\\n\" $SHA \"\$(date -u +%Y-%m-%dT%H:%M:%SZ)\" > /opt/agentrix/bin/DEPLOYED'"

  say "runner and egress to the sandbox"
  "${S[@]}" "sudo -n sh -c 'rm -rf /opt/agentrix/bin.prev /opt/agentrix/bin.new; cp -a /opt/agentrix/bin /opt/agentrix/bin.prev; cp -a /opt/agentrix/bin /opt/agentrix/bin.new'"
  # Straight from control to the sandbox over the VPC, in seconds. The hosts forward no agent, so
  # control makes a one-time key, the sandbox accepts it from control's VPC address only, and both
  # are removed right after (and by cleanup if the run dies). If that fails, the programs go
  # through this machine as before.
  SANDBOX_VPC=$(inv SANDBOX_VPC_IP)
  direct=0
  if [ -n "$SANDBOX_VPC" ]; then
    PUSH_PUB=$("${C[@]}" "rm -f /tmp/agx-push /tmp/agx-push.pub; ssh-keygen -q -t ed25519 -N '' -C agx-deploy-push -f /tmp/agx-push && cat /tmp/agx-push.pub" 2>/dev/null) || PUSH_PUB=""
    if [ -n "$PUSH_PUB" ] && printf 'from="%s" %s\n' "$CONTROL_VPC_IP" "$PUSH_PUB" | "${S[@]}" 'umask 077; mkdir -p ~/.ssh; cat >> ~/.ssh/authorized_keys'; then
      PUSH_KEY=1 direct=1
      for b in "${SANDBOX_BINS[@]}"; do
        "${C[@]}" "ssh -i /tmp/agx-push -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new deploy@$SANDBOX_VPC 'sudo -n install -m 0755 /dev/stdin /opt/agentrix/bin.new/$b' < $REL/$b" || { direct=0; break; }
      done
      remove_push_key
    fi
  fi
  if [ "$direct" = 1 ]; then
    echo "copied over the VPC"
  else
    echo "the VPC copy didn't work; copying through this machine"
    for b in "${SANDBOX_BINS[@]}"; do "${C[@]}" "cat $REL/$b" > "$TMP/$b"; done
    for b in "${SANDBOX_BINS[@]}"; do
      "${S[@]}" "sudo -n install -m 0755 /dev/stdin /opt/agentrix/bin.new/$b" < "$TMP/$b"
    done
  fi
  git show "$SHA:deploy/systemd/agentrix-runner.service" | "${S[@]}" "sudo -n install -m 0644 /dev/stdin /etc/systemd/system/agentrix-runner.service"
  "${S[@]}" "sudo -n sh -s" <<EOF
set -e
# Vultr's Ubuntu image enables ufw with only 22 open; both ufw and nftables must accept.
if command -v ufw >/dev/null && ufw status | grep -q 'Status: active'; then
  ufw allow from $CONTROL_VPC_IP to any port ${RUNNER##*:} proto tcp >/dev/null
  ufw allow in on agx-r0 to 172.30.0.1 port 3128:3191 proto tcp >/dev/null
fi
systemctl stop agentrix-runner 2>/dev/null || true
rm -rf /opt/agentrix/bin.old && mv /opt/agentrix/bin /opt/agentrix/bin.old && mv /opt/agentrix/bin.new /opt/agentrix/bin && rm -rf /opt/agentrix/bin.old
printf '%s\n' $SHA > /opt/agentrix/bin/DEPLOYED
EOF

  restart_all

  # The runner trusts only control's job-signing key, which node publishes; refresh it after
  # node has started (it is created on node's first start), then restart the runner if it changed.
  say "control's job key to the sandbox"
  "${C[@]}" "curl -s -m 5 -H 'Host: $PLATFORM' http://127.0.0.1:8080/.well-known/agentrix/keys" |
    python3 -I -c 'import json,sys; print(json.load(sys.stdin)["keys"][0]["public_key"])' > "$TMP/control-job.pub"
  [ -s "$TMP/control-job.pub" ] || die "node published no job key"
  "${S[@]}" "sudo -n sh -c 't=\$(mktemp); cat > \$t; if cmp -s \$t /etc/agentrix/control-job.pub; then rm -f \$t; else install -m 0644 \$t /etc/agentrix/control-job.pub; rm -f \$t; systemctl restart agentrix-runner; echo \"job key changed; runner restarted\"; fi; sleep 2; systemctl is-active agentrix-runner'" < "$TMP/control-job.pub"
fi

# --- smoke test ----------------------------------------------------------------------------
expected=$("${C[@]}" 'cut -d" " -f1 /opt/agentrix/bin/DEPLOYED')
say "smoke test"
if deploy/smoke/stack.sh "$expected" ${QUICK[@]+"${QUICK[@]}"}; then
  say "$(git log -1 --format='%h %s' "$expected" 2>/dev/null || echo "$expected") is live at https://$PLATFORM; smoke test passed"
else
  n=$?; say "smoke test: $n failed (deploy/deploy.sh --rollback puts back the previous binaries)"; exit "$n"
fi
if [ "$GAUNTLET" = 1 ]; then
  run_gauntlet
  [ "$GAUNTLET_LOW" = 0 ] || { say "gauntlet below 10/10 or not recorded (see above)"; exit 3; }
else
  say "gauntlet skipped (--no-gauntlet)"
fi
