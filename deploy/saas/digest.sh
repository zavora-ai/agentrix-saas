#!/usr/bin/env bash
# Agentrix · what happened on saas-node in the last hour (or --since 30m, 2h, 1d). Read only.
#
#   deploy/saas/digest.sh [--since 1h]
#
# Plain text, for the coordinator's hourly digest. It reads, over SSH, and changes nothing:
#   - model calls per company and agent (model, tokens, cost) from the gateway's meter
#     (agentrix-node's control.db), and the caps used today (UTC)
#   - each company's agent runs, audit actions (wakes, tool calls, decisions), approvals asked
#     and decided, and the held, denied or refused ones with their reasons
#   - money moved on PayPal (sandbox): payments, payouts, refunds, plan payments, webhooks, and
#     company-to-company settlements, with PayPal's ids
#   - the sample sites' payments, the test customer agents' visits and their off switch
#   - refusals at the doors, wallet checkpoints, Zavora ERA's journal entries
#   - WARN and ERROR lines from the journal
# SQLite opens read only (mode=ro); ERA is asked with SELECT only. No secret or record body is
# printed: only ids, kinds, amounts and reasons.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
STATE=${SAAS_DEPLOY_STATE:-$HOME/.config/agentrix-saas/deploy}
SINCE=1h
while [ $# -gt 0 ]; do
  case $1 in
    --since) [ $# -ge 2 ] || { echo "digest: --since needs a duration such as 1h" >&2; exit 2; }; SINCE=$2; shift ;;
    -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "digest: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
case $SINCE in
  *[0-9]m) SECS=$(( ${SINCE%m} * 60 )) ;;
  *[0-9]h) SECS=$(( ${SINCE%h} * 3600 )) ;;
  *[0-9]d) SECS=$(( ${SINCE%d} * 86400 )) ;;
  *) echo "digest: --since takes minutes, hours or days: 30m, 1h, 1d" >&2; exit 2 ;;
esac
[ -f "$STATE/ssh_config" ] || { echo "digest: no $STATE/ssh_config (provision.sh apply first)" >&2; exit 1; }
: "$HERE"

ssh -F "$STATE/ssh_config" -o BatchMode=yes saas-node "sudo -n env SECS=$SECS bash -s" <<'EOF'
set -uo pipefail
now=$(date -u +%s); since=$((now - SECS))
since_iso=$(date -u -d "@$since" +%Y-%m-%dT%H:%M:%S)
since_us=$((since * 1000000))
today=$(date -u +%Y-%m-%d)
N=/var/lib/agentrix-node/control.db
S=/var/lib/agentrix-saas
q() { [ -f "$1" ] && sqlite3 -separator ' | ' "file:$1?mode=ro" "$2" 2>/dev/null; }
# Integer times in these stores are seconds or milliseconds; compare either with $since.
secs() { echo "(case when $1 > 100000000000 then $1 / 1000 else $1 end)"; }
section() { printf '\n%s\n' "$1"; }
indent() { sed 's/^/  /'; }
none() { local t; t=$(cat); if [ -n "$t" ]; then printf '%s\n' "$t" | indent; else echo "  (none)"; fi; }
setting() { sed -n "s/^$1=//p" /etc/agentrix-saas/agentrix-node.env /etc/agentrix-saas/agentrix-models.env 2>/dev/null | tail -n 1; }

printf 'Agentrix live digest · %s to %s UTC (%s) · deployed %s\n' \
  "$(date -u -d "@$since" +%H:%M)" "$(date -u +%H:%M)" "$today" "$(head -n 1 /opt/agentrix-saas/DEPLOYED 2>/dev/null | cut -c1-12)"
printf 'Services: node %s · models %s · caddy %s · era-api %s\n' \
  "$(systemctl is-active agentrix-node)" "$(systemctl is-active agentrix-models)" "$(systemctl is-active caddy)" \
  "$(curl -fsS -m 3 -o /dev/null -w ok http://127.0.0.1:18180/health 2>/dev/null || echo down)"

