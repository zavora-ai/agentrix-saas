#!/usr/bin/env bash
# The stopgap deploy (S2-10) became deploy/deploy.sh (S1-12); its units and settings moved to
# deploy/systemd/ and deploy/env/. This forwards, so older notes keep working.
exec "$(dirname "${BASH_SOURCE[0]}")/../deploy.sh" "$@"
