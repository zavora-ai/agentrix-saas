#!/usr/bin/env bash
# SC2016: the single-quoted ssh commands expand on the remote host, on purpose.
# shellcheck disable=SC2016
# Agentrix · stack smoke test, run from the operator's machine (task S1-12; R14.1–R14.3).
#
#   deploy/smoke/stack.sh [EXPECTED_COMMIT] [--quick]
#
# deploy/deploy.sh runs it after every deploy. It checks the services on control and the
# sandbox, the public HTTPS surface, that nothing else is reachable on control's public address,
# the runner's capabilities, and (unless --quick) the sandbox containment test,
# smoke/sandbox-rooms.sh, which includes a proxy started the way the runner starts it.
# Read-only, except that the rooms test starts and removes throwaway containers and one proxy.
# Exit status is the number of failed checks.
set -u
cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 99

STATE=deploy/.state
inv() { grep -E "^$1=" "$STATE/inventory.env" | tail -n 1 | cut -d= -f2-; }
PLATFORM=$(inv PLATFORM_FQDN)
CONTROL_IP=$(inv CONTROL_IP)
RUNNER=$(inv RUNNER_ADDR)
EXPECTED="" QUICK=0
for a in "$@"; do case $a in --quick) QUICK=1 ;; *) EXPECTED=$a ;; esac; done

# One SSH connection per host for the whole run: dozens of fresh connections through the jump
# host can time out in the banner exchange.
SSH_OPTS=(-F "$STATE/ssh_config" -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30
  -o ControlMaster=auto -o "ControlPath=/tmp/agx-ssh-%C" -o ControlPersist=120)
C=(ssh "${SSH_OPTS[@]}" agx-control)
S=(ssh "${SSH_OPTS[@]}" agx-sandbox)
fails=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); }
# is NAME GOT WANT
is() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', want '$3')"; fi; }
code() { curl -s -o /dev/null -m 15 -w '%{http_code}' "$@"; }

echo "== control"
is "caddy, agentrix-node and agentrix-models are active" \
  "$("${C[@]}" 'systemctl is-active caddy agentrix-node agentrix-models' | tr '\n' ' ')" "active active active "
if [ -n "$EXPECTED" ]; then
  is "the deployed commit is ${EXPECTED:0:12}" "$("${C[@]}" 'cut -d" " -f1 /opt/agentrix/bin/DEPLOYED 2>/dev/null')" "$EXPECTED"
fi
is "the runner answers control over the VPC (404 at /)" "$("${C[@]}" "curl -s -o /dev/null -m 5 -w '%{http_code}' http://$RUNNER/")" 404
is "the gateway's user can write node's database (usage, budget stop)" \
  "$("${C[@]}" "sudo -n -u agentrix-models python3 -I -c 'import sqlite3; c = sqlite3.connect(\"/var/lib/agentrix/control.db\", timeout=5, isolation_level=None); c.execute(\"BEGIN IMMEDIATE\"); c.execute(\"ROLLBACK\"); print(\"writable\")' 2>&1 | tail -n 1")" writable
is "the gateway has logged no read-only database errors since it started" \
  "$("${C[@]}" 'since=$(systemctl show -p ActiveEnterTimestamp --value agentrix-models); sudo -n journalctl -u agentrix-models --since "$since" --no-pager | grep -c "readonly database"')" 0
is "on-demand TLS refuses a host the platform does not serve" \
  "$("${C[@]}" "curl -s -o /dev/null -m 5 -w '%{http_code}' 'http://127.0.0.1:8079/internal/tls-ask?domain=smoke-unknown.invalid'")" 404
is "Caddy's running config asks node before issuing a certificate" \
  "$("${C[@]}" "curl -s -m 5 http://localhost:2019/config/apps/tls/automation/on_demand | grep -c 127.0.0.1:8079/internal/tls-ask")" 1

echo "== public: https://$PLATFORM"
for path in / /healthz /.well-known/agentrix/keys /account/signin /account/new; do
  is "GET $path is 200" "$(code "https://$PLATFORM$path")" 200
done
# The first release's sign-in pages are deprecated: they send people to the platform's own.
is "GET /auth/signin sends people to /account/signin" \
  "$(curl -s -o /dev/null -m 15 -w '%{http_code} %{redirect_url}' "https://$PLATFORM/auth/signin" | sed "s|https://$PLATFORM||; s|?.*||")" "303 /account/signin"
redirect=$(curl -s -o /dev/null -m 15 -w '%{http_code} %{redirect_url}' "http://$PLATFORM/")
case $redirect in
  "301 https://$PLATFORM/"|"308 https://$PLATFORM/") pass "http:// redirects to https://" ;;
  *) fail "http:// redirects to https:// (got '$redirect')" ;;
esac
if curl -s -D - -o /dev/null -m 15 "https://$PLATFORM/" | grep -qi '^strict-transport-security:'; then
  pass "HSTS is set"; else fail "HSTS is set"; fi
if echo | openssl s_client -servername "$PLATFORM" -connect "$PLATFORM:443" 2>/dev/null | openssl x509 -noout -checkend 604800 >/dev/null 2>&1; then
  pass "the certificate is valid for 7 more days"; else fail "the certificate is valid for 7 more days"; fi

echo "== only 80 and 443 answer on control's public address (R14.2)"
# curl exit 7 = refused, 28 = timed out: both mean closed. SSH (22) is the admin exception.
for port in 8079 8080 8090 2019 7443 3128; do
  curl -s -o /dev/null -m 4 "http://$CONTROL_IP:$port/"; rc=$?
  case $rc in 7|28) pass "port $port is closed" ;; *) fail "port $port is closed (curl exit $rc)" ;; esac
done

echo "== sandbox"
is "docker and agentrix-runner are active" "$("${S[@]}" 'systemctl is-active docker agentrix-runner' | tr '\n' ' ')" "active active "
is "the sandbox's nftables table is loaded" "$("${S[@]}" 'sudo -n nft list table inet agentrix >/dev/null 2>&1 && echo yes')" yes
# CAP_CHOWN(0) CAP_DAC_OVERRIDE(1) CAP_KILL(5) CAP_SETGID(6) CAP_SETUID(7) = 0xe3, nothing else.
runner=$("${S[@]}" 'p=$(systemctl show -p MainPID --value agentrix-runner); u=$(id -u agentrix); awk -v u="$u" '"'"'$1=="Uid:" { print ($2==u && $3==u && $4==u && $5==u) ? "uid agentrix" : "uid " $2 } $1~/^(CapEff|CapAmb|NoNewPrivs):$/ { print $1 " " $2 }'"'"' /proc/$p/status | tr "\n" ";"')
is "the runner runs as agentrix with exactly its five capabilities" "$runner" \
  "uid agentrix;CapEff: 00000000000000e3;CapAmb: 00000000000000e3;NoNewPrivs: 1;"

if [ "$QUICK" = 0 ]; then
  echo "== sandbox containment (smoke/sandbox-rooms.sh)"
  "${S[@]}" 'sudo -n bash -s' < deploy/smoke/sandbox-rooms.sh
  rc=$?; fails=$((fails + rc))
fi

echo "== $fails failed"
exit "$fails"
