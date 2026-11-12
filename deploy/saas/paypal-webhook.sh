#!/usr/bin/env bash
# Agentrix · point our PayPal sandbox app's webhook at the money layer's pay host.
#
#   deploy/saas/paypal-webhook.sh            Read only: the webhook PAYPAL_WEBHOOK_ID names (its URL
#                                            and events), and the URL it should have.
#   deploy/saas/paypal-webhook.sh --yes      Replaces that one webhook's URL with
#                                            https://pay.<SAAS_PLATFORM_HOST>/paypal/webhook, once
#                                            that host answers over HTTPS. Nothing else changes:
#                                            not its events, not its id, no other webhook.
#
# The app is this entry's own (PAYPAL_CLIENT_ID in the repo's .env). The id, secret and token
# go to curl on stdin, never on a command line, and are never printed. The webhook id stays
# the same, so the node's signature checks (PAYPAL_WEBHOOK_ID) need no change.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON=$(git -C "$HERE" rev-parse --path-format=absolute --git-common-dir)
ENV_FILE=${SAAS_ENV_FILE:-$(dirname "$COMMON")/.env}
PLATFORM_HOST=${SAAS_PLATFORM_HOST:-$(sed -n 's/^SAAS_PLATFORM_HOST=//p' "$HERE/agentrix-saas.env" | tail -n 1)}
API=https://api-m.sandbox.paypal.com

die() { printf 'paypal-webhook: %s\n' "$*" >&2; exit 1; }
env_value() {
  awk -v k="$1" 'index($0, k "=") == 1 { v = substr($0, length(k) + 2); sub(/^["\047]/, "", v); sub(/["\047]$/, "", v); if (substr(v, 1, 1) == "#") v = ""; print v; exit }' "$ENV_FILE"
}
[ -f "$ENV_FILE" ] || die "no .env at $ENV_FILE"
[ "$(env_value PAYPAL_ENV)" = sandbox ] || die "PAYPAL_ENV is not sandbox: this script only touches the sandbox app"
ID=$(env_value PAYPAL_WEBHOOK_ID)
[ -n "$ID" ] || die "PAYPAL_WEBHOOK_ID is empty"
[ -n "$PLATFORM_HOST" ] || die "no SAAS_PLATFORM_HOST"
WANT="https://pay.$PLATFORM_HOST/paypal/webhook"

# An access token for our app (client credentials), through curl's stdin.
token() {
  local client secret
  client=$(env_value PAYPAL_CLIENT_ID); secret=$(env_value PAYPAL_CLIENT_SECRET)
  [ -n "$client" ] && [ -n "$secret" ] || die "PAYPAL_CLIENT_ID or PAYPAL_CLIENT_SECRET is empty"
  printf 'user = "%s:%s"\n' "$client" "$secret" \
    | curl -sS --fail-with-body --max-time 30 --config - -X POST "$API/v1/oauth2/token" -d grant_type=client_credentials \
    | jq -r '.access_token // empty'
}
TOKEN=$(token)
[ -n "$TOKEN" ] || die "PayPal gave no access token"

# call METHOD PATH [JSON]: the bearer token reaches curl on stdin.
call() {
  local args=(-sS --fail-with-body --max-time 30 -X "$1" "$API$2" -H 'Content-Type: application/json')
  [ -n "${3:-}" ] && args+=(--data-binary "$3")
  printf 'header = "Authorization: Bearer %s"\n' "$TOKEN" | curl --config - "${args[@]}"
}

show() { jq -r '"  id      \(.id)\n  url     \(.url)\n  events  \([.event_types[].name] | join(", "))"'; }

hook=$(call GET "/v1/notifications/webhooks/$ID") || die "our app has no webhook $ID"
echo "Webhook PAYPAL_WEBHOOK_ID names (our sandbox app's):"
show <<<"$hook"
echo "  wanted  $WANT"
[ "$(jq -r .url <<<"$hook")" != "$WANT" ] || { echo "Already there."; exit 0; }
[ "${1:-}" = --yes ] || { echo "(dry run; add --yes once https://pay.$PLATFORM_HOST answers)"; exit 0; }

code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 "https://pay.$PLATFORM_HOST/")
case $code in [234]*) ;; *) die "https://pay.$PLATFORM_HOST/ answers ${code:-nothing}; deploy it first";; esac
call PATCH "/v1/notifications/webhooks/$ID" "$(jq -nc --arg u "$WANT" '[{op: "replace", path: "/url", value: $u}]')" >/dev/null
echo "Replaced the URL. Now:"
call GET "/v1/notifications/webhooks/$ID" | show
