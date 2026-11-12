#!/bin/sh
# Starts Zavora's fixture (Kilele Bakery, fictional): seeds it and plants the gauntlet's attacks.
#   ./up.sh [dev|demo] [--clean]
# dev = 5 pages for development Moves; demo = 50 pages for acceptance, the gauntlet and the demo.
# --clean seeds without the infected overlay (a parity baseline). Same script on Docker Desktop
# and on the fixture VM; on the VM set FIXTURE_ORIGIN=https://<fixture domain>.
# Env: FIXTURE_ORIGIN (default https://kilele-bakery.example), FIXTURE_PORT (default 8081).
# Both are remembered in .env (gitignored), so every later compose call keeps them; a container
# recreated without them would serve the default origin. `rm .env` to forget them.
set -eu
cd "$(dirname "$0")"
if [ -z "${FIXTURE_ORIGIN:-}" ] && [ -z "${FIXTURE_PORT:-}" ] && [ -f .env ]; then
  set -a; . ./.env; set +a
fi
export FIXTURE_ORIGIN=${FIXTURE_ORIGIN:-https://kilele-bakery.example}
export FIXTURE_PORT=${FIXTURE_PORT:-8081}
printf 'FIXTURE_ORIGIN=%s\nFIXTURE_PORT=%s\n' "$FIXTURE_ORIGIN" "$FIXTURE_PORT" >.env
SIZE=dev
CLEAN=0
for a in "$@"; do
  case $a in
    dev|demo) SIZE=$a ;;
    --clean) CLEAN=1 ;;
    *) echo "usage: ./up.sh [dev|demo] [--clean]" >&2; exit 2 ;;
  esac
done
export FIXTURE_SIZE=$SIZE
docker compose up -d --wait db wordpress
docker compose run --rm --no-deps -T cli /fixture/site/seed.sh
if [ "$CLEAN" = 0 ]; then
  docker compose run --rm --no-deps -T cli /fixture/infected/infect.sh
  ./check.sh
fi
echo "Zavora's fixture ($SIZE) on http://127.0.0.1:$FIXTURE_PORT, origin $FIXTURE_ORIGIN"
