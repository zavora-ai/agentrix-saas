#!/usr/bin/env bash
# Offline checks of provision.sh's refusal rules: fixtures stand in for the Vultr API (nothing
# is called). Run: deploy/saas/tests/provision-checks.sh
# shellcheck disable=SC2034 # the fixtures are globals provision.sh's check() reads
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
ssh-keygen -q -t ed25519 -N '' -C test -f "$tmp/key"
export SAAS_PROVISION_LIB=1 SAAS_SSH_KEY="$tmp/key" SAAS_ENV_FILE=/dev/null SAAS_DEPLOY_STATE="$tmp/state"
# shellcheck source=/dev/null
. "$here/provision.sh"
set +e

REGION=atl ADMIN=203.0.113.7/32
pub=$(cut -d' ' -f1-2 < "$tmp/key.pub")
ours_vm='{"id":"i1","label":"saas-node","region":"atl","plan":"vc2-4c-8gb","main_ip":"198.51.100.9","status":"active","tags":["agentrix-saas"]}'
agx='{"id":"i0","label":"agx-control","region":"atl","plan":"vhp-4c-8gb-intel","main_ip":"45.76.253.35","status":"active","tags":["agentrix"]}'
wild='{"id":"r0","name":"*","type":"A","data":"45.76.253.35","ttl":300}'
rec() { printf '{"id":"r-%s","name":"%s","type":"%s","data":"%s","ttl":300}' "$1" "$1" "$2" "$3"; }
good_rules='[{"ip_type":"v4","protocol":"tcp","port":"22","subnet":"203.0.113.7","subnet_size":32},{"ip_type":"v4","protocol":"tcp","port":"80","subnet":"0.0.0.0","subnet_size":0},{"ip_type":"v4","protocol":"tcp","port":"443","subnet":"0.0.0.0","subnet_size":0}]'
RULES=$good_rules
api_all() { printf '%s' "$RULES"; }   # only check() reads rules through it here
pass=0 fail=0
expect() { # NAME WANT(0 ok / 1 refused) [ACTIONS]
  local got; check >"$tmp/out" 2>&1; got=$?
  if [ "$got" = "$2" ] && [ "${ACTIONS[*]:-}" = "${3:-}" ]; then pass=$((pass+1)); printf 'ok    %s\n' "$1"
  else fail=$((fail+1)); printf 'FAIL  %s: exit %s (want %s), actions [%s] (want [%s])\n' "$1" "$got" "$2" "${ACTIONS[*]:-}" "${3:-}"; sed 's/^/      /' "$tmp/out"; fi
}

INSTANCES="[$agx]" RECORDS="[$wild]" FIREWALLS='[]' SSHKEYS='[]'
expect "fresh account: adds all seven" 0 "instance record:money record:kilele.money record:pay record:books firewall sshkey"

RECORDS="[$wild,$(rec money A 45.76.253.35)]"
expect "money already has an A record elsewhere: refused" 1 "instance record:kilele.money record:pay record:books firewall sshkey"

RECORDS="[$wild,$(rec pay CNAME elsewhere.example.)]"
expect "pay is a CNAME: refused" 1 "instance record:money record:kilele.money record:books firewall sshkey"

INSTANCES="[$agx,${ours_vm/agentrix-saas/someone-else}]" RECORDS="[$wild]"
expect "a saas-node VM without our tag: refused" 1 "record:money record:kilele.money record:pay record:books firewall sshkey"

INSTANCES="[$agx,${ours_vm/vc2-4c-8gb/vc2-1c-1gb}]"
expect "our saas-node on another plan: refused" 1 "record:money record:kilele.money record:pay record:books firewall sshkey"

INSTANCES="[$agx,$ours_vm]" FIREWALLS='[{"id":"f1","description":"saas-fw"}]' RULES=${good_rules/203.0.113.7/198.51.100.1}
expect "saas-fw lets SSH in from another address: refused" 1 "record:money record:kilele.money record:pay record:books sshkey"

RULES='[{"ip_type":"v4","protocol":"tcp","port":"3306","subnet":"0.0.0.0","subnet_size":0}]'
expect "saas-fw with other rules: refused" 1 "record:money record:kilele.money record:pay record:books sshkey"