section "Model calls (site | agent | model | calls | tokens in | tokens out | USD | not ok)"
q "$N" "select site_id, coalesce(webagent, '-'), model, count(*), sum(tokens_in), sum(tokens_out),
          printf('%.4f', sum(cost_micro_usd) / 1e6), sum(outcome != 'ok')
        from usage where at >= '$since_iso' group by 1, 2, 3 order by 1, 2, 3" | none
q "$N" "select '  not ok: ' || outcome || ' (' || site_id || ', ' || coalesce(webagent, '-') || ', ' || purpose || ') x' || count(*)
        from usage where at >= '$since_iso' and outcome != 'ok' group by outcome, site_id, webagent, purpose"

section "Caps used today ($today UTC)"
fleet=$(setting AGENTRIX_FLEET_DAILY_CAP_USD); site=$(setting AGENTRIX_SITE_DAILY_CAP_USD)
q "$N" "select '  all sites: \$' || printf('%.4f', coalesce(sum(cost_micro_usd), 0) / 1e6) || ' of \$${fleet:-?}' from usage where at >= '$today'"
q "$N" "select '  ' || site_id || ': \$' || printf('%.4f', sum(cost_micro_usd) / 1e6) || ' of \$${site:-?}' from usage where at >= '$today' group by site_id order by site_id"

for c in agentrix kilele; do
  db=$S/$c/$c.db
  section "Agents: $c"
  echo "  runs (target | state | count):"
  q "$db" "select target, state, count(*) from adk_runs where created_at_us >= $since_us group by 1, 2 order by 1, 2" | indent | none
  echo "  audit (kind | action | count):"
  q "$db" "select json_extract(record, '\$.kind'), json_extract(record, '\$.action'), count(*) from adk_audit_entries
           where json_extract(record, '\$.ts') >= '$since_iso' group by 1, 2 order by 1, 2" | indent | none
  echo "  tool calls with effects (tool | outcome | count):"
  q "$db" "select tool_name, coalesce(outcome, 'in flight'), count(*) from adk_action_ledger where started_at >= '$since_iso' group by 1, 2 order by 1, 2" | indent | none
  echo "  approvals asked (state | count):"
  q "$db" "select state, count(*) from adk_approval_requests where created_at_us >= $since_us group by 1 order by 1" | indent | none
  echo "  held, denied or refused (time | action | reason):"
  q "$db" "select substr(json_extract(record, '\$.ts'), 12, 8), json_extract(record, '\$.action'),
                  substr(coalesce(json_extract(record, '\$.ext.reason'), json_extract(record, '\$.ext.why'),
                                  json_extract(record, '\$.ext.message'), json_extract(record, '\$.ext.code'), ''), 1, 160)
           from adk_audit_entries
           where json_extract(record, '\$.ts') >= '$since_iso'
             and (json_extract(record, '\$.action') like '%den%' or json_extract(record, '\$.action') like '%hold%'
               or json_extract(record, '\$.action') like '%held%' or json_extract(record, '\$.action') like '%refus%'
               or json_extract(record, '\$.action') like '%block%' or json_extract(record, '\$.action') like '%freeze%')
           order by 1 limit 25" | indent | none
done

section "Money on PayPal (sandbox)"
for c in agentrix kilele; do
  E=$S/$c-effects.db
  echo "  $c payments (state | amount | currency | PayPal order | capture):"
  q "$E" "select state, printf('%.2f', amount_minor / 100.0), currency, coalesce(provider_ref, '-'), coalesce(capture_id, '-')
          from customer_payments where $(secs updated_at) >= $since order by updated_at" | indent | none
  echo "  $c payouts (state | amount | currency | batch | item | item status):"
  q "$E" "select state, printf('%.2f', amount_minor / 100.0), currency, coalesce(batch_id, '-'), coalesce(item_id, '-'), coalesce(item_status, '-')
          from payouts where $(secs updated_at) >= $since order by updated_at" | indent | none
  echo "  $c refunds (state | amount | currency | PayPal refund):"
  q "$E" "select state, printf('%.2f', amount_minor / 100.0), currency, coalesce(provider_ref, '-')
          from refunds where $(secs created_at) >= $since order by created_at" | indent | none
  echo "  $c plan payments (subscription | PayPal sale | amount):"
  q "$E" "select provider_ref, provider_payment_id, printf('%.2f', amount_minor / 100.0)
          from plan_payments where $(secs paid_at) >= $since order by paid_at" | indent | none
  echo "  $c webhooks received (kind | resource | count):"
  q "$E" "select kind, resource, count(*) from paypal_events where $(secs received_at) >= $since group by 1, 2 order by 1" | indent | none
