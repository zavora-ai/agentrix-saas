#!/bin/sh
# room-probe (test bench only): exercises a sealed room for the runner's live checks
# (S1-01, S1-03, gauntlet attack 9). Reads /in/job.json {"mode": ..., "url": ...}.
#   fetch   GET url through the job's proxy into /out/page.html
#   escape  try every way out; each attempt's outcome goes to /out/escape.txt (all should fail)
#   oom     allocate far past the memory cap
#   bomb    fork far past the process cap
set -u
note() { printf '{"event":"note","text":"%s"}\n' "$1"; }
mode=$(jq -r '.mode // "fetch"' /in/job.json)
url=$(jq -r '.url // empty' /in/job.json)
case "$mode" in
fetch)
  note "fetching $url through the proxy"
  code=$(curl -sS -o /out/page.html -w '%{http_code}' --max-time 20 "$url") || true
  note "status $code, $(wc -c </out/page.html 2>/dev/null || echo 0) bytes"
  [ "$code" = 200 ] ;;
escape)
  try() { # name, command...
    name=$1; shift
    if out=$("$@" 2>&1); then r="REACHED"; else r="blocked"; fi
    printf '%s\t%s\t%s\n' "$name" "$r" "$(printf '%s' "$out" | head -c 160 | tr '\n\t' '  ')" >>/out/escape.txt
    note "$name: $r"
  }
  : >/out/escape.txt
  try off-allowlist      curl -sS --max-time 8 -o /dev/null https://example.org/
  try metadata-proxy     curl -sS --max-time 8 -o /dev/null http://169.254.169.254/v1.json
  try metadata-direct    curl -sS --noproxy '*' --max-time 8 -o /dev/null http://169.254.169.254/v1.json
  try internet-direct    curl -sS --noproxy '*' --max-time 8 -o /dev/null http://1.1.1.1/
  try loopback-proxy     curl -sS --max-time 8 -o /dev/null http://127.0.0.1/console
  try host-runner-port   curl -sS --noproxy '*' --max-time 8 -o /dev/null http://172.30.0.1:7443/healthz
  try host-ssh           sh -c 'echo | nc -w 5 172.30.0.1 22'
  try other-room-proxy   curl -sS --max-time 8 -o /dev/null -x http://172.30.0.1:3191 https://example.org/
  try dns                nslookup example.com
  try vpc-control        curl -sS --noproxy '*' --max-time 8 -o /dev/null http://10.42.0.3/
  grep -c REACHED /out/escape.txt | sed 's/^/reached: /' >/out/summary.txt
  true ;;
oom)
  note "allocating 8 GiB"
  head -c 8589934592 /dev/zero | tail ;;
bomb)
  note "forking 20000 sleepers"
  i=0; while [ $i -lt 20000 ]; do sleep 300 & i=$((i + 1)); done; wait ;;
*) note "unknown mode $mode"; exit 2 ;;
esac
