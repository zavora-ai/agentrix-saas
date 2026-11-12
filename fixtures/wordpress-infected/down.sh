#!/bin/sh
# Stops Zavora's fixture and removes its containers, network and volumes.
#   ./down.sh [--images]   also removes the wordpress and mariadb images it pulled
set -eu
cd "$(dirname "$0")"
if [ "${1:-}" = "--images" ]; then
  docker compose --profile tools down -v --remove-orphans --rmi all
else
  docker compose --profile tools down -v --remove-orphans
fi
