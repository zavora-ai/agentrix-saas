#!/usr/bin/env bash
# Agentrix SaaS: this is the WebAgent Platform's three-VM (agx-*) deploy, kept only for reference.
# It must never run from this repository: it would reach the Vultr entry's servers and domain.
echo "$(basename "$0"): disabled in agentrix-saas (see specs/saas/README.md for this entry's deploy)" >&2; exit 2
# Agentrix · provision the Vultr substrate (task S0-04; design §2; R14.1–R14.2).
#
#   deploy/provision.sh [plan]         Dry run: prices, what would be created, rendered
#                                      cloud-init. Needs no key and creates nothing.
#   deploy/provision.sh apply --yes    Creates (or reuses) the VPC, firewall groups, SSH key, the
#                                      three VMs and DNS records. PAID: only after James says yes.
#   deploy/provision.sh status         Lists what exists, with addresses and each host's state.
#   deploy/provision.sh render-file F  Prints template F with this deployment's values filled in
#                                      (deploy.sh uses it for the Caddyfile).
#   deploy/provision.sh ssh-close --yes   Removes the admin SSH rule from control's firewall group.
#   deploy/provision.sh destroy --yes  Deletes the three VMs, the firewall groups and the VPC.
#
# Costs, and sleeping the VMs nobody needs (specs/platform/cost.md). Without --yes each one is
# a dry run that prints what it would do and what it would cost or save:
#   deploy/provision.sh costs                    Read only: this month's charges, the hourly run
#                                                rate, snapshots and what sleeping would save.
#   deploy/provision.sh snapshot [ROLE…] [--yes] Snapshots VMs (default all three); changes nothing.
#   deploy/provision.sh sleep ROLE… [--yes]      sandbox and/or fixture: a fresh snapshot, then the
#                                                VM is deleted. James's yes; never before
#                                                SLEEP_NOT_BEFORE (when judging ends).
#   deploy/provision.sh wake ROLE… [--yes]       Recreates them from their latest snapshot, control
#                                                first, with DNS, inventory and SSH updated.
#   deploy/provision.sh restore-test ROLE [--yes]  Times a restore on a throwaway copy, then deletes it.
#
# Configuration: deploy/.env (see deploy/.env.example). The Vultr API key is read from there or
# from the environment, sent to the API on curl's stdin (never on a command line), and never
# printed. Everything is matched by label, so `apply` is safe to re-run.
set -euo pipefail

DEPLOY_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ENV_FILE=${AGENTRIX_DEPLOY_ENV:-$DEPLOY_DIR/.env}
STATE_DIR=${AGENTRIX_DEPLOY_STATE:-$DEPLOY_DIR/.state}   # the checkout that ran apply holds it
API=https://api.vultr.com/v2
PREFIX=${AGENTRIX_PREFIX:-agx}   # agx-b1 for the VMs made before the move to the new account (8 Nov)
TAGS='["agentrix","agentrix-platform"]'

