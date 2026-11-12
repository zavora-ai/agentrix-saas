#!/usr/bin/env bash
# Agentrix (spec v3) · provision saas-node on Vultr: one VM, its firewall group, its SSH key, and
# the agentrix.website zone on it (four explicit A records from apply; the apex and the wildcard
# through `zone`). deploy/provision.sh is the ended Vultr entry's three-VM script and stays
# disabled here.
#
#   deploy/saas/provision.sh [plan]          Read only (the default). Lists the account's VMs, the
#                                            zone's records, firewall groups and SSH keys; shows
#                                            exactly what apply would add and the price from the
#                                            API; refuses when something saas-* or one of the four
#                                            names already exists with other values.
#   deploy/saas/provision.sh apply --yes     PAID. Runs the plan's checks, then creates what is
#                                            missing: SSH key saas-admin, firewall group saas-fw,
#                                            VM saas-node, and the four A records. Re-runnable.
#   deploy/saas/provision.sh status          Read only: our objects, and the VM's first boot.
#   deploy/saas/provision.sh ssh-allow [--yes]   Points saas-fw's SSH rule at this Mac's IP now.
#   deploy/saas/provision.sh snapshot [--yes] [--wait]   Snapshots saas-node (after submission).
#   deploy/saas/provision.sh destroy --yes   Deletes only what apply created (state file ids,
#                                            each checked again first). After judging (~21 Dec).
#   deploy/saas/provision.sh zone [--yes]    Spec v3: the apex and wildcard A records to saas-node,
#                                            their AAAA records and fixture's deleted, each only
#                                            while it holds an address no VM in the account has.
#                                            A dry run without --yes (James's yes first).
#
# Safety:
#   - apply only adds records (money, kilele.money, pay, books), never PATCHes or deletes one.
#     `zone --yes` is the one command that changes records it did not create: the apex and
#     wildcard A (to saas-node) and stale AAAA and fixture records, each only while it still
#     holds an address no VM in the account has. MX, TXT, CNAME and NS are never touched.
#   - No VM is touched except the one labelled saas-node and tagged agentrix-saas.
#   - The Vultr key is read from the repo's .env line by line, sent to curl on stdin (never on a
#     command line), and never printed. A value starting with '#' counts as empty.
#
# Settings (environment, all optional): SAAS_ENV_FILE (default: the repo's .env),
# SAAS_DEPLOY_STATE (default ~/.config/agentrix-saas/deploy), SAAS_VULTR_REGION (default: the
# region of agx-control, read from the API), SAAS_VULTR_PLAN (default vc2-4c-8gb),
# SAAS_VULTR_OS_ID (default 2284, Ubuntu 24.04 LTS x64, as on the agx-* VMs), SAAS_ADMIN_CIDR
# (default: this Mac's public IPv4 as /32), SAAS_SSH_KEY (default ~/.ssh/agentrix_saas_ed25519).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON=$(git -C "$HERE" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
ENV_FILE=${SAAS_ENV_FILE:-${COMMON:+$(dirname "$COMMON")/.env}}
STATE=${SAAS_DEPLOY_STATE:-$HOME/.config/agentrix-saas/deploy}
API=https://api.vultr.com/v2

LABEL=saas-node
TAG=agentrix-saas
FW_NAME=saas-fw
KEY_NAME=saas-admin
ZONE=agentrix.website
NAMES=(money kilele.money pay books)         # exactly these four A records, nothing else
# Spec v3's platform host (agentrix-saas.env): with the apex and the wildcard, its record and its
# own wildcard point at saas-node (paypal and *.paypal for paypal.agentrix.website).
PLATFORM_HOST=${SAAS_PLATFORM_HOST:-$(sed -n 's/^SAAS_PLATFORM_HOST=//p' "$HERE/agentrix-saas.env" 2>/dev/null | tail -n 1)}
POINT_NAMES=("" "*")
case $PLATFORM_HOST in
  *."$ZONE") POINT_NAMES+=("${PLATFORM_HOST%".$ZONE"}" "*.${PLATFORM_HOST%".$ZONE"}") ;;
esac
PLAN=${SAAS_VULTR_PLAN:-vc2-4c-8gb}
OS_ID=${SAAS_VULTR_OS_ID:-2284}
SSH_KEY=${SAAS_SSH_KEY:-$HOME/.ssh/agentrix_saas_ed25519}
SNAPSHOT_USD_GB_MONTH=0.05                    # Vultr bills a snapshot's compressed size

die() { printf 'provision: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }
need curl; need jq; need base64; need python3
tilde() { case $1 in "$HOME"*) printf '~%s' "${1#"$HOME"}";; *) printf '%s' "$1";; esac; }

# --- configuration -----------------------------------------------------------------------

# One value from the .env, read line by line, never sourced. A value starting with '#' is empty.
env_value() {
  [ -n "$ENV_FILE" ] && [ -f "$ENV_FILE" ] || return 0
  awk -v k="$1" 'index($0, k "=") == 1 { v = substr($0, length(k) + 2); sub(/^["\047]/, "", v); sub(/["\047]$/, "", v); if (substr(v, 1, 1) == "#") v = ""; print v; exit }' "$ENV_FILE"
}