done
for c in agentrix kilele; do
  echo "  $c settlements (state | route | amount | currency | invoice | PayPal):"
  q "$S/settle/$c.db" "select state, route, printf('%.2f', amount_minor / 100.0), currency, invoice_id, coalesce(paypal_key, '-')
                      from settle_carts where $(secs updated_at) >= $since order by updated_at" | indent | none
done

echo "  sample sites' payments (site | state | amount | currency | PayPal order | capture):"
q "$N" "select site_id, state, printf('%.2f', amount_minor / 100.0), currency, coalesce(provider_ref, '-'), coalesce(capture_id, '-')
        from customer_payments where $(secs updated_at) >= $since order by updated_at" | indent | none

section "Test customer agents (time | agent | kind | ok | site | what)"
A=$S/test-agents/activity.jsonl
if [ -f "$A" ]; then
  jq -r --arg s "$since_iso" 'select(.at >= $s) | [(.at[11:19]), .agent, .kind, (if .ok then "ok" else "NOT OK" end),
         (.site | sub("^https://"; "") | sub("\\.paypal\\.agentrix\\.website$"; "")), (.detail | .[0:110])] | join(" | ")' "$A" 2>/dev/null | tail -n 40 | none
  printf '  service %s; %s\n' "$(systemctl is-active agentrix-test-agents)" "$(test -e "$S/test-agents/off" && echo 'paused (off file present)' || echo 'not paused')"
else
  echo "  (no activity log yet)"
fi

section "Refusals at the doors (company | code | count)"
for c in agentrix kilele; do
  q "$S/settle/$c.db" "select '$c', code, count(*) from settle_refusals where $(secs at) >= $since group by code"
done | none

section "Wallet checkpoints (log | size | note)"
for w in "$S"/wallets/*.db; do
  [ -f "$w" ] || continue
  # A checkpoint note is a signed note (origin, size, root hash, blank line, signature): one line.
  q "$w" "select log_id, size, substr(replace(replace(trim(note), char(10) || char(10), char(10)), char(10), ' / '), 1, 140)
          from adk_audit_checkpoints where created_at_us >= $since_us order by created_at_us"
done | none

section "Zavora ERA journal entries (company | number | source | reference | status)"
pg=$(docker ps -q --filter label=com.docker.compose.project=saas-era --filter label=com.docker.compose.service=postgres)
if [ -n "$pg" ]; then
  map=$(jq -r 'to_entries[] | "\(.value.entity_id)=\(.key)"' "$S/era/tenants.json" 2>/dev/null)
  docker exec "$pg" psql -U zavora -d zavora_era -At -F ' | ' -c \
    "select entity_id, number, coalesce(source, '-'), coalesce(reference, '-'), status from journal_entries
     where created_at >= to_timestamp($since) order by created_at" 2>/dev/null \
    | while IFS= read -r line; do
        id=${line%% | *}; name=$(printf '%s\n' "$map" | sed -n "s/^$id=//p")
        printf '%s | %s\n' "${name:-$id}" "${line#* | }"
      done | none
else
  echo "  (ERA's Postgres is not running)"
fi

section "Warnings and errors in the journal (count | message)"
journalctl -u agentrix-node -u agentrix-models --since "@$since" -o cat --no-pager 2>/dev/null \
  | grep -E ' (WARN|ERROR) ' | sed -E 's/^[0-9T:.Z-]+ +//; s/ +/ /g' | cut -c1-200 | sort | uniq -c | sort -rn | head -n 15 | none
EOF