die() { printf 'provision: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }
need curl; need jq; need base64; need sed

# --- configuration -----------------------------------------------------------------------

# Reads NAME from the environment, else from deploy/.env (KEY=value lines; quotes stripped).
# The file is parsed, never sourced.
conf() {
  local name=$1 default=${2:-} value=""
  if [ -n "${!name:-}" ]; then printf '%s' "${!name}"; return; fi
  if [ -f "$ENV_FILE" ]; then
    value=$(grep -E "^${name}=" "$ENV_FILE" | tail -n 1 | cut -d= -f2- || true)
    value=${value%$'\r'}; value=${value#\"}; value=${value%\"}; value=${value#\'}; value=${value%\'}
  fi
  printf '%s' "${value:-$default}"
}

REGION=$(conf VULTR_REGION atl)
OS_ID=$(conf VULTR_OS_ID 2284)                       # Ubuntu 24.04 LTS x64
CONTROL_PLAN=$(conf CONTROL_PLAN vhp-4c-8gb-intel)
SANDBOX_PLAN=$(conf SANDBOX_PLAN vx1-g-4c-16g-240s)
FIXTURE_PLAN=$(conf FIXTURE_PLAN vc2-1c-2gb)
VPC_SUBNET=$(conf VPC_SUBNET 10.42.0.0)
VPC_SUBNET_SIZE=$(conf VPC_SUBNET_SIZE 24)
ADMIN_CIDR=$(conf ADMIN_CIDR "")
SSH_PUBLIC_KEY_FILE=$(conf SSH_PUBLIC_KEY_FILE "$HOME/.ssh/id_ed25519.pub")
SSH_PUBLIC_KEY_FILE=${SSH_PUBLIC_KEY_FILE/#\~/$HOME}
PLATFORM_ZONE=$(conf PLATFORM_ZONE "")
PLATFORM_NAME=$(conf PLATFORM_NAME "")
FIXTURE_ZONE=$(conf FIXTURE_ZONE "")
FIXTURE_NAME=$(conf FIXTURE_NAME "")
NODE_PORT=$(conf NODE_PORT 8080)
MODELS_PORT=$(conf MODELS_PORT 8090)
INTERNAL_PORT=$(conf INTERNAL_PORT 8079)               # node's loopback-only internal listener
RUNNER_PORT=$(conf RUNNER_PORT 7443)
EGRESS_PORTS=$(conf EGRESS_PORTS 3128-3191)
ROOMS_SUBNET=$(conf ROOMS_SUBNET 172.30.0.0/24)
ROOMS_GATEWAY=$(conf ROOMS_GATEWAY 172.30.0.1)
ROOMS_IP_RANGE=$(conf ROOMS_IP_RANGE 172.30.0.128/25)
FIXTURE_UPSTREAM=$(conf FIXTURE_UPSTREAM 127.0.0.1:8081)

fqdn() { local zone=$1 name=$2; [ -z "$zone" ] && return 0; if [ -n "$name" ]; then printf '%s.%s' "$name" "$zone"; else printf '%s' "$zone"; fi; }
PLATFORM_FQDN=$(fqdn "$PLATFORM_ZONE" "$PLATFORM_NAME")
FIXTURE_FQDN=$(fqdn "$FIXTURE_ZONE" "$FIXTURE_NAME")

# --- the Vultr API -----------------------------------------------------------------------

key() {
  local k; k=$(conf VULTR_API_KEY "")
  [ -n "$k" ] || die "VULTR_API_KEY is not set in $ENV_FILE or the environment (task S0-02)"
  printf '%s' "$k"
}

# api METHOD PATH [JSON]. The Authorization header goes to curl through --config on stdin.
api() {
  local method=$1 path=$2 body=${3:-}
  local args=(-sS --fail-with-body --max-time 60 -X "$method" "$API$path" -H 'Content-Type: application/json')
  [ -n "$body" ] && args+=(--data-binary "$body")
  printf 'header = "Authorization: Bearer %s"\n' "$(key)" | curl --config - "${args[@]}"
}

# Public endpoints need no key.
public() { curl -sS --fail-with-body --max-time 60 "$API$1"; }

# --- rendering ---------------------------------------------------------------------------

b64() { base64 < "$1" | tr -d '\n'; }

# Shows a path with $HOME as ~.
tilde() { case $1 in "$HOME"*) printf '~%s' "${1#"$HOME"}";; *) printf '%s' "$1";; esac; }

# render FILE CONTROL_VPC_IP: fills the @@TOKEN@@ placeholders.
render() {
  local file=$1 control_ip=$2 platform_site=${PLATFORM_FQDN:-:80} fixture_site=${FIXTURE_FQDN:-:80}
  sed -e "s|@@CONTROL_VPC_IP@@|$control_ip|g" \
      -e "s|@@VPC_SUBNET@@|$VPC_SUBNET/$VPC_SUBNET_SIZE|g" \
      -e "s|@@RUNNER_PORT@@|$RUNNER_PORT|g" \
      -e "s|@@EGRESS_PORTS@@|$EGRESS_PORTS|g" \
      -e "s|@@ROOMS_SUBNET@@|$ROOMS_SUBNET|g" \
      -e "s|@@ROOMS_GATEWAY@@|$ROOMS_GATEWAY|g" \
      -e "s|@@ROOMS_IP_RANGE@@|$ROOMS_IP_RANGE|g" \
      -e "s|@@NODE_PORT@@|$NODE_PORT|g" \
      -e "s|@@MODELS_PORT@@|$MODELS_PORT|g" \
      -e "s|@@INTERNAL_PORT@@|$INTERNAL_PORT|g" \
      -e "s|@@EGRESS_PORT_RANGE@@|${EGRESS_PORTS/-/:}|g" \
      -e "s|@@PLATFORM_FQDN@@|$PLATFORM_FQDN|g" \
      -e "s|@@PLATFORM_SITE@@|$platform_site|g" \
      -e "s|@@FIXTURE_SITE@@|$fixture_site|g" \
      -e "s|@@FIXTURE_UPSTREAM@@|$FIXTURE_UPSTREAM|g" \
      "$file"
}

# user_data ROLE CONTROL_VPC_IP: writes the rendered cloud-init to .state/rendered/ROLE.yaml.
user_data() {
  local role=$1 control_ip=$2 out=$STATE_DIR/rendered tmp
  mkdir -p "$out/files"
  render "$DEPLOY_DIR/nftables/$role.nft" "$control_ip" > "$out/files/$role.nft"
  local nft_b64; nft_b64=$(b64 "$out/files/$role.nft")
  local caddy_b64="" docker_b64=""
  if [ -f "$DEPLOY_DIR/caddy/$role.Caddyfile" ]; then
    render "$DEPLOY_DIR/caddy/$role.Caddyfile" "$control_ip" > "$out/files/$role.Caddyfile"
    caddy_b64=$(b64 "$out/files/$role.Caddyfile")
  fi
  [ "$role" = sandbox ] && docker_b64=$(b64 "$DEPLOY_DIR/docker/sandbox-daemon.json")
  tmp=$(render "$DEPLOY_DIR/cloud-init/$role.yaml" "$control_ip")
  tmp=${tmp//@@NFT_B64@@/$nft_b64}
  tmp=${tmp//@@CADDY_B64@@/$caddy_b64}
  tmp=${tmp//@@DOCKER_DAEMON_B64@@/$docker_b64}
  printf '%s\n' "$tmp" > "$out/$role.yaml"
  if grep -q '@@[A-Z_0-9]*@@' "$out/$role.yaml" "$out/files/$role.nft"; then
    die "unfilled token in $role: $(grep -ho '@@[A-Z_0-9]*@@' "$out/$role.yaml" "$out/files/$role.nft" | sort -u | tr '\n' ' ')"
  fi
}

check_yaml() {
  command -v python3 >/dev/null 2>&1 || return 0
  python3 -c 'import sys, yaml
for p in sys.argv[1:]:
    with open(p) as f:
        first = f.readline().strip()
        f.seek(0)
        d = yaml.safe_load(f)
    assert first == "#cloud-config", p + ": first line must be #cloud-config"
    assert isinstance(d, dict) and d.get("runcmd") and d.get("write_files"), p
' "$@" || die "rendered cloud-init is not valid YAML: $*"
}

# --- plan --------------------------------------------------------------------------------

cmd_plan() {
  say "Agentrix · provisioning plan (dry run: nothing is created)"
  say ""
  local avail plans
  avail=$(public "/regions/$REGION/availability") || die "region $REGION not found"
  plans=$(public "/plans?per_page=500")
  local total_m=0 total_h=0 role plan row ok
  say "Region: $REGION · OS $OS_ID (Ubuntu 24.04 LTS)"
  for role in control sandbox fixture; do
    case $role in control) plan=$CONTROL_PLAN;; sandbox) plan=$SANDBOX_PLAN;; fixture) plan=$FIXTURE_PLAN;; esac
    row=$(jq -r --arg p "$plan" '.plans[] | select(.id==$p) | "\(.vcpu_count) vCPU · \(.ram/1024) GB RAM · \(.disk) GB disk · $\(.monthly_cost)/mo · $\(.hourly_cost)/h"' <<<"$plans")
    [ -n "$row" ] || die "plan $plan does not exist"
    ok=$(jq -r --arg p "$plan" 'if (.available_plans | index($p)) then "available" else "NOT AVAILABLE" end' <<<"$avail")
    printf '  %-8s %-20s %s · %s\n' "$role" "$plan" "$row" "$ok"
    total_m=$(jq -n --argjson a "$total_m" --argjson b "$(jq --arg p "$plan" '.plans[] | select(.id==$p) | .monthly_cost' <<<"$plans")" '$a+$b')
    total_h=$(jq -n --argjson a "$total_h" --argjson b "$(jq --arg p "$plan" '.plans[] | select(.id==$p) | .hourly_cost' <<<"$plans")" '$a+$b')
  done
  printf '  Total: $%.2f a month at most; $%.3f an hour (Vultr bills hourly up to the monthly cap)\n' "$total_m" "$total_h"
  say ""
  say "Also created, at no charge: VPC $VPC_SUBNET/$VPC_SUBNET_SIZE ($PREFIX); firewall groups $PREFIX-{control,sandbox,fixture}; SSH key $PREFIX-admin; DNS A/AAAA records."
  say "Public firewall (Vultr groups; VPC traffic is limited by each host's nftables instead):"
  say "  control  tcp 80, 443 from anywhere; tcp 22 from ADMIN_CIDR=${ADMIN_CIDR:-<not set>} only"
  say "  sandbox  nothing inbound (the runner on :$RUNNER_PORT and SSH are reachable from control's VPC address only)"
  say "  fixture  tcp 80, 443 from anywhere (SSH from control's VPC address only)"
  say "Rooms: internal network agx-rooms (bridge agx-r0, $ROOMS_SUBNET; Docker assigns from $ROOMS_IP_RANGE); egress proxies on $ROOMS_GATEWAY:$EGRESS_PORTS as uid 65533; no forwarding."
  say "Domains: platform=${PLATFORM_FQDN:-<not chosen, S0-03>} · fixture=${FIXTURE_FQDN:-<not chosen, S0-03>}"
  local keystate="missing (S0-02)"; [ -n "$(conf VULTR_API_KEY "")" ] && keystate="present (not shown)"
  say "Vultr API key in $(tilde "$ENV_FILE"): $keystate"
  if [ -f "$SSH_PUBLIC_KEY_FILE" ]; then say "SSH public key: $(tilde "$SSH_PUBLIC_KEY_FILE")"; else say "SSH public key: $(tilde "$SSH_PUBLIC_KEY_FILE") NOT FOUND (set SSH_PUBLIC_KEY_FILE)"; fi
  say ""
  user_data control "CONTROL_VPC_IP_AT_APPLY"
  user_data sandbox "203.0.113.10"
  user_data fixture "203.0.113.10"
  check_yaml "$STATE_DIR"/rendered/{control,sandbox,fixture}.yaml
  say "Rendered cloud-init (valid YAML): $(tilde "$STATE_DIR")/rendered/ (sandbox and fixture use the documentation address 203.0.113.10 for control until apply)"
  say ""
  say "To create these, James approves, then: deploy/provision.sh apply --yes"
}

# --- apply -------------------------------------------------------------------------------

ensure_ssh_key() {
  [ -f "$SSH_PUBLIC_KEY_FILE" ] || die "no SSH public key at $SSH_PUBLIC_KEY_FILE"
  local pub id; pub=$(cut -d' ' -f1-2 < "$SSH_PUBLIC_KEY_FILE")
  id=$(api GET "/ssh-keys?per_page=500" | jq -r --arg k "$pub" '.ssh_keys[] | select((.ssh_key|split(" ")[0:2]|join(" "))==$k) | .id' | head -n 1)
  if [ -z "$id" ]; then
    id=$(api POST /ssh-keys "$(jq -n --arg n "$PREFIX-admin" --arg k "$pub" '{name:$n, ssh_key:$k}')" | jq -r '.ssh_key.id')
    say "  ssh key      created $id" >&2
  else
    say "  ssh key      reusing $id" >&2
  fi
  printf '%s' "$id"
}

ensure_vpc() {
  local id
  id=$(api GET "/vpcs?per_page=500" | jq -r --arg d "$PREFIX" --arg r "$REGION" '.vpcs[] | select(.description==$d and .region==$r) | .id' | head -n 1)
  if [ -z "$id" ]; then
    id=$(api POST /vpcs "$(jq -n --arg r "$REGION" --arg d "$PREFIX" --arg s "$VPC_SUBNET" --argjson m "$VPC_SUBNET_SIZE" '{region:$r, description:$d, v4_subnet:$s, v4_subnet_mask:$m}')" | jq -r '.vpc.id')
    say "  vpc          created $id" >&2
  else
    say "  vpc          reusing $id" >&2
  fi
  printf '%s' "$id"
}

ensure_group() {
  local name=$PREFIX-$1 id
  id=$(api GET "/firewalls?per_page=500" | jq -r --arg d "$name" '.firewall_groups[] | select(.description==$d) | .id' | head -n 1)
  if [ -z "$id" ]; then
    id=$(api POST /firewalls "$(jq -n --arg d "$name" '{description:$d}')" | jq -r '.firewall_group.id')
    say "  firewall     created $name $id" >&2
  else
    say "  firewall     reusing $name $id" >&2
  fi
  printf '%s' "$id"
}

# ensure_rule GROUP IP_TYPE SUBNET SIZE PORT NOTE
ensure_rule() {
  local group=$1 type=$2 subnet=$3 size=$4 port=$5 note=$6 exists
  exists=$(api GET "/firewalls/$group/rules?per_page=500" | jq -r --arg t "$type" --arg s "$subnet" --argjson z "$size" --arg p "$port" \
    '[.firewall_rules[] | select(.ip_type==$t and .protocol=="tcp" and .subnet==$s and .subnet_size==$z and .port==$p)] | length')
  if [ "$exists" = 0 ]; then
    api POST "/firewalls/$group/rules" "$(jq -n --arg t "$type" --arg s "$subnet" --argjson z "$size" --arg p "$port" --arg n "$note" \
      '{ip_type:$t, protocol:"tcp", subnet:$s, subnet_size:$z, port:$p, notes:$n}')" >/dev/null
    say "    rule       $type tcp $port from $subnet/$size" >&2
  fi
}

instance_id() { api GET "/instances?label=$PREFIX-$1&per_page=100" | jq -r '.instances[0].id // empty'; }

# create_instance ROLE PLAN FIREWALL VPC SSHKEY
create_instance() {
  local role=$1 plan=$2 fw=$3 vpc=$4 sshkey=$5 id body
  id=$(instance_id "$role")
  if [ -n "$id" ]; then say "  instance     reusing $PREFIX-$role $id" >&2; printf '%s' "$id"; return; fi
  body=$(jq -n --arg r "$REGION" --arg p "$plan" --argjson o "$OS_ID" --arg l "$PREFIX-$role" --arg h "$PREFIX-$role" \
    --arg fw "$fw" --arg vpc "$vpc" --arg k "$sshkey" --arg ud "$(b64 "$STATE_DIR/rendered/$role.yaml")" --argjson tags "$TAGS" \
    '{region:$r, plan:$p, os_id:$o, label:$l, hostname:$h, firewall_group_id:$fw, attach_vpc:[$vpc], sshkey_id:[$k],
      user_data:$ud, enable_ipv6:true, backups:"disabled", activation_email:false, tags:$tags}')
  id=$(api POST /instances "$body" | jq -r '.instance.id')
  say "  instance     created $PREFIX-$role $id ($plan)" >&2
  printf '%s' "$id"
}

wait_active() {
  local id=$1 status=""
  for _ in $(seq 1 90); do
    status=$(api GET "/instances/$id" | jq -r '.instance.status')
    [ "$status" = active ] && return 0
    sleep 10
  done
  die "instance $id is not active after 15 minutes (status: $status)"
}

vpc_ip() {
  local id=$1 vpc=$2 ip
  for _ in $(seq 1 30); do
    ip=$(api GET "/instances/$id/vpcs" | jq -r --arg v "$vpc" '.vpcs[] | select(.id==$v) | .ip_address // empty')
    [ -n "$ip" ] && { printf '%s' "$ip"; return 0; }
    sleep 5
  done
  die "instance $id has no address on VPC $vpc"
}

main_ip() { api GET "/instances/$1" | jq -r ".instance.$2 // empty"; }

# dns_upsert ZONE NAME TYPE VALUE
dns_upsert() {
  local zone=$1 name=$2 type=$3 value=$4 rec
  [ -n "$zone" ] && [ -n "$value" ] || return 0
  ensure_zone "$zone"
  rec=$(api GET "/domains/$zone/records?per_page=500" | jq -r --arg n "$name" --arg t "$type" '.records[] | select(.name==$n and .type==$t) | .id' | head -n 1)
  if [ -z "$rec" ]; then
    api POST "/domains/$zone/records" "$(jq -n --arg n "$name" --arg t "$type" --arg d "$value" '{name:$n, type:$t, data:$d, ttl:300}')" >/dev/null
  else
    api PATCH "/domains/$zone/records/$rec" "$(jq -n --arg d "$value" '{data:$d, ttl:300}')" >/dev/null
  fi
  say "  dns          $type ${name:+$name.}$zone -> $value" >&2
}

# ensure_zone ZONE: creates the Vultr DNS zone if it is missing (free). The registrar must
# delegate it to ns1.vultr.com and ns2.vultr.com before the records resolve.
ensure_zone() {
  local zone=$1
  api GET "/domains/$zone" >/dev/null 2>&1 && return 0
  api POST /domains "$(jq -n --arg d "$zone" '{domain:$d}')" >/dev/null
  say "  dns          created zone $zone (delegate it to ns1.vultr.com and ns2.vultr.com)" >&2
}

cmd_apply() {
  [ "${1:-}" = --yes ] || die "apply creates paid Vultr resources. Run the plan, get James's yes, then: deploy/provision.sh apply --yes"
  [ -n "$ADMIN_CIDR" ] || die "set ADMIN_CIDR in $ENV_FILE (your public IPv4 as a.b.c.d/32): control accepts SSH only from it"
  case $ADMIN_CIDR in */*) ;; *) die "ADMIN_CIDR must be a CIDR such as 203.0.113.7/32";; esac
  key >/dev/null
  mkdir -p "$STATE_DIR"
  say "Applying in $REGION"
  local sshkey vpc fw_c fw_s fw_f
  sshkey=$(ensure_ssh_key)
  vpc=$(ensure_vpc)
  fw_c=$(ensure_group control); fw_s=$(ensure_group sandbox); fw_f=$(ensure_group fixture)
  ensure_rule "$fw_c" v4 0.0.0.0 0 80 "http"
  ensure_rule "$fw_c" v4 0.0.0.0 0 443 "https"
  ensure_rule "$fw_c" v6 :: 0 80 "http"
  ensure_rule "$fw_c" v6 :: 0 443 "https"
  ensure_rule "$fw_c" v4 "${ADMIN_CIDR%/*}" "${ADMIN_CIDR#*/}" 22 "admin ssh (deploy/README.md)"
  ensure_rule "$fw_f" v4 0.0.0.0 0 80 "http"
  ensure_rule "$fw_f" v4 0.0.0.0 0 443 "https"
  ensure_rule "$fw_f" v6 :: 0 80 "http"
  ensure_rule "$fw_f" v6 :: 0 443 "https"
  : "$fw_s" # the sandbox group has no rules: nothing inbound on the public interface

  # Control first: the other two hosts' firewalls name its VPC address.
  user_data control "CONTROL_IS_THIS_HOST"
  check_yaml "$STATE_DIR/rendered/control.yaml"
  local c s f c_vpc
  c=$(create_instance control "$CONTROL_PLAN" "$fw_c" "$vpc" "$sshkey")
  wait_active "$c"
  c_vpc=$(vpc_ip "$c" "$vpc")
  user_data sandbox "$c_vpc"; user_data fixture "$c_vpc"
  check_yaml "$STATE_DIR"/rendered/{sandbox,fixture}.yaml
  s=$(create_instance sandbox "$SANDBOX_PLAN" "$fw_s" "$vpc" "$sshkey")
  f=$(create_instance fixture "$FIXTURE_PLAN" "$fw_f" "$vpc" "$sshkey")
  wait_active "$s"; wait_active "$f"

  local c_ip c_ip6 s_vpc f_ip f_ip6 f_vpc
  c_ip=$(main_ip "$c" main_ip); c_ip6=$(main_ip "$c" v6_main_ip)
  f_ip=$(main_ip "$f" main_ip); f_ip6=$(main_ip "$f" v6_main_ip)
  s_vpc=$(vpc_ip "$s" "$vpc"); f_vpc=$(vpc_ip "$f" "$vpc")
  # AGENTRIX_DNS_LATER=1 (a move to a new account while the old stack still serves): create the
  # VMs and write the inventory, but leave the records alone until `provision.sh dns --yes`.
  if [ "${AGENTRIX_DNS_LATER:-0}" = 1 ]; then
    say "  dns          left as it is (AGENTRIX_DNS_LATER=1); point it here later with: deploy/provision.sh dns --yes"
  else
    point_dns "$c_ip" "$c_ip6" "$f_ip" "$f_ip6"
  fi

  write_inventory "$vpc" "$c" "$c_ip" "$c_ip6" "$c_vpc" "$s" "$s_vpc" "$f" "$f_ip" "$f_ip6" "$f_vpc"
  say ""
  say "Done. Inventory: $(tilde "$STATE_DIR")/inventory.env · SSH: ssh -F $(tilde "$STATE_DIR")/ssh_config agx-control (or agx-sandbox, agx-fixture)"
  say "cloud-init takes a few minutes; then: deploy/provision.sh status"
}