key() {
  local k; k=$(env_value VULTR_API_KEY)
  [ -n "$k" ] || die "VULTR_API_KEY is empty in $(tilde "${ENV_FILE:-?}")"
  printf '%s' "$k"
}

# The state file is KEY=value lines, parsed, never sourced.
inv() { [ -f "$STATE/inventory.env" ] && grep -E "^$1=" "$STATE/inventory.env" | tail -n 1 | cut -d= -f2- || true; }
inv_set() {
  mkdir -p "$STATE" && chmod 700 "$STATE"
  touch "$STATE/inventory.env"
  { grep -vE "^$1=" "$STATE/inventory.env" || true; printf '%s=%s\n' "$1" "$2"; } > "$STATE/inventory.env.new"
  mv "$STATE/inventory.env.new" "$STATE/inventory.env"
}

# --- the Vultr API -----------------------------------------------------------------------

# api METHOD PATH [JSON]: the body on stdout. On HTTP >= 300 the status and Vultr's error go to
# stderr (they never contain the key) and it returns 1. The key reaches curl through --config
# on stdin.
API_LAST_STATUS=""
api() {
  local method=$1 path=$2 body=${3:-} out code
  local args=(-sS --max-time 60 -X "$method" "$API$path" -H 'Content-Type: application/json' -w '\n%{http_code}')
  [ -n "$body" ] && args+=(--data-binary "$body")
  out=$(printf 'header = "Authorization: Bearer %s"\n' "$(key)" | curl --config - "${args[@]}") || { API_LAST_STATUS=000; printf 'provision: %s %s: no answer from the API\n' "$method" "$path" >&2; return 1; }
  code=${out##*$'\n'}; out=${out%$'\n'*}
  API_LAST_STATUS=$code
  if [ "$code" -ge 300 ]; then
    printf 'provision: %s %s answered HTTP %s: %s\n' "$method" "$path" "$code" "$(jq -c . <<<"$out" 2>/dev/null || printf '%s' "$out")" >&2
    return 1
  fi
  printf '%s' "$out"
}

# api_all PATH FIELD: every item of FIELD across the cursor pages, as one JSON array.
api_all() {
  local path=$1 field=$2 cursor="" page sep acc='[]'
  case $path in *\?*) sep='&';; *) sep='?';; esac
  while :; do
    page=$(api GET "$path${sep}per_page=500${cursor:+&cursor=$cursor}") || return 1
    acc=$(jq -c --argjson a "$acc" --arg f "$field" '$a + (.[$f] // [])' <<<"$page")
    cursor=$(jq -r '.meta.links.next // ""' <<<"$page")
    [ -n "$cursor" ] || break
  done
  printf '%s' "$acc"
}

public() { curl -sS --fail-with-body --max-time 60 "$API$1"; }

# --- reading the account (read only) -----------------------------------------------------

