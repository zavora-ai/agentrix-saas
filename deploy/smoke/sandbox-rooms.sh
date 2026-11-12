#!/usr/bin/env bash
# SC2016: the sh -c strings are single-quoted on purpose (they run in a child shell).
# SC2329: as_egress and room are called through expect_ok and expect_refused.
# shellcheck disable=SC2016,SC2329
# Agentrix · containment smoke test, run ON the sandbox VM (as root, after cloud-init):
#
#   ssh -F deploy/.state/ssh_config agx-sandbox 'sudo bash -s' < deploy/smoke/sandbox-rooms.sh
#
# It checks the host the way a room sees it, using a throwaway busybox room on agx-rooms under
# gVisor. Every check that should fail must fail. Exit status is the number of failed checks.
# It changes nothing except pulling busybox and starting and removing one container.
set -u

# Export host.env so the sh -c checks below see it.
set -a
# shellcheck source=/dev/null
. /etc/agentrix/host.env
set +a
fails=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); }
# expect_ok NAME CMD...   /  expect_refused NAME CMD...
expect_ok() { local n=$1; shift; if "$@" >/dev/null 2>&1; then pass "$n"; else fail "$n"; fi; }
expect_refused() { local n=$1; shift; if "$@" >/dev/null 2>&1; then fail "$n (it worked)"; else pass "refused: $n"; fi; }

echo "== host"
expect_ok "nftables table inet agentrix is loaded" nft list table inet agentrix
expect_ok "Docker's default runtime is runsc" sh -c '[ "$(docker info --format "{{.DefaultRuntime}}")" = runsc ]'
expect_ok "agx-rooms exists and is internal" sh -c '[ "$(docker network inspect -f "{{.Internal}}" agx-rooms)" = true ]'
expect_ok "agx-rooms uses bridge agx-r0" sh -c '[ "$(docker network inspect -f "{{index .Options \"com.docker.network.bridge.name\"}}" agx-rooms)" = agx-r0 ]'
expect_ok "Docker assigns room addresses only from $AGENTRIX_ROOMS_IP_RANGE" sh -c '[ "$(docker network inspect -f "{{(index .IPAM.Config 0).IPRange}}" agx-rooms)" = "$AGENTRIX_ROOMS_IP_RANGE" ]'
expect_ok "the gateway $AGENTRIX_ROOMS_GATEWAY is on the host" sh -c "ip -4 addr show agx-r0 | grep -q 'inet $AGENTRIX_ROOMS_GATEWAY/'"
expect_ok "user agentrix-egress is 65533:65533" sh -c '[ "$(id -u agentrix-egress):$(id -g agentrix-egress)" = 65533:65533 ]'
expect_refused "agentrix-egress is in the docker group" sh -c 'id -nG agentrix-egress | tr " " "\n" | grep -qx docker'
expect_ok "setpriv is at /usr/bin/setpriv (the runner drops proxies to 65533 with it)" test -x /usr/bin/setpriv
expect_ok "the runner's work directory belongs to agentrix" sh -c '[ "$(stat -c %U /var/lib/agentrix-runner)" = agentrix ]'
expect_ok "the host holds no secrets directory" sh -c '[ ! -e /etc/agentrix/secrets ]'

echo "== the proxies' user (uid 65533)"
as_egress() { runuser -u agentrix-egress -- "$@"; }
expect_refused "uid 65533 reaches the metadata service" as_egress curl -sS -m 5 -o /dev/null http://169.254.169.254/
expect_refused "uid 65533 reaches control over the VPC" as_egress curl -sS -m 5 -o /dev/null "http://$AGENTRIX_CONTROL_VPC_IP/"
expect_refused "uid 65533 reaches a loopback service" as_egress curl -sS -m 3 -o /dev/null "http://127.0.0.1:${AGENTRIX_RUNNER_LISTEN##*:}/"
expect_ok "uid 65533 resolves names" as_egress getent hosts example.com
expect_ok "uid 65533 reaches a public site" as_egress curl -sS -m 10 -o /dev/null https://example.com/

echo "== a room (busybox on agx-rooms, gVisor)"
docker pull -q busybox:1.36 >/dev/null
room() { docker run --rm --network agx-rooms --runtime runsc --read-only --user 65534:65534 busybox:1.36 "$@"; }
expect_refused "a room resolves names (DNS side channel)" room nslookup example.com
expect_refused "a room reaches the internet directly" room wget -q -T 5 -O /dev/null http://1.1.1.1/
expect_refused "a room reaches the metadata service" room wget -q -T 5 -O /dev/null http://169.254.169.254/
expect_refused "a room reaches control over the VPC" room wget -q -T 5 -O /dev/null "http://$AGENTRIX_CONTROL_VPC_IP:${AGENTRIX_RUNNER_LISTEN##*:}/"
expect_refused "a room reaches the gateway's SSH" room nc -z -w 3 "$AGENTRIX_ROOMS_GATEWAY" 22
expect_refused "a room reaches the runner on the gateway" room nc -z -w 3 "$AGENTRIX_ROOMS_GATEWAY" "${AGENTRIX_RUNNER_LISTEN##*:}"

echo "== a proxy, started the way the runner starts it"
# A transient unit with the runner unit's user and capabilities runs setpriv exactly as the
# runner does, then reads the proxy's /proc status and stops it (which needs CAP_KILL, since
# the proxy runs as another user). The last proxy port is used, and a peer no room has.
if [ -x /opt/agentrix/bin/agentrix-egress ]; then
  caps="CAP_SETUID CAP_SETGID CAP_CHOWN CAP_DAC_OVERRIDE CAP_KILL"
  listen="$AGENTRIX_ROOMS_GATEWAY:${AGENTRIX_EGRESS_PORTS##*-}"
  status=$(systemd-run --quiet --wait --pipe --collect \
    -p User=agentrix -p Group=agentrix -p SupplementaryGroups=docker \
    -p "AmbientCapabilities=$caps" -p "CapabilityBoundingSet=$caps" -p NoNewPrivileges=yes \
    /bin/sh -c "/usr/bin/setpriv --reuid=65533 --regid=65533 --clear-groups --inh-caps=-all --ambient-caps=-all --no-new-privs -- /opt/agentrix/bin/agentrix-egress --job smoke --listen $listen --peer 172.30.0.254 --allow example.com >/dev/null 2>&1 & p=\$!; sleep 1; cat /proc/\$p/status; kill \$p && echo KILLED_OK" 2>&1)
  field() { printf '%s\n' "$status" | awk -v k="$1:" '$1 == k { $1 = ""; sub(/^ /, ""); print }'; }
  expect_ok "the proxy's uids are all 65533" sh -c "[ '$(field Uid)' = '65533 65533 65533 65533' ]"
  expect_ok "the proxy's gids are all 65533" sh -c "[ '$(field Gid)' = '65533 65533 65533 65533' ]"
  expect_ok "the proxy has no supplementary groups" sh -c "[ -z '$(field Groups)' ]"
  for set in CapInh CapPrm CapEff CapAmb; do
    expect_ok "the proxy's $set is empty" sh -c "[ '$(field "$set")' = 0000000000000000 ]"
  done
  expect_ok "the proxy has no_new_privs" sh -c "[ '$(field NoNewPrivs)' = 1 ]"
  expect_ok "the runner's user can stop the proxy (CAP_KILL)" sh -c "printf '%s' '$status' | grep -q KILLED_OK"
else
  fail "agentrix-egress is installed at /opt/agentrix/bin (deploy.sh puts it there)"
fi

echo "== $fails failed"
exit "$fails"