# point_dns CONTROL_IP CONTROL_IP6 FIXTURE_IP FIXTURE_IP6: the platform's apex and wildcard to
# control, the fixture's name to the fixture.
point_dns() {
  dns_upsert "$PLATFORM_ZONE" "$PLATFORM_NAME" A "$1"
  dns_upsert "$PLATFORM_ZONE" "$PLATFORM_NAME" AAAA "$2"
  # Live sites' platform addresses ({domain-with-dashes}.<platform>) via on-demand TLS on control.
  dns_upsert "$PLATFORM_ZONE" "*${PLATFORM_NAME:+.$PLATFORM_NAME}" A "$1"
  dns_upsert "$PLATFORM_ZONE" "*${PLATFORM_NAME:+.$PLATFORM_NAME}" AAAA "$2"
  dns_upsert "$FIXTURE_ZONE" "$FIXTURE_NAME" A "$3"
  dns_upsert "$FIXTURE_ZONE" "$FIXTURE_NAME" AAAA "$4"
}

# `dns --yes`: points the records at the VMs in the inventory (after AGENTRIX_DNS_LATER=1 apply).
cmd_dns() {
  [ "${1:-}" = --yes ] || die "dns changes live records: run it with --yes"
  local f="$STATE_DIR/inventory.env"
  [ -f "$f" ] || die "no inventory at $f: run apply first"
  iv() { grep -E "^$1=" "$f" | tail -n 1 | cut -d= -f2-; }
  local c_ip c_ip6 f_ip f_ip6
  c_ip=$(iv CONTROL_IP); c_ip6=$(iv CONTROL_IP6); f_ip=$(iv FIXTURE_IP); f_ip6=$(iv FIXTURE_IP6)
  [ -n "$c_ip" ] && [ -n "$f_ip" ] || die "the inventory lacks CONTROL_IP or FIXTURE_IP"
  ensure_zone "$PLATFORM_ZONE"
  [ "$FIXTURE_ZONE" = "$PLATFORM_ZONE" ] || ensure_zone "$FIXTURE_ZONE"
  point_dns "$c_ip" "$c_ip6" "$f_ip" "$f_ip6"
}