admin_cidr() {
  if [ -n "${SAAS_ADMIN_CIDR:-}" ]; then printf '%s' "$SAAS_ADMIN_CIDR"; return; fi
  local ip; ip=$(curl -4 -sS --max-time 10 https://api.ipify.org || true)
  [[ $ip =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "could not read this Mac's public IPv4; set SAAS_ADMIN_CIDR=a.b.c.d/32"
  printf '%s/32' "$ip"
}

pubkey() { [ -f "$SSH_KEY.pub" ] && cut -d' ' -f1-2 < "$SSH_KEY.pub" || true; }

# Loads everything the checks need into globals. Stops at the first refused read (the 401 when
# Vultr's access control does not list this Mac's IP), after showing it.
INSTANCES='' RECORDS='' FIREWALLS='' SSHKEYS='' REGION='' ADMIN=''
read_account() {
  local blocked=0
  INSTANCES=$(api_all /instances instances) || blocked=1
  [ $blocked = 0 ] && { RECORDS=$(api_all "/domains/$ZONE/records" records) || blocked=1; }
  [ $blocked = 0 ] && { FIREWALLS=$(api_all /firewalls firewall_groups) || blocked=1; }
  [ $blocked = 0 ] && { SSHKEYS=$(api_all /ssh-keys ssh_keys) || blocked=1; }
  if [ $blocked = 1 ]; then
    say ""
    if [ "$API_LAST_STATUS" = 401 ]; then
      say "STOPPED: the Vultr API refused a read (HTTP 401 above). Its access control must allow this"
      say "Mac's IP (my.vultr.com → Account → API → Access Control). Nothing was created or changed."
    else
      say "STOPPED: a read failed (HTTP ${API_LAST_STATUS:-?} above). Nothing was created or changed."
    fi
    exit 3
  fi
  REGION=${SAAS_VULTR_REGION:-$(jq -r '[.[] | select(.label=="agx-control")][0].region // empty' <<<"$INSTANCES")}
  REGION=${REGION:-atl}
  ADMIN=$(admin_cidr)
}

fw_group() { jq -c --arg d "$FW_NAME" '[.[] | select(.description==$d)]' <<<"$FIREWALLS"; }

# The rules saas-fw must hold, in a canonical form for comparing.
wanted_rules() {
  jq -nc --arg a "${ADMIN%/*}" --argjson z "${ADMIN#*/}" '[
    {ip_type:"v4", protocol:"tcp", port:"22",  subnet:$a,        subnet_size:$z},
    {ip_type:"v4", protocol:"tcp", port:"443", subnet:"0.0.0.0", subnet_size:0},
    {ip_type:"v4", protocol:"tcp", port:"80",  subnet:"0.0.0.0", subnet_size:0}] | sort_by(.port)'
}
canon_rules() { jq -c '[.[] | {ip_type, protocol, port:(.port|tostring), subnet, subnet_size}] | sort_by(.port)'; }

# --- the checks plan and apply share -----------------------------------------------------

# check: prints what exists and what would be added, sets ACTIONS, and returns 1 on a conflict.
ACTIONS=()
check() {
  local conflict=0 n rows count
  ACTIONS=()
  local node node_ip=""
  node=$(jq -c --arg l "$LABEL" '[.[] | select(.label==$l)]' <<<"$INSTANCES")
  count=$(jq length <<<"$node")

  say "VMs in the account (read only; none is changed):"
  jq -r '.[] | "  \(.label)\t\(.region)\t\(.plan)\t\(.main_ip)\t\(.status)\ttags=\(.tags|join(","))"' <<<"$INSTANCES" | column -t -s $'\t' | sed 's/^/  /'
  if [ "$count" = 0 ]; then
    ACTIONS+=("instance")
  elif [ "$count" = 1 ] && [ "$(jq -r --arg t "$TAG" '.[0] | (.tags|index($t)) != null' <<<"$node")" = true ] \
      && [ "$(jq -r '.[0].plan' <<<"$node")" = "$PLAN" ] && [ "$(jq -r '.[0].region' <<<"$node")" = "$REGION" ]; then
    node_ip=$(jq -r '.[0].main_ip' <<<"$node")
    say "  -> $LABEL exists, tagged $TAG, $PLAN in $REGION: reused ($node_ip)"
  else
    say "  CONFLICT: $count VM(s) labelled $LABEL exist that are not this script's (tag $TAG, $PLAN, $REGION)."
    conflict=1
  fi
  local others; others=$(jq -r --arg l "$LABEL" '[.[] | select((.label|startswith("saas-")) and .label!=$l) | .label] | join(", ")' <<<"$INSTANCES")
  [ -z "$others" ] || say "  note: other saas-* VMs exist and are left alone: $others"

  say ""
  say "Records in $ZONE (read only; none is changed or deleted):"
  jq -r '.[] | "\(.type)\t\(if .name=="" then "@" else .name end)\t\(if (.data|length) > 48 then .data[0:45] + "..." else .data end)\tttl \(.ttl)"' <<<"$RECORDS" | sort | column -t -s $'\t' | sed 's/^/    /'
  for n in "${NAMES[@]}"; do
    rows=$(jq -c --arg n "$n" '[.[] | select(.name==$n)]' <<<"$RECORDS")
    count=$(jq length <<<"$rows")
    if [ "$count" = 0 ]; then
      ACTIONS+=("record:$n")
    elif [ "$count" = 1 ] && [ -n "$node_ip" ] && [ "$(jq -r '.[0].type' <<<"$rows")" = A ] && [ "$(jq -r '.[0].data' <<<"$rows")" = "$node_ip" ]; then
      say "  -> A $n.$ZONE already points at $LABEL ($node_ip): kept"
    else
      say "  CONFLICT: $n.$ZONE already has: $(jq -r 'map("\(.type) \(.data)") | join("; ")' <<<"$rows")"
      conflict=1
    fi
  done
  # Spec v3 serves the whole zone from saas-node: the apex and the wildcard point at it too.
  local where
  for n in "${POINT_NAMES[@]}"; do
    where=$(jq -r --arg n "$n" '[.[] | select(.name==$n and (.type=="A" or .type=="AAAA")) | "\(.type) \(.data)"] | join(", ")' <<<"$RECORDS")
    if [ -n "$node_ip" ] && [ "$where" = "A $node_ip" ]; then
      say "  -> ${n:-@} points at $LABEL ($node_ip), spec v3"
    else
      say "  note: ${n:-@} holds ${where:-nothing}; spec v3 points it at $LABEL (provision.sh zone, James's yes)"
      [ -n "$where" ] || say "        (zone points only existing records; a missing one is added in the console)"
    fi
  done

  say ""
  say "Firewall groups (read only): $(jq -r 'map(.description) | join(", ")' <<<"$FIREWALLS")"
  local fw; fw=$(fw_group)
  count=$(jq length <<<"$fw")
  if [ "$count" = 0 ]; then
    ACTIONS+=("firewall")
  elif [ "$count" = 1 ]; then
    local id have want
    id=$(jq -r '.[0].id' <<<"$fw")
    have=$(api_all "/firewalls/$id/rules" firewall_rules | canon_rules)
    want=$(wanted_rules)
    if [ "$have" = "$want" ]; then
      say "  -> $FW_NAME exists with the same rules: reused"
    elif [ "$id" = "$(inv FIREWALL_ID)" ] && jq -e --argjson w "$want" 'all(.[]; . as $r | any($w[]; . == $r))' <<<"$have" >/dev/null; then
      say "  -> $FW_NAME was made by this script ($id) and lacks rules: apply adds the missing ones"
      ACTIONS+=("rules")
    else
      say "  CONFLICT: $FW_NAME exists with other rules: $have"
      say "            wanted: $want  (if only the SSH address moved: provision.sh ssh-allow --yes)"
      conflict=1
    fi
  else
    say "  CONFLICT: $count firewall groups are named $FW_NAME"; conflict=1
  fi

  say ""
  say "SSH keys (read only): $(jq -r 'map(.name) | join(", ")' <<<"$SSHKEYS")"
  local ours pub; ours=$(jq -c --arg n "$KEY_NAME" '[.[] | select(.name==$n)]' <<<"$SSHKEYS"); pub=$(pubkey)
  count=$(jq length <<<"$ours")
  if [ "$count" = 0 ]; then
    ACTIONS+=("sshkey")
  elif [ "$count" = 1 ] && [ -n "$pub" ] && [ "$(jq -r '.[0].ssh_key | split(" ")[0:2] | join(" ")' <<<"$ours")" = "$pub" ]; then
    say "  -> $KEY_NAME exists with $(tilde "$SSH_KEY.pub"): reused"
  else
    say "  CONFLICT: an SSH key named $KEY_NAME exists that is not $(tilde "$SSH_KEY.pub")"; conflict=1
  fi
  return $conflict
}

# Renders cloud-init with the admin key and checks it is valid YAML.
render_cloud_init() {
  local pub=$1 out=$STATE/cloud-init.yaml
  mkdir -p "$STATE" && chmod 700 "$STATE"
  sed -e "s|@@SSH_PUBKEY@@|$pub|" "$HERE/cloud-init.yaml" > "$out"
  python3 -I -c 'import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
assert isinstance(d, dict) and d.get("hostname") == "saas-node", "not saas-node cloud-config"
assert "@@" not in open(sys.argv[1]).read(), "unfilled token"' "$out" 2>/dev/null \
    || python3 -I -c 'import sys; t = open(sys.argv[1]).read(); assert t.startswith("#cloud-config") and "@@" not in t' "$out" \
    || die "rendered cloud-init is not valid: $out"
  printf '%s' "$out"
}

price() {
  local plans avail
  plans=$(public "/plans?per_page=500") || die "could not read Vultr's plans"
  avail=$(public "/regions/$REGION/availability") || die "region $REGION not found"
  jq -r --arg p "$PLAN" '.plans[] | select(.id==$p) | "\(.vcpu_count) vCPU, \(.ram/1024) GB RAM, \(.disk) GB SSD, \(.bandwidth/1024) TB transfer: $\(.monthly_cost)/month, $\(.hourly_cost)/hour"' <<<"$plans" | grep . || die "plan $PLAN does not exist"
  jq -e --arg p "$PLAN" '.available_plans | index($p)' <<<"$avail" >/dev/null || die "plan $PLAN is not available in $REGION"
}

plan_header() {
  say "Agentrix SaaS · saas-node provisioning plan (read only: nothing is created)"
  say ""
  say "  .env:    $(tilde "${ENV_FILE:-?}") (Vultr key $( [ -n "$(env_value VULTR_API_KEY)" ] && echo 'present, not shown' || echo MISSING))"
  say "  state:   $(tilde "$STATE")"
}

cmd_plan() {
  plan_header
  read_account
  local cost; cost=$(price)
  say "  region:  $REGION (as agx-control) · OS $OS_ID (Ubuntu 24.04 LTS x64) · plan $PLAN"
  say "  price:   $cost"
  say "  SSH:     $(tilde "$SSH_KEY") $( [ -f "$SSH_KEY.pub" ] && echo '(exists)' || echo '(apply creates it: ed25519, no passphrase)') · port 22 from $ADMIN only"
  say ""
  local ok=0; check || ok=1
  say ""
  if [ ${#ACTIONS[@]} = 0 ]; then say "Nothing to add."; else
    say "apply would add only these (each matched by name first, never overwritten):"
    local a
    # In the order apply makes them.
    for a in $(printf '%s\n' ${ACTIONS[@]+"${ACTIONS[@]}"} | awk '{ o = /^sshkey/ ? 1 : /^(firewall|rules)/ ? 2 : /^instance/ ? 3 : 4; print o "\t" $0 }' | sort | cut -f2); do
      case $a in
        sshkey)   say "  + SSH key   $KEY_NAME ($(tilde "$SSH_KEY.pub"))" ;;
        firewall) say "  + firewall  $FW_NAME: tcp 22 from $ADMIN; tcp 80, 443 from anywhere (IPv4; no IPv6 rules, the VM has no IPv6)" ;;
        rules)    say "  + rules     $FW_NAME's missing ones of: tcp 22 from $ADMIN; tcp 80, 443 from anywhere" ;;
        instance) say "  + VM        $LABEL: $PLAN in $REGION, OS $OS_ID, tag $TAG, group $FW_NAME, key $KEY_NAME, no IPv6, no auto backups" ;;
        record:*) say "  + DNS       A ${a#record:}.$ZONE -> $LABEL's IPv4, ttl 300" ;;
      esac
    done
  fi
  local pub rendered; pub=$(pubkey); [ -n "$pub" ] || pub="ssh-ed25519 PLACEHOLDER-UNTIL-APPLY"
  rendered=$(render_cloud_init "$pub")
  say "  cloud-init rendered and checked: $(tilde "$rendered")"
  say ""
  if [ $ok = 1 ]; then say "REFUSED: resolve the conflicts above first; apply would stop at the same place."; exit 4; fi
  say "To create these (paid, after James approves the spend):"
  say "  deploy/saas/provision.sh apply --yes"
}

