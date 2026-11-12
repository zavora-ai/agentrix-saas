#!/bin/sh
# Writes the Move's replay and fallback inputs for the running fixture into exports/<size>/:
#   export.xml        WXR export (R4.1, the R4.4 fallback)
#   dump.sql.gz       MariaDB dump (room-replay's `dump`)
#   wp-content.tar.gz mu-plugins, themes/kilele and uploads, under wp-content/ (`wp_content`)
#   manifest.json     size, origin, counts and the sha256 of each file
# Run after ./up.sh <size>. EXPORT_DIR overrides the folder (for example exports/live, taken
# from the deployed fixture). The exports carry the planted attacks: never import them
# anywhere but a sealed room.
set -eu
cd "$(dirname "$0")"
SIZE=$(docker compose run --rm --no-deps -T cli -c 'wp option get kl_fixture_size' 2>/dev/null | tr -d '\r\n')
case $SIZE in dev|demo) ;; *) echo "start the fixture first: ./up.sh dev|demo" >&2; exit 1 ;; esac
ORIGIN=$(docker compose run --rm --no-deps -T cli -c 'wp option get home' 2>/dev/null | tr -d '\r\n')
OUT=${EXPORT_DIR:-exports/$SIZE}
mkdir -p "$OUT"
docker compose run --rm --no-deps -T cli -c 'wp export --stdout' 2>/dev/null >"$OUT/export.xml"
grep -q '<wp:wxr_version>' "$OUT/export.xml" || { echo "the WXR export failed" >&2; exit 1; }
docker compose exec -T db sh -c 'mariadb-dump -uwordpress -pfixture-only-not-a-secret --skip-dump-date --single-transaction wordpress' |
  gzip -n >"$OUT/dump.sql.gz"
docker compose exec -T wordpress tar --sort=name --owner=0 --group=0 --numeric-owner \
  --mtime='2026-11-07 00:00:00' -C /var/www/html -cf - \
  wp-content/index.php wp-content/mu-plugins wp-content/themes/kilele wp-content/uploads |
  gzip -n >"$OUT/wp-content.tar.gz"
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
PAGES=$(docker compose run --rm --no-deps -T cli -c 'wp post list --post_type=page --post_status=publish --format=count' 2>/dev/null | tr -d '\r\n')
POSTS=$(docker compose run --rm --no-deps -T cli -c 'wp post list --post_type=post --post_status=publish --format=count' 2>/dev/null | tr -d '\r\n')
INFECTED=$(docker compose run --rm --no-deps -T cli -c 'wp option get kl_infected' 2>/dev/null | tr -d '\r\n' || true)
cat >"$OUT/manifest.json" <<JSON
{
  "fixture": "Zavora's fixture: Kilele Bakery (fictional), deliberately infected",
  "size": "$SIZE",
  "public_origin": "$ORIGIN",
  "pages": $PAGES,
  "posts": $POSTS,
  "infected": $([ "$INFECTED" = 1 ] && echo true || echo false),
  "files": {
    "export.xml": "sha256:$(sha "$OUT/export.xml")",
    "dump.sql.gz": "sha256:$(sha "$OUT/dump.sql.gz")",
    "wp-content.tar.gz": "sha256:$(sha "$OUT/wp-content.tar.gz")"
  }
}
JSON
ls -la "$OUT"
cat "$OUT/manifest.json"