write_inventory() {
  cat > "$STATE_DIR/inventory.env" <<EOF
# Written by deploy/provision.sh apply. No secrets.
REGION=$REGION
VPC_ID=$1
CONTROL_ID=$2
CONTROL_IP=$3
CONTROL_IP6=$4
CONTROL_VPC_IP=$5
SANDBOX_ID=$6
SANDBOX_VPC_IP=$7
FIXTURE_ID=$8
FIXTURE_IP=$9
FIXTURE_IP6=${10}
FIXTURE_VPC_IP=${11}
PLATFORM_FQDN=$PLATFORM_FQDN
FIXTURE_FQDN=$FIXTURE_FQDN
RUNNER_ADDR=$7:$RUNNER_PORT
EOF
  cat > "$STATE_DIR/ssh_config" <<EOF
# Written by deploy/provision.sh apply. Sandbox and fixture are reachable only through control.
Host agx-control
  HostName $3
  User deploy
  IdentitiesOnly yes
  IdentityFile ${SSH_PUBLIC_KEY_FILE%.pub}
Host agx-sandbox
  HostName $7
  User deploy
  ProxyJump agx-control
  IdentitiesOnly yes
  IdentityFile ${SSH_PUBLIC_KEY_FILE%.pub}
Host agx-fixture
  HostName ${11}
  User deploy
  ProxyJump agx-control
  IdentitiesOnly yes
  IdentityFile ${SSH_PUBLIC_KEY_FILE%.pub}
EOF
}

# --- status, ssh-close, destroy ------------------------------------------------------------

cmd_status() {
  key >/dev/null
  say "Instances ($PREFIX-*) in $REGION:"
  api GET "/instances?per_page=500" | jq -r --arg p "$PREFIX-" \
    '.instances[] | select(.label|startswith($p)) | "  \(.label)\t\(.status)/\(.power_status)/\(.server_status)\t\(.plan)\t\(.main_ip)\t\(.v6_main_ip)"'
  say "Firewall groups:"
  local instances groups id desc rules members
  instances=$(api GET "/instances?per_page=500")
  groups=$(api GET "/firewalls?per_page=500" | jq -r --arg p "$PREFIX-" '.firewall_groups[] | select(.description|startswith($p)) | "\(.id)\t\(.description)"')
  while IFS=$'\t' read -r id desc; do
    [ -n "$id" ] || continue
    rules=$(api GET "/firewalls/$id/rules?per_page=500" | jq -r '[.firewall_rules[] | "\(.protocol) \(.port) from \(.subnet)/\(.subnet_size)"] | if length == 0 then "no rules" else join(", ") end')
    members=$(printf '%s' "$instances" | jq -r --arg g "$id" '[.instances[] | select(.firewall_group_id==$g) | .label] | if length == 0 then "no instances" else join(", ") end')
    printf '  %s\t%s\t(%s)\n' "$desc" "$rules" "$members"
  done <<< "$groups"
  say "VPC:"
  api GET "/vpcs?per_page=500" | jq -r --arg d "$PREFIX" '.vpcs[] | select(.description==$d) | "  \(.id)\t\(.region)\t\(.v4_subnet)/\(.v4_subnet_mask)"'
  if [ -f "$STATE_DIR/ssh_config" ]; then
    say "Host facts (over SSH):"
    for h in agx-control agx-sandbox agx-fixture; do
      printf '  %-12s ' "$h"
      ssh -F "$STATE_DIR/ssh_config" -o BatchMode=yes -o ConnectTimeout=8 "$h" \
        'cloud-init status 2>/dev/null | tail -n 1; sudo -n cat /var/lib/agentrix/host-facts.json 2>/dev/null || echo "(no host facts)"; d=$(cat /opt/agentrix/bin/DEPLOYED 2>/dev/null) && echo "deployed $d"; true' 2>/dev/null | tr '\n' ' ' || printf 'unreachable'
      printf '\n'
    done
  fi
}

cmd_ssh_close() {
  [ "${1:-}" = --yes ] || die "ssh-close removes control's admin SSH rule; run with --yes"
  local fw ids
  fw=$(api GET "/firewalls?per_page=500" | jq -r --arg d "$PREFIX-control" '.firewall_groups[] | select(.description==$d) | .id' | head -n 1)
  [ -n "$fw" ] || die "no $PREFIX-control firewall group"
  ids=$(api GET "/firewalls/$fw/rules?per_page=500" | jq -r '.firewall_rules[] | select(.port=="22") | .id')
  for r in $ids; do api DELETE "/firewalls/$fw/rules/$r" >/dev/null; say "  removed rule $r (tcp 22)"; done
}

cmd_destroy() {
  [ "${1:-}" = --yes ] || die "destroy deletes the three VMs and their data; only with James's yes: deploy/provision.sh destroy --yes"
  local role id fw vpc
  for role in sandbox fixture control; do
    id=$(instance_id "$role")
    [ -n "$id" ] && { api DELETE "/instances/$id" >/dev/null; say "  deleted instance $PREFIX-$role $id"; }
  done
  sleep 20
  for role in control sandbox fixture; do
    fw=$(api GET "/firewalls?per_page=500" | jq -r --arg d "$PREFIX-$role" '.firewall_groups[] | select(.description==$d) | .id' | head -n 1)
    [ -n "$fw" ] || continue
    if api DELETE "/firewalls/$fw" >/dev/null; then say "  deleted firewall group $PREFIX-$role"; else say "  firewall group $PREFIX-$role still in use; re-run destroy"; fi
  done
  vpc=$(api GET "/vpcs?per_page=500" | jq -r --arg d "$PREFIX" --arg r "$REGION" '.vpcs[] | select(.description==$d and .region==$r) | .id' | head -n 1)
  if [ -n "$vpc" ]; then
    if api DELETE "/vpcs/$vpc" >/dev/null; then say "  deleted VPC $vpc"; else say "  VPC still attached; re-run destroy"; fi
  fi
  say "DNS records and the SSH key are kept."
}

