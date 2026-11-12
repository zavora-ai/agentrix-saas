#!/usr/bin/env bash
# Renders the submission's deck PDF and cover image with headless Chrome.
#   specs/submission/render.sh deck    -> specs/submission/deck/agentrix-vultr-deck.pdf
#   specs/submission/render.sh cover   -> specs/submission/cover/cover.png (1920 x 1080)
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
PROFILE=$(mktemp -d)
trap 'rm -rf "$PROFILE"' EXIT
URL="file://$HERE/deck/deck.html"
case ${1:-deck} in
  deck)
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars --user-data-dir="$PROFILE" \
      --no-pdf-header-footer --virtual-time-budget=8000 --run-all-compositor-stages-before-draw \
      --print-to-pdf="$HERE/deck/agentrix-vultr-deck.pdf" "$URL" 2>/dev/null
    echo "$HERE/deck/agentrix-vultr-deck.pdf" ;;
  cover)
    mkdir -p "$HERE/cover"
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars --user-data-dir="$PROFILE" \
      --window-size=1920,1080 --virtual-time-budget=8000 --run-all-compositor-stages-before-draw \
      --screenshot="$HERE/cover/cover.png" "$URL?only=${2:-cover}" 2>/dev/null
    echo "$HERE/cover/cover.png" ;;
  *) echo "usage: render.sh deck|cover" >&2; exit 2 ;;
esac