# --- apply -------------------------------------------------------------------------------

ensure_local_key() {
  [ -f "$SSH_KEY" ] && [ -f "$SSH_KEY.pub" ] && return 0
  [ ! -e "$SSH_KEY" ] || die "$SSH_KEY exists without its .pub"
  need ssh-keygen
  ssh-keygen -q -t ed25519 -N '' -C "$KEY_NAME" -f "$SSH_KEY"
  say "  ssh key      made $(tilde "$SSH_KEY") locally"
}

cmd_apply() {
  [ "${1:-}" = --yes ] || die "apply creates a paid VM. Run the plan, get James's yes, then: deploy/saas/provision.sh apply --yes"
  plan_header
  read_account
  local cost; cost=$(price)
  say "  region:  $REGION · plan $PLAN: $cost"
  say ""
  check || die "refused: conflicts above; nothing was created"
  say ""
  ensure_local_key
  local pub; pub=$(pubkey)
  local ud; ud=$(render_cloud_init "$pub")
  inv_set REGION "$REGION"; inv_set PLAN "$PLAN"; inv_set ADMIN_CIDR "$ADMIN"

  local sshkey fw id ip a
  sshkey=$(jq -r --arg n "$KEY_NAME" '[.[] | select(.name==$n)][0].id // empty' <<<"$SSHKEYS")
  if [ -z "$sshkey" ]; then
    sshkey=$(api POST /ssh-keys "$(jq -nc --arg n "$KEY_NAME" --arg k "$pub" '{name:$n, ssh_key:$k}')" | jq -r '.ssh_key.id')
    say "  ssh key      created $KEY_NAME $sshkey"
  fi
  inv_set SSHKEY_ID "$sshkey"

  fw=$(fw_group | jq -r '.[0].id // empty')
  if [ -z "$fw" ]; then
    fw=$(api POST /firewalls "$(jq -nc --arg d "$FW_NAME" '{description:$d}')" | jq -r '.firewall_group.id')
    inv_set FIREWALL_ID "$fw"
    say "  firewall     created $FW_NAME $fw"
  fi
  inv_set FIREWALL_ID "$fw"
  ensure_rules "$fw"

  id=$(jq -r --arg l "$LABEL" '[.[] | select(.label==$l)][0].id // empty' <<<"$INSTANCES")
  if [ -z "$id" ]; then
    id=$(api POST /instances "$(jq -nc --arg r "$REGION" --arg p "$PLAN" --argjson o "$OS_ID" --arg l "$LABEL" \
      --arg fw "$fw" --arg k "$sshkey" --arg ud "$(base64 < "$ud" | tr -d '\n')" --arg t "$TAG" \
      '{region:$r, plan:$p, os_id:$o, label:$l, hostname:$l, firewall_group_id:$fw, sshkey_id:[$k], user_data:$ud,
        enable_ipv6:false, backups:"disabled", activation_email:false, tags:[$t]}')" | jq -r '.instance.id')
    say "  instance     created $LABEL $id ($PLAN)"
  fi
  inv_set INSTANCE_ID "$id"

  say "  waiting for $LABEL to be active (a few minutes)"
  local status=""
  for _ in $(seq 1 90); do
    status=$(api GET "/instances/$id" | jq -r '.instance.status + " " + .instance.main_ip')
    case $status in "active "[1-9]*) break;; esac
    sleep 10
  done
  case $status in "active "[1-9]*) ;; *) die "instance $id is not active after 15 minutes ($status)";; esac
  ip=${status#active }
  [ "$ip" != 0.0.0.0 ] || die "instance $id has no IPv4 yet"
  inv_set NODE_IP "$ip"
  say "  instance     active at $ip"

  for a in ${ACTIONS[@]+"${ACTIONS[@]}"}; do
    case $a in record:*)
      local n=${a#record:} rec
      rec=$(api POST "/domains/$ZONE/records" "$(jq -nc --arg n "$n" --arg d "$ip" '{name:$n, type:"A", data:$d, ttl:300}')" | jq -r '.record.id')
      inv_set "RECORD_${n//./_}" "$rec"
      say "  dns          created A $n.$ZONE -> $ip ($rec)"
      ;;
    esac
  done
  for n in "${NAMES[@]}"; do
    [ -n "$(inv "RECORD_${n//./_}")" ] && continue
    inv_set "RECORD_${n//./_}" "$(api_all "/domains/$ZONE/records" records | jq -r --arg n "$n" --arg d "$ip" '[.[] | select(.name==$n and .type=="A" and .data==$d)][0].id // empty')"
  done

  write_ssh_config "$ip"
  say ""
  say "Waiting for first boot (cloud-init installs Docker, gVisor, Caddy, Node.js and Rust: ~10 min)"
  wait_first_boot
  say ""
  say "Done. Next: deploy/saas/deploy.sh   (state: $(tilde "$STATE"))"
}

# ensure_rules GROUP: adds each wanted rule the group lacks, one call per rule; removes none.
ensure_rules() {
  local fw=$1 have missing rule
  have=$(api_all "/firewalls/$fw/rules" firewall_rules | canon_rules)
  missing=$(wanted_rules | jq -c --argjson h "$have" '.[] | . as $r | select(any($h[]; . == $r) | not)')
  while IFS= read -r rule; do
    [ -n "$rule" ] || continue
    api POST "/firewalls/$fw/rules" "$(jq -c '. + {notes: (if .port == "22" then "admin ssh" else "web" end)}' <<<"$rule")" >/dev/null
    say "    rule       $(jq -r '"tcp \(.port) from \(.subnet)/\(.subnet_size)"' <<<"$rule")"
  done <<<"$missing"
}

write_ssh_config() {
  cat > "$STATE/ssh_config" <<EOF
# saas-node, written by deploy/saas/provision.sh. Root login is off; deploys use 'deploy'.
Host saas-node
  HostName $1
  User deploy
  IdentityFile $SSH_KEY
  IdentitiesOnly yes
  UserKnownHostsFile $STATE/known_hosts
  StrictHostKeyChecking accept-new
  ServerAliveInterval 30
  ConnectTimeout 15
EOF
  chmod 600 "$STATE/ssh_config"
}

wait_first_boot() {
  local ssh=(ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node)
  for _ in $(seq 1 60); do "${ssh[@]}" true 2>/dev/null && break; sleep 10; done
  "${ssh[@]}" true || die "no SSH to saas-node as deploy after 10 minutes (state: $(tilde "$STATE"))"
  "${ssh[@]}" 'sudo -n cloud-init status --wait >/dev/null 2>&1; sudo -n cloud-init status --long | sed -n "1,3p"; cat /var/lib/agentrix-saas/host-facts.json 2>/dev/null || echo "no host facts yet"'
}

# --- spec v3: the whole zone on saas-node -----------------------------------------------

# zone_changes IP: one JSON line per change spec v3 asks of the zone, given $RECORDS and
# $INSTANCES. The A records of POINT_NAMES (the apex, the wildcard, the platform host and its
# wildcard) point at IP; their AAAA records go (saas-node
# has no IPv6, and a record left on a released address can be claimed by whoever gets it next);
# fixture's records go. Each line says whether its current address belongs to a VM that still
# exists ("live"), which refuses the change. MX, TXT, CNAME, NS and every other name stay.
zone_changes() {
  local live
  live=$(jq -c '[.[] | (.main_ip // empty), (.v6_main_ip // empty)] | map(select(. != "" and . != "0.0.0.0"))' <<<"$INSTANCES")
  local names; names=$(printf '%s\n' "${POINT_NAMES[@]}" | jq -R . | jq -sc .)
  jq -c --arg ip "$1" --argjson live "$live" --argjson names "$names" '
    def canon: if test(":") then ascii_downcase | split(":") | map(sub("^0+(?=.)"; "")) | join(":") else . end;
    ($live | map(canon)) as $alive
    | .[] | select(.type == "A" or .type == "AAAA")
    | (if .name == "" then "@" else .name end) as $shown
    | (.data | canon) as $d
    | (.name as $n | any($names[]; . == $n)) as $pointed
    | if $pointed and .type == "A" then
        (if .data == $ip then empty else {op: "point", id, name: $shown, type, from: .data, to: $ip} end)
      elif $pointed and .type == "AAAA" then {op: "delete", id, name: $shown, type, from: .data}
      elif .name == "fixture" then {op: "delete", id, name: $shown, type, from: .data}
      else empty end
    | . + {live: ($alive | index([$d]) != null)}' <<<"$RECORDS"
}

cmd_zone() {
  local yes=0; [ "${1:-}" = --yes ] && yes=1
  say "Agentrix (spec v3) · the agentrix.website zone onto saas-node$( [ $yes = 1 ] || echo ' (dry run: nothing is changed)')"
  say ""
  read_account
  local ip changes refused c
  ip=$(jq -r --arg l "$LABEL" --arg t "$TAG" '[.[] | select(.label == $l and (.tags | index($t)))][0].main_ip // empty' <<<"$INSTANCES")
  [ -n "$ip" ] || die "no $LABEL VM tagged $TAG in the account"
  say "VMs: $(jq -r 'map("\(.label) \(.main_ip)") | join(", ")' <<<"$INSTANCES")"
  changes=$(zone_changes "$ip")
  if [ -z "$changes" ]; then say "Nothing to change: $(printf '%s ' "${POINT_NAMES[@]/#/}" | sed 's/^ /@ /')already point at $LABEL ($ip)."; return; fi
  say "Changes (each only while the record still holds the address shown):"
  while IFS= read -r c; do
    jq -r '"  \(if .op == "point" then "~ point " else "- delete" end)  \(.type)\t\(.name)\t\(.from)\(if .op == "point" then " -> " + .to else "" end)\(if .live then "   REFUSED: a VM in the account holds this address" else "" end)"' <<<"$c"
  done <<<"$changes"
  say "Unchanged: every other record ($(jq -r --argjson ch "$(jq -s 'map(.id)' <<<"$changes")" '[.[] | select(.id as $i | $ch | index($i) | not)] | length' <<<"$RECORDS") of them: MX, TXT, CNAME, NS, the four saas-node A records)."
  refused=$(jq -s 'map(select(.live)) | length' <<<"$changes")
  [ "$refused" = 0 ] || die "refused: a record above points at a VM that still exists"
  if [ $yes = 0 ]; then say ""; say "To make these changes (after James approves): deploy/saas/provision.sh zone --yes"; return; fi

  mkdir -p "$STATE" && chmod 700 "$STATE"
  jq -s . <<<"$changes" > "$STATE/zone-before-$(date -u +%Y%m%dT%H%M%SZ).json"
  local id now from
  while IFS= read -r c; do
    id=$(jq -r .id <<<"$c"); from=$(jq -r .from <<<"$c")
    now=$(api GET "/domains/$ZONE/records/$id" | jq -r .record.data) || die "record $id unreadable"
    [ "$now" = "$from" ] || { say "  skipped $(jq -r '.type + " " + .name' <<<"$c"): it now holds $now, not $from"; continue; }
    case $(jq -r .op <<<"$c") in
      point)  api PATCH "/domains/$ZONE/records/$id" "$(jq -nc --arg d "$ip" '{data:$d, ttl:300}')" >/dev/null
              say "  pointed $(jq -r '.type + " " + .name' <<<"$c") -> $ip" ;;
      delete) api DELETE "/domains/$ZONE/records/$id" >/dev/null
              say "  deleted $(jq -r '.type + " " + .name + " " + .from' <<<"$c")" ;;
    esac
  done <<<"$changes"
  say "Done. The previous values are in $(tilde "$STATE")/zone-before-*.json."
}

# --- status, ssh-allow, snapshot, destroy ------------------------------------------------

our_instance() {
  local id; id=$(inv INSTANCE_ID)
  [ -n "$id" ] || die "no INSTANCE_ID in $(tilde "$STATE")/inventory.env (run apply first)"
  local row; row=$(api GET "/instances/$id") || die "instance $id is gone or unreadable"
  if [ "$(jq -r '.instance.label' <<<"$row")" != "$LABEL" ] || ! jq -e --arg t "$TAG" '.instance.tags | index($t)' <<<"$row" >/dev/null; then
    die "instance $id is not labelled $LABEL and tagged $TAG: not touching it"
  fi
  printf '%s' "$row"
}

cmd_status() {
  local row; row=$(our_instance)
  jq -r '.instance | "saas-node  \(.id)  \(.status)/\(.power_status)  \(.main_ip)  \(.plan)  \(.region)  created \(.date_created)"' <<<"$row"
  local n rec
  for n in "${NAMES[@]}"; do
    rec=$(inv "RECORD_${n//./_}")
    printf 'dns        A %-32s %s (authoritative: %s)\n' "$n.$ZONE" "${rec:-not created}" "$(dig +short A "$n.$ZONE" @ns1.vultr.com 2>/dev/null | tr '\n' ' ')"
  done
  local fw; fw=$(inv FIREWALL_ID)
  [ -z "$fw" ] || printf 'firewall   %s %s\n' "$FW_NAME" "$(api_all "/firewalls/$fw/rules" firewall_rules | jq -r 'map("\(.port) from \(.subnet)/\(.subnet_size)") | join(", ")')"
  [ -f "$STATE/ssh_config" ] && ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node 'sudo -n cloud-init status; cat /var/lib/agentrix-saas/host-facts.json 2>/dev/null; cat /opt/agentrix-saas/DEPLOYED 2>/dev/null' || true
}

cmd_ssh_allow() {
  local fw; fw=$(inv FIREWALL_ID); [ -n "$fw" ] || die "no FIREWALL_ID in the state file"
  local group; group=$(api GET "/firewalls/$fw") || die "firewall group $fw unreadable"
  [ "$(jq -r '.firewall_group.description' <<<"$group")" = "$FW_NAME" ] || die "group $fw is not $FW_NAME: not touching it"
  ADMIN=$(admin_cidr)
  local rules old; rules=$(api_all "/firewalls/$fw/rules" firewall_rules)
  old=$(jq -r '[.[] | select(.port=="22")] | map("\(.subnet)/\(.subnet_size)") | join(", ")' <<<"$rules")
  if [ "$old" = "$ADMIN" ]; then say "SSH already allowed from $ADMIN only"; return; fi
  say "SSH rule in $FW_NAME: $old -> $ADMIN"
  [ "${1:-}" = --yes ] || { say "(dry run; add --yes)"; return; }
  api POST "/firewalls/$fw/rules" "$(jq -nc --arg a "${ADMIN%/*}" --argjson z "${ADMIN#*/}" '{ip_type:"v4", protocol:"tcp", port:"22", subnet:$a, subnet_size:$z, notes:"admin ssh"}')" >/dev/null
  jq -r --arg a "${ADMIN%/*}" '.[] | select(.port=="22" and .subnet!=$a) | .id' <<<"$rules" | while read -r rid; do
    api DELETE "/firewalls/$fw/rules/$rid" >/dev/null
  done
  inv_set ADMIN_CIDR "$ADMIN"
  say "done"
}

cmd_snapshot() {
  local yes=0 wait=0 a
  for a in "$@"; do case $a in --yes) yes=1;; --wait) wait=1;; *) die "snapshot takes --yes and --wait";; esac; done
  local row id; row=$(our_instance); id=$(jq -r '.instance.id' <<<"$row")
  local used deployed desc
  used=$(ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node "df -BG --output=used / | tail -n 1 | tr -dc 0-9" 2>/dev/null || true)
  deployed=$(ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node "head -n 1 /opt/agentrix-saas/DEPLOYED 2>/dev/null" 2>/dev/null || true)
  desc="saas-node ${deployed:-unknown} $(date -u +%Y-%m-%dT%H:%MZ)"
  say "Snapshot of saas-node ($id): \"$desc\""
  [ -n "$used" ] && say "  ${used} GB in use, so at most \$$(awk "BEGIN { printf \"%.2f\", $used * $SNAPSHOT_USD_GB_MONTH }")/month while it is kept (Vultr bills the compressed size)"
  say "  existing snapshots: $(api_all /snapshots snapshots | jq -r 'map(select((.description // "")|startswith("saas-node"))) | map("\(.description) [\(.status)]") | join("; ") | if .=="" then "none" else . end')"
  [ $yes = 1 ] || { say "(dry run; add --yes)"; return; }
  ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node 'sync' || true
  local snap; snap=$(api POST /snapshots "$(jq -nc --arg i "$id" --arg d "$desc" '{instance_id:$i, description:$d}')" | jq -r '.snapshot.id')
  inv_set LAST_SNAPSHOT_ID "$snap"
  say "  snapshot $snap started"
  [ $wait = 1 ] || return 0
  local s=""
  for _ in $(seq 1 120); do s=$(api GET "/snapshots/$snap" | jq -r '.snapshot.status'); [ "$s" = complete ] && break; sleep 15; done
  say "  snapshot $snap: $s"
}