# --- costs, snapshot, sleep, wake (specs/platform/cost.md) ---------------------------------
#
# A sleeping VM is a snapshot and no instance. Vultr bills a stopped instance in full, so only
# deleting it saves money. Only the sandbox and the fixture sleep: control serves
# agentrix.website, holds control.db and the secrets, and the other two hosts' firewalls name
# its VPC address, so it stays up (destroy and apply are the way to switch everything off).
# Snapshots keep what a rebuild would lose: the room images and their IDs, which node.env pins,
# the fixture's WordPress exactly as the gauntlet knows it, the host keys, and the runner's copy
# of control's job key.

SNAPSHOT_USD_GB_MONTH=0.05       # Vultr bills a snapshot's compressed size at $0.05/GB a month
SLEEP_NOT_BEFORE=$(conf SLEEP_NOT_BEFORE "")
SSH_OPTS=(-F "$STATE_DIR/ssh_config" -o BatchMode=yes -o ConnectTimeout=10)
PENDING="" PLANS=""

calc() { awk "BEGIN { printf \"%.${2:-2}f\", $1 }"; }
role_plan() { case $1 in control) printf '%s' "$CONTROL_PLAN";; sandbox) printf '%s' "$SANDBOX_PLAN";; fixture) printf '%s' "$FIXTURE_PLAN";; esac; }
load_prices() {
  [ -n "$PENDING" ] || PENDING=$(api GET /billing/pending-charges)
  [ -n "$PLANS" ] || PLANS=$(public "/plans?per_page=500")
}

# rate ROLE: dollars an hour Vultr bills this VM: the unit price on this month's pending charges
# (control and fixture bill monthly/672 an hour until the cap), else the plan's list price.
rate() {
  local r
  r=$(jq -r --arg l "[$PREFIX-$1]" '[.pending_charges[] | select(.description|endswith($l)) | .unit_price] | last // empty' <<<"$PENDING")
  [ -n "$r" ] || r=$(jq -r --arg p "$(role_plan "$1")" '.plans[] | select(.id==$p) | .hourly_cost' <<<"$PLANS")
  printf '%s' "$r"
}

# used_gb ROLE: disk in use, the upper bound of its snapshot's billed size. Empty if unreachable.
used_gb() {
  [ -f "$STATE_DIR/ssh_config" ] || return 0
  ssh "${SSH_OPTS[@]}" "agx-$1" "df -BG --output=used / | tail -n 1" 2>/dev/null | tr -dc '0-9' || true
}

# snap_cost GB: the most a snapshot of that much used disk costs a month, in words.
snap_cost() { if [ -n "$1" ]; then printf '%s GB in use, so at most $%s/month' "$1" "$(calc "$1 * $SNAPSHOT_USD_GB_MONTH")"; else printf 'size unknown without SSH ($%s per GB a month)' "$SNAPSHOT_USD_GB_MONTH"; fi; }

# snapshots_of ROLE: "id<TAB>date<TAB>compressed bytes<TAB>status" of the role's snapshots, newest
# last. A description is "agx-b1-ROLE <stamp>" (this script) or "agx-b1-ROLE-<anything>" (console),
# leading spaces ignored.
snapshots_of() {
  api GET "/snapshots?per_page=500" | jq -r --arg re "^$PREFIX-$1([ -]|$)" \
    '[.snapshots[] | select(.description|gsub("^\\s+"; "")|test($re))] | sort_by(.date_created) | .[] | "\(.id)\t\(.date_created)\t\(.compressed_size)\t\(.status)"'
}
latest_snapshot() { snapshots_of "$1" | awk -F'\t' '$4=="complete"' | tail -n 1; }

# Refuses while a deploy holds its lock on control (deploy/deploy.sh --status).
require_lock_free() {
  [ -f "$STATE_DIR/ssh_config" ] || die "no $(tilde "$STATE_DIR")/ssh_config: run this from the checkout that ran apply"
  local rc=0
  ssh "${SSH_OPTS[@]}" agx-control 'test -d /run/lock/agentrix-deploy' 2>/dev/null || rc=$?
  case $rc in
    0) die "a deploy holds the lock ($(ssh "${SSH_OPTS[@]}" agx-control 'cat /run/lock/agentrix-deploy/holder' 2>/dev/null)); try again after it" ;;
    1) ;;
    *) die "could not reach control to check the deploy lock" ;;
  esac
}

# Refuses before SLEEP_NOT_BEFORE: judges use the live site, Assess needs the sandbox and every
# deploy needs the fixture until judging ends.
require_after_judging() {
  [ -n "$SLEEP_NOT_BEFORE" ] || die "SLEEP_NOT_BEFORE is not set in $(tilde "$ENV_FILE"). Set it to when judging ends (specs/platform/cost.md); nothing sleeps before then"
  local now not_before
  now=$(date -u +%s)
  not_before=$(date -u -d "$SLEEP_NOT_BEFORE" +%s 2>/dev/null || date -u -j -f '%Y-%m-%dT%H:%M:%S%z' "$SLEEP_NOT_BEFORE" +%s 2>/dev/null) ||
    die "SLEEP_NOT_BEFORE=$SLEEP_NOT_BEFORE is not a time such as 2026-11-20T00:00:00+0300"
  [ "$now" -ge "$not_before" ] || die "not before $SLEEP_NOT_BEFORE: judging may still be running"
}

parse_roles() { # sets ROLES and YES
  YES=0 ROLES=()
  local a
  for a in "$@"; do
    case $a in
      --yes) YES=1 ;;
      control|sandbox|fixture) ROLES+=("$a") ;;
      *) die "unknown argument $a (roles: control, sandbox, fixture; and --yes)" ;;
    esac
  done
}

sleepable() {
  case $1 in
    sandbox|fixture) ;;
    control) die "control does not sleep: it serves agentrix.website, holds control.db and the secrets, and the other hosts' firewalls name its VPC address (specs/platform/cost.md)" ;;
  esac
}

what_goes_down() {
  case $1 in
    control) printf 'agentrix.website, every site it serves, the console and the Model Gateway' ;;
    sandbox) printf 'Assess and the Move (the runner and the sealed rooms); deploys too, whose smoke test checks the runner' ;;
    fixture) printf 'fixture.agentrix.website, the gauntlet'"'"'s target; deploys too, which re-run the gauntlet' ;;
  esac
}

cmd_costs() {
  key >/dev/null; load_prices
  local account balance pending
  account=$(api GET /account)
  balance=$(jq -r '.account.balance' <<<"$account"); pending=$(jq -r '.account.pending_charges' <<<"$account")
  say "Vultr, this month so far: \$$pending pending$(awk -v b="$balance" 'BEGIN { if (b < 0) printf " · $%.2f credit left", -b; else if (b > 0) printf " · $%.2f owed", b }')"
  jq -r '.pending_charges[] | "  \(.description)\t\(.units) \(.unit_type) × $\(.unit_price*10000|round/10000)\t$\(.total)"' <<<"$PENDING"
  say "Running now (billed by the hour up to each monthly cap):"
  local role id r total=0
  for role in control sandbox fixture; do
    id=$(instance_id "$role")
    if [ -n "$id" ]; then r=$(rate "$role"); total=$(calc "$total + $r" 4)
      printf '  %-8s %-20s $%s/h  $%s/day\n' "$role" "$(role_plan "$role")" "$(calc "$r" 4)" "$(calc "$r * 24")"
    else printf '  %-8s asleep or absent (no instance)\n' "$role"; fi
  done
  local os; os=$(jq -r '[.pending_charges[] | select(.product=="Object Storage") | .unit_price] | add // 0' <<<"$PENDING")
  [ "$os" = 0 ] || { printf '  %-8s %-20s $%s/h  $%s/day\n' storage "Object Storage" "$(calc "$os" 4)" "$(calc "$os * 24")"; total=$(calc "$total + $os" 4); }
  local snaps gb
  snaps=$(api GET "/snapshots?per_page=500" | jq -r --arg p "$PREFIX-" '.snapshots[] | (.description |= gsub("^\\s+"; "")) | select(.description|startswith($p)) | "\(.description)\t\(.status)\t\(.compressed_size)"')
  gb=$(printf '%s\n' "$snaps" | awk -F'\t' 'NF { s += $3 } END { printf "%.1f", s / 1e9 }')
  say "  Total    \$$(calc "$total" 3)/h, \$$(calc "$total * 24")/day, plus snapshots \$$(calc "$gb * $SNAPSHOT_USD_GB_MONTH")/month (${gb} GB)"
  if [ -n "$snaps" ]; then say "Snapshots:"; printf '%s\n' "$snaps" | awk -F'\t' -v p="$SNAPSHOT_USD_GB_MONTH" '{ printf "  %s\t%s\t%.1f GB\t$%.2f/month\n", $1, $2, $3/1e9, $3/1e9*p }'; fi
  local rips; rips=$(api GET "/reserved-ips?per_page=500" | jq -r '.reserved_ips | length')
  [ "$rips" = 0 ] || say "Reserved IPs: $rips at \$3/month each"
  local save=0.0000
  for role in sandbox fixture; do [ -n "$(instance_id "$role")" ] && save=$(calc "$save + $(rate "$role")" 4); done
  [ "$save" = 0.0000 ] || say "Sleeping sandbox and fixture would save \$$(calc "$save * 24")/day (deploy/provision.sh sleep sandbox fixture)."
  say "Model spend is metered by the gateway (control.db usage), not on this bill: specs/platform/cost.md."
}

