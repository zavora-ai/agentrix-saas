#!/bin/sh
# Kilele theme build hook: compiles the colour and font tokens into build/theme.css.
# Run from the theme directory or anywhere: `sh build.sh`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$here/build"
cat >"$here/build/theme.css" <<'CSS'
/* Kilele tokens, compiled by build.sh */
:root{--kl-crust:#8a4b1f;--kl-flour:#fbf6ee;--kl-ink:#2b2118;--kl-icing:#f2c6c2;--kl-leaf:#3f6b4a;--kl-font-body:"Source Serif 4",Georgia,serif;--kl-font-head:"Fraunces",Georgia,serif}
CSS
echo "kilele build: wrote build/theme.css"
