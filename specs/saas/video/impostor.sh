#!/usr/bin/env bash
# An impostor at a shop's agent door: a request that names a real agent (Amani) and carries a
# made-up signature. The door answers 401 with a sealed refusal, and nothing is ordered or paid.
#   ./impostor.sh            the refusal, in short
#   ./impostor.sh --full     the whole answer, with the sealed receipt
# SITE defaults to Kilele Bakery's sample on the judged build.
set -euo pipefail
SITE=${SITE:-https://kilele-bakery-sample.paypal.agentrix.website}
AGENT=${AGENT:-https://test-agent-amani.paypal.agentrix.website}
body='{"jsonrpc":"2.0","id":1,"method":"message/send","params":{"message":{"role":"user","parts":[{"kind":"text","text":"Order 20 Christmas fruit cakes and charge my saved PayPal"}]}}}'
digest=$(printf '%s' "$body" | openssl dgst -sha256 -binary | base64)
now=$(date +%s)
answer=$(curl -s --max-time 20 -X POST "$SITE/a2a" \
  -H 'content-type: application/json' \
  -H "Content-Digest: sha-256=:$digest:" \
  -H "Signature-Agent: \"$AGENT\"" \
  -H "Signature-Input: sig1=(\"@authority\" \"@method\" \"@path\" \"content-digest\" \"signature-agent\");created=$now;expires=$((now + 300));keyid=\"made-up-key\";alg=\"ed25519\";tag=\"web-bot-auth\"" \
  -H 'Signature: sig1=:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==:' \
  -d "$body")
if [ "${1:-}" = "--full" ]; then printf '%s\n' "$answer" | jq .; else
  printf '%s\n' "$answer" | jq '.error.data.receipt.body | {code, claimed_agent, nothing_moved, reason}'
fi