RULES=$good_rules SSHKEYS='[{"id":"k1","name":"saas-admin","ssh_key":"ssh-ed25519 AAAAsomeoneelse"}]'
expect "saas-admin is another key: refused" 1 "record:money record:kilele.money record:pay record:books"

SSHKEYS="[{\"id\":\"k1\",\"name\":\"saas-admin\",\"ssh_key\":\"$pub saas-admin\"}]"
RECORDS="[$wild,$(rec money A 198.51.100.9),$(rec kilele.money A 198.51.100.9),$(rec pay A 198.51.100.9),$(rec books A 198.51.100.9)]"
expect "everything already ours (a re-run): nothing to add" 0 ""

RECORDS="[$wild,$(rec money A 198.51.100.9),$(rec money AAAA 2001:db8::1),$(rec kilele.money A 198.51.100.9),$(rec pay A 198.51.100.9),$(rec books A 198.51.100.9)]"
expect "money also has an AAAA: refused" 1 ""

INSTANCES="[$agx]" RECORDS="[$wild]" SSHKEYS='[]' FIREWALLS='[{"id":"f1","description":"saas-fw"}]' RULES='[]'
mkdir -p "$tmp/state"; printf 'FIREWALL_ID=f1\n' > "$tmp/state/inventory.env"
expect "our own saas-fw (state id) left without rules: apply adds them" 0 "instance record:money record:kilele.money record:pay record:books rules sshkey"

printf 'FIREWALL_ID=someone-elses\n' > "$tmp/state/inventory.env"
expect "an empty saas-fw that the state file doesn't name: refused" 1 "instance record:money record:kilele.money record:pay record:books sshkey"

# zone_changes: spec v3's repoint of the apex and the wildcard.
zone_expect() { # NAME WANT (the ops, "op type name live" joined by ";")
  local got; got=$(zone_changes 198.51.100.9 | jq -rs 'map("\(.op) \(.type) \(.name) \(.live)") | join(";")')
  if [ "$got" = "$2" ]; then pass=$((pass+1)); printf 'ok    %s\n' "$1"
  else fail=$((fail+1)); printf 'FAIL  %s: [%s] (want [%s])\n' "$1" "$got" "$2"; fi
}
stale4='{"id":"z1","name":"","type":"A","data":"45.76.253.35"}'
stale6='{"id":"z2","name":"*","type":"AAAA","data":"2001:19f0:5401:1d3f:5400:07ff:fe06:7e5f"}'
mx='{"id":"z3","name":"","type":"MX","data":"mail.example."}'
INSTANCES="[$ours_vm]" RECORDS="[$stale4,${wild/45.76.253.35/45.76.253.35},$stale6,$mx,$(rec fixture A 64.177.42.234),$(rec money A 198.51.100.9)]"
zone_expect "stale apex and wildcard: point A, drop AAAA and fixture; MX and money stay" "point A @ false;point A * false;delete AAAA * false;delete A fixture false"
INSTANCES="[$ours_vm,$agx]"
zone_expect "the old address belongs to a live VM again: refused" "point A @ true;point A * true;delete AAAA * false;delete A fixture false"
INSTANCES="[$ours_vm,{\"label\":\"x\",\"main_ip\":\"203.0.113.1\",\"v6_main_ip\":\"2001:19f0:5401:1d3f:5400:7ff:fe06:7e5f\"}]"
zone_expect "a live VM's IPv6 written with leading zeros still counts as live" "point A @ false;point A * false;delete AAAA * true;delete A fixture false"
INSTANCES="[$ours_vm]" RECORDS="[$(rec "*" A 198.51.100.9),$mx,$(rec money A 198.51.100.9)]"
zone_expect "already on saas-node: nothing to do" ""
POINT_NAMES=("" "*" paypal "*.paypal")
RECORDS="[$(rec paypal A 45.76.253.35),$(rec "*.paypal" AAAA 2001:db8::7),$(rec ops.paypal A 45.76.253.35)]"
zone_expect "the platform host and its wildcard follow; other names under it stay" "point A paypal false;delete AAAA *.paypal false"

printf '%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
