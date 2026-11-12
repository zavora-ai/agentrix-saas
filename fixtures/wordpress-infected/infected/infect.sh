#!/bin/sh
# Applies the `infected` overlay to a seeded Kilele Bakery (Zavora's fixture), in the `cli`
# container: plants the gauntlet's attacks (docs/04-vultr-agent-rush.md §3; attacks.json).
# Every payload is inert outside a sealed room; see README.md.
# Adapted from adk-rust examples/website/fixtures/wordpress/infected/infect.sh.
set -eu
cd /var/www/html
I=/fixture/infected
if [ "$(wp option get kl_infected 2>/dev/null || true)" = "1" ]; then
  echo "already infected"; exit 0
fi
# Files: mu-plugins (1, 5, 8), the hacked theme footer (7) and build hook (9), uploads (6).
cp "$I"/mu-plugins/*.php wp-content/mu-plugins/
cp "$I/theme/footer.php" wp-content/themes/kilele/footer.php
cp "$I/theme/build.sh" wp-content/themes/kilele/build.sh
mkdir -p wp-content/uploads/2026/10
cp "$I/uploads/2026/10/cache.php" "$I/uploads/2026/10/photo.jpg" wp-content/uploads/2026/10/
# Content: the offer link (1), the loopback link (2), hidden text for AI agents (3), spam (4).
wp eval-file "$I/infect.php" "${FIXTURE_SIZE:-dev}"
wp option update kl_infected 1 >/dev/null
echo "infected: attacks 1-9 planted (10 is the visitor prompt in visitor-prompt.txt)"
