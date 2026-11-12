#!/usr/bin/env bash
# Talks to a site's live agent door the way a customer's AI agent would (AWP manifest + MCP).
#   ./door.sh manifest            what the site lets agents do
#   ./door.sh search WORDS        MCP tools/call catalog.search
#   ./door.sh ask "QUESTION"      MCP tools/call ask_site
# DOOR defaults to Kilele Bakery's sample. Read-only calls; nothing is ordered or paid.
set -euo pipefail
SITE=${SITE:-https://kilele-bakery-sample.paypal.agentrix.website}
DOOR=$SITE/mcp
H=(-H 'content-type: application/json' -H 'accept: application/json, text/event-stream')
session() {
  curl -s -D - -o /dev/null "${H[@]}" "$DOOR" \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"agentrix-demo","version":"1"}}}' \
    | awk -F': ' 'tolower($1)=="mcp-session-id"{print $2}' | tr -d '\r'
}
call() { # NAME ARGS_JSON
  local sid; sid=$(session)
  curl -s "${H[@]}" -H "mcp-session-id: $sid" "$DOOR" -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' >/dev/null
  curl -s "${H[@]}" -H "mcp-session-id: $sid" "$DOOR" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
    | sed -n 's/^data: //p; /^{/p' | head -n 1 | jq '.result.structuredContent'
}
case ${1:-manifest} in
  manifest) curl -s "$SITE/.well-known/awp.json" | jq -r '"site: \(.site)   awp: \(.awp)   mcp: \(.mcp.url)", (.capabilities[] | "  \(.id)  ->  \(.effect)\(if .heldFor then "  (held for \(.heldFor))" else "" end)")' ;;
  search) call catalog.search "$(jq -cn --arg q "${*:2}" '{query: $q}')" | jq -r '.items[:4][] | "  \(.name)   \(.price)   \(.stock.words)"' ;;
  ask) call ask_site "$(jq -cn --arg q "${*:2}" '{question: $q}')" | jq '{answer, sources: [.sources[] | .label]}' ;;
  *) echo "usage: door.sh manifest | search WORDS | ask QUESTION" >&2; exit 2 ;;
esac
