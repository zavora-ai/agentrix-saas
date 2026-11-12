#!/bin/sh
# room-fetch (S1-02; design §3.1; R1.2): fetches exactly the URLs control gives it, through the
# job's egress proxy, and records each answer. Control has already applied robots.txt and the
# crawl budget; the room fetches nothing else.
#
# Contract (Assess lane, assess-room-fetch-contract.md):
#   /in/job.json   {task:"fetch", urls:[...], user_agent, delay_ms, timeout_ms, max_bytes,
#                   max_redirects, screenshots}
#   /out/fetch.json {results:[{i, url, final_url, status|null, content_type, bytes, truncated,
#                   ms, body:"bodies/<i>"|null, error:null|"words"}]}, one per URL, in order
#   /out/bodies/<i> the raw body, cut at max_bytes
#   stdout         {"event":"note","text":"fetched <i> <status>|error"} after each URL
# Errors keep the words the report looks for: refused, dns/resolve, tls/certificate, timed out.
#
# /out/fetch.json always exists (R1.6): it is rewritten after every URL, so a room killed at its
# deadline still reports what it fetched, and on exit any URL not reached gets an error entry.
set -u
J=${ROOM_IN:-/in}/job.json
OUT=${ROOM_OUT:-/out}
mkdir -p "$OUT/bodies" /tmp/fetch
: >/tmp/fetch/results.jsonl

# Writes /out/fetch.json from the results so far, atomically.
manifest() {
  if jq -s '{results: .}' /tmp/fetch/results.jsonl >"$OUT/.fetch.json.tmp" 2>/dev/null; then
    mv -f "$OUT/.fetch.json.tmp" "$OUT/fetch.json"
  else
    printf '{"results":[]}\n' >"$OUT/fetch.json"
  fi
}

# On any exit: every URL not reached is reported, then the manifest is written.
finish() {
  total=$(jq '.urls | length' "$J" 2>/dev/null || echo 0)
  done_n=$(wc -l </tmp/fetch/results.jsonl | tr -d ' ')
  k=${done_n:-0}
  while [ "$k" -lt "${total:-0}" ]; do
    jq -cn --argjson i "$k" --arg url "$(jq -r --argjson i "$k" '.urls[$i]' "$J")" \
      '{i: $i, url: $url, status: null, bytes: 0, truncated: false, body: null,
        error: "the room stopped before fetching this"}' >>/tmp/fetch/results.jsonl
    k=$((k + 1))
  done
  manifest
}
trap finish EXIT
manifest
ua=$(jq -r '.user_agent // "Mozilla/5.0 (compatible; AgentrixBot/1.0; +https://agentrix.website/bot)"' "$J")
delay=$(jq -r '.delay_ms // 500' "$J")
timeout_ms=$(jq -r '.timeout_ms // 20000' "$J")
max_bytes=$(jq -r '.max_bytes // 5000000' "$J")
redirects=$(jq -r '.max_redirects // 5' "$J")
secs=$(( (timeout_ms + 999) / 1000 ))
n=$(jq '.urls | length' "$J")
i=0
while [ "$i" -lt "$n" ]; do
  url=$(jq -r --argjson i "$i" '.urls[$i]' "$J")
  body="$OUT/bodies/$i"
  start=$(date +%s%3N 2>/dev/null || date +%s000)
  # The full answer lands in /tmp (the proxy caps it at 25 MB per connection); /out keeps the
  # first max_bytes. curl's status line goes to its own file, so a long body cannot hide it.
  curl -sS -A "$ua" --max-time "$secs" --max-redirs "$redirects" -L --proto '=http,https' \
    --proto-redir '=http,https' -o /tmp/fetch/full \
    -w '%{http_code}\n%{url_effective}\n%{content_type}\n' "$url" \
    >/tmp/fetch/meta 2>/tmp/fetch/err
  code=$?
  end=$(date +%s%3N 2>/dev/null || date +%s000)
  status=$(sed -n 1p /tmp/fetch/meta)
  final=$(sed -n 2p /tmp/fetch/meta)
  ctype=$(sed -n 3p /tmp/fetch/meta)
  [ -n "$final" ] || final="$url"
  if [ -f /tmp/fetch/full ]; then head -c "$max_bytes" /tmp/fetch/full >"$body"; else : >"$body"; fi
  full_bytes=0
  [ -f /tmp/fetch/full ] && full_bytes=$(wc -c </tmp/fetch/full)
  rm -f /tmp/fetch/full
  bytes=$(wc -c <"$body")
  truncated=false
  [ "$full_bytes" -gt "$max_bytes" ] && truncated=true
  err=$(tr '\n' ' ' </tmp/fetch/err | head -c 300)
  words=null
  case "$status" in ''|000) status=null ;; esac
  if [ "$status" = null ]; then
    lower=$(printf '%s' "$err" | tr 'A-Z' 'a-z')
    case "$lower" in
      *"connect tunnel failed"*|*"response 403"*|*refused*) w="refused by the egress proxy ($(printf '%s' "$url" | sed -E 's#^[a-z]+://([^/:]+).*#\1#'))" ;;
      *"could not resolve"*|*resolve*) w="dns: could not resolve the host" ;;
      *ssl*|*tls*|*certificate*) w="tls: certificate or handshake failed" ;;
      *"timed out"*|*timeout*) w="timed out after ${secs}s" ;;
      *) w="fetch failed: ${err:-curl exit $code}" ;;
    esac
    words=$(jq -cn --arg w "$w" '$w')
    : >"$body"; bytes=0
  fi
  line=$(jq -cn --argjson i "$i" --arg url "$url" --arg final "$final" --argjson status "$status" \
    --arg ctype "$ctype" --argjson bytes "$bytes" --argjson truncated "$truncated" \
    --argjson ms "$(( end - start ))" --argjson error "$words" --arg body "bodies/$i" \
    '{i: $i, url: $url, final_url: $final, status: $status, content_type: $ctype, bytes: $bytes,
      truncated: $truncated, ms: $ms, body: (if $status == null then null else $body end), error: $error}' \
    2>/dev/null) || line=$(jq -cn --argjson i "$i" --arg url "$url" \
    '{i: $i, url: $url, status: null, bytes: 0, truncated: false, body: null,
      error: "the room could not record this answer"}')
  printf '%s\n' "$line" >>/tmp/fetch/results.jsonl
  manifest
  if [ "$status" = null ]; then
    printf '{"event":"note","text":"fetched %s error"}\n' "$i"
  else
    printf '{"event":"note","text":"fetched %s %s"}\n' "$i" "$status"
  fi
  i=$((i + 1))
  [ "$i" -lt "$n" ] && sleep "$(awk -v d="$delay" 'BEGIN { printf "%.3f", d / 1000 }')"
done
exit 0