cmd_destroy() {
  [ "${1:-}" = --yes ] || die "destroy deletes saas-node and its records (James's yes; snapshot first). Add --yes"
  local row id ip n rec r
  row=$(our_instance); id=$(jq -r '.instance.id' <<<"$row"); ip=$(jq -r '.instance.main_ip' <<<"$row")
  for n in "${NAMES[@]}"; do
    rec=$(inv "RECORD_${n//./_}"); [ -n "$rec" ] || continue
    r=$(api GET "/domains/$ZONE/records/$rec" 2>/dev/null) || continue
    if [ "$(jq -r '.record.name + " " + .record.type + " " + .record.data' <<<"$r")" = "$n A $ip" ]; then
      api DELETE "/domains/$ZONE/records/$rec" >/dev/null && say "  dns          deleted A $n.$ZONE"
    else
      say "  dns          record $rec no longer is A $n -> $ip: left alone"
    fi
  done
  api DELETE "/instances/$id" >/dev/null && say "  instance     deleted $LABEL $id"
  local fw; fw=$(inv FIREWALL_ID)
  if [ -n "$fw" ]; then
    sleep 20 # the group stays in use until the instance is gone
    r=$(api GET "/firewalls/$fw") && [ "$(jq -r '.firewall_group.description' <<<"$r")" = "$FW_NAME" ] \
      && api DELETE "/firewalls/$fw" >/dev/null && say "  firewall     deleted $FW_NAME"
  fi
  local k; k=$(inv SSHKEY_ID)
  if [ -n "$k" ]; then
    r=$(api GET "/ssh-keys/$k") && [ "$(jq -r '.ssh_key.name' <<<"$r")" = "$KEY_NAME" ] \
      && api DELETE "/ssh-keys/$k" >/dev/null && say "  ssh key      deleted $KEY_NAME"
  fi
  say "Snapshots are kept (provision.sh snapshot lists them)."
  say "WARNING: the apex and the wildcard still point at $ip, now released. Point them at the next"
  say "server or delete them in the console: whoever gets $ip next would receive agentrix.website."
}

# tests/provision-checks.sh loads the functions without running a command.
[ "${SAAS_PROVISION_LIB:-}" = 1 ] && return 0

case ${1:-plan} in
  plan) cmd_plan ;;
  apply) shift; cmd_apply "$@" ;;
  status) cmd_status ;;
  ssh-allow) shift; cmd_ssh_allow "$@" ;;
  snapshot) shift; cmd_snapshot "$@" ;;
  destroy) shift; cmd_destroy "$@" ;;
  zone) shift; cmd_zone "$@" ;;
  -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command $1 (plan, apply --yes, status, ssh-allow, snapshot, destroy, zone)" ;;
esac