# snapshot_now ROLE: takes a snapshot, records the host key, prints its id. Waits for nothing.
snapshot_now() {
  local role=$1 id sid stamp
  id=$(instance_id "$role"); [ -n "$id" ] || die "$PREFIX-$role is not running"
  mkdir -p "$STATE_DIR/hostkeys"
  ssh "${SSH_OPTS[@]}" "agx-$role" 'cat /etc/ssh/ssh_host_ed25519_key.pub' > "$STATE_DIR/hostkeys/$role.pub" 2>/dev/null ||
    say "  $role: could not read its host key over SSH; wake will not be able to check it" >&2
  stamp=$(date -u +%Y-%m-%dT%H:%MZ)
  sid=$(api POST /snapshots "$(jq -n --arg i "$id" --arg d "$PREFIX-$role $stamp" '{instance_id:$i, description:$d}')" | jq -r '.snapshot.id')
  [ -n "$sid" ] && [ "$sid" != null ] || die "Vultr did not start a snapshot of $PREFIX-$role"
  say "  snapshot     $PREFIX-$role $stamp started ($sid)" >&2
  printf '%s' "$sid"
}

wait_snapshot() {
  local sid=$1 role=$2 t0=$3 s status size
  for _ in $(seq 1 270); do
    s=$(api GET "/snapshots/$sid"); status=$(jq -r '.snapshot.status' <<<"$s")
    if [ "$status" = complete ]; then
      size=$(jq -r '.snapshot.compressed_size' <<<"$s")
      say "  snapshot     $PREFIX-$role complete in $(( ($(date +%s) - t0) / 60 )) min: $(calc "$size / 1e9" 1) GB, \$$(calc "$size / 1e9 * $SNAPSHOT_USD_GB_MONTH")/month"
      printf '%s %s %s %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$role" "$sid" "$size" "$(( $(date +%s) - t0 ))s" >> "$STATE_DIR/snapshots.log"
      return 0
    fi
    sleep 20
  done
  die "snapshot $sid of $PREFIX-$role is not complete after 90 minutes (status: $status); nothing was deleted"
}

