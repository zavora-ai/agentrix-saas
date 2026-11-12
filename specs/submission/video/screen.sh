#!/usr/bin/env bash
# Records the Mac's main display while James drives the live Move (passkey steps only he can do).
#   specs/submission/video/screen.sh start NAME   -> clips/raw-NAME.mov, recording in the background
#   specs/submission/video/screen.sh stop         -> stops it cleanly (SIGINT finalises the file)
# Then crop to the browser window in beats.json ("crop": "w:h:x:y") or with ffmpeg.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PIDFILE="$HERE/clips/.screen.pid"
mkdir -p "$HERE/clips"
case ${1:-} in
  start)
    [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null && { echo "already recording"; exit 1; }
    free=$(df -g "$HERE" | awk 'NR==2 {print $4}')
    [ "$free" -ge 14 ] || { echo "only ${free} GB free; not recording"; exit 1; }
    out="$HERE/clips/raw-${2:-move}.mov"
    nohup screencapture -v -x "$out" >/dev/null 2>&1 &
    echo $! > "$PIDFILE"
    echo "recording $out (pid $(cat "$PIDFILE")) from $(TZ=Africa/Nairobi date +%H:%M:%S) EAT" ;;
  stop)
    pid=$(cat "$PIDFILE" 2>/dev/null || true)
    [ -n "$pid" ] && kill -INT "$pid" 2>/dev/null && sleep 3
    rm -f "$PIDFILE"
    ls -la "$HERE"/clips/raw-*.mov ;;
  *) echo "usage: screen.sh start NAME | stop" >&2; exit 2 ;;
esac
