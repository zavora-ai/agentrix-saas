#!/usr/bin/env bash
# Verifies an agent's badge from outside: resolves its DID from the site's own domain and runs
# every check the platform's resolver runs. Read-only.
#   ./verify.sh [DID]        default: Kilele Bakery's Cashier on the judged build
set -euo pipefail
PLATFORM=${PLATFORM:-https://paypal.agentrix.website}
DID=${1:-did:web:kilele-bakery-sample.paypal.agentrix.website:agents:cashier}
curl -s --max-time 30 -G "$PLATFORM/agents/verify" --data-urlencode "did=$DID" \
  | jq -r '(.checks[] | (if .ok then "✓ " else "✗ " end) + .check), (if .verified then "VERIFIED  " else "NOT VERIFIED  " end) + .did'