cmd_snapshot() {
  parse_roles "$@"; [ ${#ROLES[@]} -gt 0 ] || ROLES=(control sandbox fixture)
  key >/dev/null
  local role id gb
  if [ "$YES" = 0 ]; then
    say "Snapshot plan (dry run). A snapshot changes nothing on the VM and runs while it serves."
    for role in "${ROLES[@]}"; do
      id=$(instance_id "$role"); [ -n "$id" ] || { say "  $role: not running, nothing to snapshot"; continue; }
      gb=$(used_gb "$role")
      say "  $PREFIX-$role ($id): $(snap_cost "$gb") (Vultr bills the compressed size)"
      say "    already: $(snapshots_of "$role" | awk -F'\t' 'END { print NR " snapshot(s)" }')"
    done
    say "Take them outside the first release's windows A to D and log each on the board; then: deploy/provision.sh snapshot ${ROLES[*]} --yes"
    return 0
  fi
  require_lock_free
  local sids=() t0; t0=$(date +%s)
  for role in "${ROLES[@]}"; do sids+=("$(snapshot_now "$role")"); done
  local i; for i in "${!ROLES[@]}"; do wait_snapshot "${sids[$i]}" "${ROLES[$i]}" "$t0"; done
}

cmd_sleep() {
  parse_roles "$@"; [ ${#ROLES[@]} -gt 0 ] || die "name what sleeps: deploy/provision.sh sleep sandbox fixture"
  local role; for role in "${ROLES[@]}"; do sleepable "$role"; done
  key >/dev/null; load_prices
  local id r gb save=0 month=0 snap=0
  say "Sleep plan$([ "$YES" = 1 ] || printf ' (dry run)'): a fresh snapshot of each, then the VM is deleted."
  for role in "${ROLES[@]}"; do
    id=$(instance_id "$role"); [ -n "$id" ] || { say "  $role: already asleep"; continue; }
    r=$(rate "$role"); gb=$(used_gb "$role"); save=$(calc "$save + $r" 4); snap=$(calc "$snap + ${gb:-0} * $SNAPSHOT_USD_GB_MONTH" 4)
    month=$(calc "$month + $(jq -r --arg p "$(role_plan "$role")" '.plans[] | select(.id==$p) | .monthly_cost' <<<"$PLANS")")
    say "  $PREFIX-$role ($(role_plan "$role")): saves \$$(calc "$r" 4)/h, \$$(calc "$r * 24")/day"
    say "    goes down: $(what_goes_down "$role")"
    say "    its snapshot: $(snap_cost "$gb")"
  done
  say "  Net: \$$(calc "$save * 24 - $snap / 30.4")/day saved, up to \$$(calc "$month - $snap")/month (the plans' monthly caps), snapshots included."
  say "  Kept: control, the VPC, the firewall groups, DNS, Object Storage. deploy/provision.sh wake ${ROLES[*]} brings them back."
  say "  Guards: SLEEP_NOT_BEFORE=${SLEEP_NOT_BEFORE:-<not set: sleep refuses>} · no deploy holding the lock."
  [ "$YES" = 1 ] || { say "Needs James's yes on the board, then: deploy/provision.sh sleep ${ROLES[*]} --yes"; return 0; }
  require_after_judging
  require_lock_free
  local sids=() live=() t0; t0=$(date +%s)
  for role in "${ROLES[@]}"; do
    [ -n "$(instance_id "$role")" ] || continue
    live+=("$role"); sids+=("$(snapshot_now "$role")")
  done
  local i; for i in "${!live[@]}"; do wait_snapshot "${sids[$i]}" "${live[$i]}" "$t0"; done
  for role in "${live[@]}"; do
    id=$(instance_id "$role")
    api DELETE "/instances/$id" >/dev/null
    say "  deleted      $PREFIX-$role $id (asleep: its snapshot is the only copy)"
  done
  say "Asleep. Deploys and Assess need them: deploy/provision.sh wake ${live[*]} --yes"
}

# wait_ready ID: a restore reports active early and stays locked while it writes the disk.
wait_ready() {
  local id=$1 s=""
  for _ in $(seq 1 180); do
    s=$(api GET "/instances/$id" | jq -r '"\(.instance.status)/\(.instance.server_status)"')
    [ "$s" = active/ok ] && return 0
    sleep 10
  done
  die "instance $id is not ready after 30 minutes ($s)"
}

inv_set() {
  local f=$STATE_DIR/inventory.env
  if grep -q "^$1=" "$f"; then sed -i.bak "s|^$1=.*|$1=$2|" "$f" && rm -f "$f.bak"; else printf '%s=%s\n' "$1" "$2" >> "$f"; fi
}
ssh_host_set() { # ROLE IP: the HostName under "Host agx-ROLE" in ssh_config
  local f=$STATE_DIR/ssh_config
  awk -v h="Host agx-$1" -v ip="$2" '$0 == h { inb = 1; print; next } /^Host / { inb = 0 } inb && $1 == "HostName" { print "  HostName " ip; next } { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}

# known_key ROLE OLD_ADDRESS: the host key recorded at snapshot time, else the ed25519 key
# ~/.ssh/known_hosts holds for the address the VM had before it slept.
known_key() {
  if [ -s "$STATE_DIR/hostkeys/$1.pub" ]; then cut -d' ' -f1-2 < "$STATE_DIR/hostkeys/$1.pub"; return 0; fi
  [ -n "${2:-}" ] || return 0
  ssh-keygen -F "$2" 2>/dev/null | awk '$2 == "ssh-ed25519" { print $2 " " $3; exit }'
}

# trust_host ROLE ADDRESS OLD_ADDRESS: if SSH refuses a new address, compare the key seen there
# (control's from here, the others' from control) with the known one, and only then add it to
# ~/.ssh/known_hosts. Cloud-init keeps the snapshot's keys because wake asks it to.
trust_host() {
  local role=$1 ip=$2 old=${3:-} want seen
  ssh "${SSH_OPTS[@]}" "agx-$role" true 2>/dev/null && return 0
  want=$(known_key "$role" "$old")
  [ -n "$want" ] || die "SSH to agx-$role at $ip fails and no earlier host key is known; check it by hand"
  if [ "$role" = control ]; then
    seen=$(ssh-keyscan -t ed25519 -T 5 "$ip" 2>/dev/null | awk '{ print $2 " " $3 }' | head -n 1)
  else
    seen=$(ssh "${SSH_OPTS[@]}" agx-control "ssh-keyscan -t ed25519 -T 5 $ip 2>/dev/null" | awk '{ print $2 " " $3 }' | head -n 1)
  fi
  [ -n "$seen" ] || return 1
  [ "$seen" = "$want" ] || die "agx-$role at $ip shows a host key that differs from the one known for ${old:-its old address} (cloud-init may have made new keys; control's did on 8 Nov). Check its fingerprint in Vultr's web console (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub), then ssh-keygen -R $ip, connect once to accept it, and run wake again for the roles still asleep. Restored roles are skipped; once all three exist, provision.sh apply --yes rewrites DNS and the inventory"
  ssh-keygen -R "$ip" >/dev/null 2>&1 || true
  printf '%s %s\n' "$ip" "$want" >> "$HOME/.ssh/known_hosts"
  say "  ssh          agx-$role at $ip: host key matches the snapshot's; added to ~/.ssh/known_hosts"
}

cmd_wake() {
  parse_roles "$@"; [ ${#ROLES[@]} -gt 0 ] || die "name what wakes: deploy/provision.sh wake sandbox fixture"
  # Always control first: the sandbox's and fixture's firewalls name its VPC address, and in an
  # empty VPC the first VM attached gets the first address, as when apply created them.
  local role ordered=()
  for role in control sandbox fixture; do [[ " ${ROLES[*]} " == *" $role "* ]] && ordered+=("$role"); done
  ROLES=("${ordered[@]}")
  key >/dev/null; load_prices
  local snap r add=0
  say "Wake plan$([ "$YES" = 1 ] || printf ' (dry run)'): each VM recreated from its latest snapshot, same label, plan, firewall group and VPC."
  for role in "${ROLES[@]}"; do
    [ -z "$(instance_id "$role")" ] || { say "  $role: already running"; continue; }
    snap=$(latest_snapshot "$role"); [ -n "$snap" ] || die "$role has no complete snapshot to wake from"
    r=$(jq -r --arg p "$(role_plan "$role")" '.plans[] | select(.id==$p) | .hourly_cost' <<<"$PLANS"); add=$(calc "$add + $r" 4)
    say "  $PREFIX-$role from snapshot $(cut -f2 <<<"$snap") ($(calc "$(cut -f3 <<<"$snap") / 1e9" 1) GB): adds about \$$(calc "$r" 3)/h back"
    case $role in
      sandbox) say "    its VPC address is Vultr's choice; if it is not $(grep -E '^SANDBOX_VPC_IP=' "$STATE_DIR/inventory.env" 2>/dev/null | cut -d= -f2-), node reaches the runner only after the next deploy" ;;
      fixture) say "    new public address: ${FIXTURE_FQDN:-the fixture}'s A and AAAA records are rewritten (TTL 300)" ;;
      control) say "    goes first. New public address: ${PLATFORM_FQDN:-the platform}'s and *.${PLATFORM_FQDN:-platform}'s A and AAAA records are rewritten (TTL 300)."
               say "    Its VPC address must come back as $(grep -E '^CONTROL_VPC_IP=' "$STATE_DIR/inventory.env" 2>/dev/null | cut -d= -f2-): the other hosts' firewalls accept only it" ;;
    esac
  done
  say "  Kept from the snapshot: room images and their pinned IDs, WordPress, the runner's job key; host keys if cloud-init keeps them."
  [ "$YES" = 1 ] || { say "Needs James's yes on the board, then: deploy/provision.sh wake ${ROLES[*]} --yes"; return 0; }
  [ -f "$STATE_DIR/inventory.env" ] || die "no $(tilde "$STATE_DIR")/inventory.env"
  local sshkey vpc fw id t0 t1 t2 vip ip ip6 old ud code want_code what failed=()
  sshkey=$(ensure_ssh_key)
  vpc=$(api GET "/vpcs?per_page=500" | jq -r --arg d "$PREFIX" --arg r "$REGION" '.vpcs[] | select(.description==$d and .region==$r) | .id' | head -n 1)
  [ -n "$vpc" ] || die "no $PREFIX VPC in $REGION"
  # Keep the snapshot's host keys (cloud-init deletes them on a new instance by default).
  ud=$(printf '#cloud-config\nssh_deletekeys: false\nssh_pwauth: false\n' | base64 | tr -d '\n')
  for role in "${ROLES[@]}"; do
    [ -z "$(instance_id "$role")" ] || continue
    [ "$role" = control ] || [ -n "$(instance_id control)" ] || die "wake control first: the other hosts' firewalls name its VPC address"
    snap=$(latest_snapshot "$role"); fw=$(ensure_group "$role")
    t0=$(date +%s)
    id=$(api POST /instances "$(jq -n --arg r "$REGION" --arg p "$(role_plan "$role")" --arg s "$(cut -f1 <<<"$snap")" \
      --arg l "$PREFIX-$role" --arg fw "$fw" --arg vpc "$vpc" --arg k "$sshkey" --arg ud "$ud" --argjson tags "$TAGS" \
      '{region:$r, plan:$p, snapshot_id:$s, label:$l, hostname:$l, firewall_group_id:$fw, attach_vpc:[$vpc], sshkey_id:[$k],
        user_data:$ud, enable_ipv6:true, backups:"disabled", activation_email:false, tags:$tags}')" | jq -r '.instance.id')
    say "  instance     $PREFIX-$role $id from $(cut -f2 <<<"$snap")"
    wait_ready "$id"; t1=$(date +%s)
    vip=$(vpc_ip "$id" "$vpc")
    local addr=$vip oldaddr
    case $role in
      control)
        old=$(grep -E '^CONTROL_VPC_IP=' "$STATE_DIR/inventory.env" | cut -d= -f2-)
        oldaddr=$(grep -E '^CONTROL_IP=' "$STATE_DIR/inventory.env" | cut -d= -f2-)
        ip=$(main_ip "$id" main_ip); ip6=$(main_ip "$id" v6_main_ip); addr=$ip
        inv_set CONTROL_ID "$id"; inv_set CONTROL_IP "$ip"; inv_set CONTROL_IP6 "$ip6"; inv_set CONTROL_VPC_IP "$vip"
        dns_upsert "$PLATFORM_ZONE" "$PLATFORM_NAME" A "$ip"
        dns_upsert "$PLATFORM_ZONE" "$PLATFORM_NAME" AAAA "$ip6"
        dns_upsert "$PLATFORM_ZONE" "*${PLATFORM_NAME:+.$PLATFORM_NAME}" A "$ip"
        dns_upsert "$PLATFORM_ZONE" "*${PLATFORM_NAME:+.$PLATFORM_NAME}" AAAA "$ip6" ;;
      sandbox)
        old=$(grep -E '^SANDBOX_VPC_IP=' "$STATE_DIR/inventory.env" | cut -d= -f2-)
        oldaddr=$old
        inv_set SANDBOX_ID "$id"; inv_set SANDBOX_VPC_IP "$vip"; inv_set RUNNER_ADDR "$vip:$RUNNER_PORT" ;;
      fixture)
        old=$(grep -E '^FIXTURE_VPC_IP=' "$STATE_DIR/inventory.env" | cut -d= -f2-); oldaddr=$old
        ip=$(main_ip "$id" main_ip); ip6=$(main_ip "$id" v6_main_ip)
        inv_set FIXTURE_ID "$id"; inv_set FIXTURE_IP "$ip"; inv_set FIXTURE_IP6 "$ip6"; inv_set FIXTURE_VPC_IP "$vip"
        dns_upsert "$FIXTURE_ZONE" "$FIXTURE_NAME" A "$ip"
        dns_upsert "$FIXTURE_ZONE" "$FIXTURE_NAME" AAAA "$ip6" ;;
    esac
    ssh_host_set "$role" "$addr"
    for _ in $(seq 1 30); do trust_host "$role" "$addr" "$oldaddr" && break; sleep 10; done
    ssh "${SSH_OPTS[@]}" "agx-$role" true || die "agx-$role does not answer SSH (ADMIN_CIDR still your address? deploy/provision.sh apply --yes re-adds the rule)"
    # "Serving" means the role's own check passes, not just that the VM booted.
    want_code=200
    case $role in
      control) what="https://$PLATFORM_FQDN/healthz" ;;
      sandbox) what="the runner at $vip:$RUNNER_PORT, from control"; want_code=404 ;;
      fixture) what="https://$FIXTURE_FQDN/"
        # The fixture's compose project has no restart policy, so a restored VM boots with
        # WordPress and its database stopped (Exited 255). Start them as they were.
        if ssh "${SSH_OPTS[@]}" agx-fixture 'cd /opt/fixture && sudo -n docker compose start' >/dev/null 2>&1; then
          say "  fixture      started WordPress and its database (docker compose start in /opt/fixture)"
        else
          say "  !! could not start the fixture's containers: ssh agx-fixture 'cd /opt/fixture && sudo docker compose start'"
        fi ;;
    esac
    code=""
    for _ in $(seq 1 60); do
      case $role in
        control) code=$(curl -s -o /dev/null -m 10 -w '%{http_code}' --resolve "$PLATFORM_FQDN:443:$ip" "https://$PLATFORM_FQDN/healthz") ;;
        sandbox) code=$(ssh "${SSH_OPTS[@]}" agx-control "curl -s -o /dev/null -m 5 -w '%{http_code}' http://$vip:$RUNNER_PORT/") ;;
        fixture) code=$(curl -s -o /dev/null -m 10 -w '%{http_code}' --resolve "$FIXTURE_FQDN:443:$ip" "https://$FIXTURE_FQDN/") ;;
      esac
      [ "$code" = "$want_code" ] && break
      sleep 5
    done
    t2=$(date +%s)
    if [ "$code" = "$want_code" ]; then
      say "  awake        $PREFIX-$role: disk restored in $(( (t1 - t0) / 60 )) min, serving after $(( (t2 - t0) / 60 )) min $(( (t2 - t0) % 60 )) s"
      printf '%s wake %s %s %ss %ss\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$role" "$id" "$(( t1 - t0 ))" "$(( t2 - t0 ))" >> "$STATE_DIR/snapshots.log"
    else
      say "  !! $PREFIX-$role: restored in $(( (t1 - t0) / 60 )) min, but $what answered ${code:-nothing}, not $want_code, after 5 minutes"
      printf '%s wake %s %s %ss not-serving\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$role" "$id" "$(( t1 - t0 ))" >> "$STATE_DIR/snapshots.log"
      failed+=("$role")
    fi
    if [ "$role" = control ] && [ "$vip" != "$old" ]; then
      say "  !! control's VPC address moved from $old to $vip. The sandbox's and fixture's snapshots accept SSH and"
      say "     the runner only from $old: fix their nftables and ufw from Vultr's web console, or rebuild them with apply."
      die "stopping before the sandbox and fixture (specs/platform/cost.md, restore)"
    fi
    if [ "$role" = sandbox ] && [ "$vip" != "$old" ]; then
      say "  !! the sandbox's VPC address moved from $old to $vip. inventory.env now says RUNNER_ADDR=$vip:$RUNNER_PORT;"
      say "     node reaches the runner only after the next deploy/deploy.sh, which installs that address."
    fi
  done
  [ ${#failed[@]} -eq 0 ] || die "restored, but not serving: ${failed[*]} (see above). Fix that before any deploy"
  say "Awake. deploy/provision.sh costs shows the new run rate."
}

# Times a restore without touching the live VM: a copy from the role's latest snapshot, labelled
# -restoretest, outside the VPC and in the sandbox's firewall group (nothing inbound, so the
# fixture's infected WordPress stays off the internet), deleted as soon as Vultr says it's ready.
cmd_restore_test() {
  parse_roles "$@"; [ ${#ROLES[@]} -eq 1 ] || die "name one role: deploy/provision.sh restore-test sandbox"
  local role=${ROLES[0]} snap r label=$PREFIX-${ROLES[0]}-restoretest
  key >/dev/null; load_prices
  snap=$(latest_snapshot "$role"); [ -n "$snap" ] || die "$role has no complete snapshot yet: deploy/provision.sh snapshot $role"
  r=$(jq -r --arg p "$(role_plan "$role")" '.plans[] | select(.id==$p) | .monthly_cost / 672' <<<"$PLANS")
  say "Restore test$([ "$YES" = 1 ] || printf ' (dry run)'): $label from snapshot $(cut -f2 <<<"$snap") ($(calc "$(cut -f3 <<<"$snap") / 1e9" 1) GB)"
  say "  on $(role_plan "$role"), no VPC, firewall group $PREFIX-sandbox (nothing inbound); deleted when ready."
  say "  Costs about \$$(calc "$r" 2) per started hour (Vultr bills whole hours). The live $PREFIX-$role is not touched."
  [ "$YES" = 1 ] || { say "Needs James's yes on the board, then: deploy/provision.sh restore-test $role --yes"; return 0; }
  [ -z "$(api GET "/instances?label=$label&per_page=100" | jq -r '.instances[0].id // empty')" ] || die "$label already exists; delete it first"
  local sshkey fw id t0 t1
  sshkey=$(ensure_ssh_key); fw=$(ensure_group sandbox)
  t0=$(date +%s)
  id=$(api POST /instances "$(jq -n --arg r "$REGION" --arg p "$(role_plan "$role")" --arg s "$(cut -f1 <<<"$snap")" \
    --arg l "$label" --arg fw "$fw" --arg k "$sshkey" --argjson tags "$TAGS" \
    '{region:$r, plan:$p, snapshot_id:$s, label:$l, hostname:$l, firewall_group_id:$fw, sshkey_id:[$k],
      enable_ipv6:false, backups:"disabled", activation_email:false, tags:$tags}')" | jq -r '.instance.id')
  [ -n "$id" ] && [ "$id" != null ] || die "Vultr did not create $label"
  # shellcheck disable=SC2064 # the id is fixed now: the copy is deleted however this ends
  trap "api DELETE '/instances/$id' >/dev/null 2>&1 || true" EXIT
  say "  instance     $label $id"
  local s=""
  for _ in $(seq 1 180); do
    s=$(api GET "/instances/$id" | jq -r '"\(.instance.status)/\(.instance.server_status)"')
    [ "$s" = active/ok ] && break
    sleep 10
  done
  t1=$(date +%s)
  api DELETE "/instances/$id" >/dev/null && say "  deleted      $label $id"
  trap - EXIT
  [ "$s" = active/ok ] || die "$label was not ready after 30 minutes ($s); the copy was deleted"
  say "  restore time: $(( (t1 - t0) / 60 )) min $(( (t1 - t0) % 60 )) s from create to ready"
  mkdir -p "$STATE_DIR"
  printf '%s restore-test %s %s %ss\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$role" "$(cut -f1 <<<"$snap")" "$(( t1 - t0 ))" >> "$STATE_DIR/snapshots.log"
}

case ${1:-plan} in
  plan) cmd_plan ;;
  apply) shift; cmd_apply "$@" ;;
  status) cmd_status ;;
  render-file) shift; [ -f "${1:-}" ] || die "render-file needs a template path"
    render "$1" "$( [ -f "$STATE_DIR/inventory.env" ] && grep -E '^CONTROL_VPC_IP=' "$STATE_DIR/inventory.env" | cut -d= -f2- || echo 203.0.113.10)" ;;
  ssh-close) shift; cmd_ssh_close "$@" ;;
  destroy) shift; cmd_destroy "$@" ;;
  costs) cmd_costs ;;
  snapshot) shift; cmd_snapshot "$@" ;;
  sleep) shift; cmd_sleep "$@" ;;
  wake) shift; cmd_wake "$@" ;;
  dns) shift; cmd_dns "$@" ;;
  restore-test) shift; cmd_restore_test "$@" ;;
  -h|--help|help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command $1 (plan, apply --yes, dns --yes, status, render-file F, ssh-close --yes, destroy --yes, costs, snapshot, sleep, wake, restore-test)" ;;
esac
