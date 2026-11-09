#!/usr/bin/env bash
# Builds the website and publishes it as a new release on the server, then
# switches /var/www/<domain>/current to it and keeps the last five releases.
#
#   TRAILSCOPE_SITE_HOST=ubuntu@203.0.113.10 website/deploy/deploy.sh
#
# Run setup-server.sh once before the first deploy.
set -euo pipefail

HOST=${TRAILSCOPE_SITE_HOST:?set TRAILSCOPE_SITE_HOST, e.g. ubuntu@203.0.113.10}
SITE=$(cd "$(dirname "$0")/.." && pwd)
DOMAIN=$(tr -d '[:space:]' < "$SITE/domain.txt")
ROOT=/var/www/$DOMAIN
RELEASE=$(date -u +%Y%m%d%H%M%S)

(cd "$SITE" && pnpm build)

rsync -az --delete "$SITE/dist/" "$HOST:$ROOT/releases/$RELEASE/"
ssh "$HOST" bash -s -- "$ROOT" "$RELEASE" <<'REMOTE'
set -euo pipefail
ROOT=$1
RELEASE=$2
cd "$ROOT"
ln -sfn "releases/$RELEASE" current.next
mv -Tf current.next current
ls -1 releases | sort | head -n -5 | while read -r old; do rm -rf "releases/$old"; done
echo "live: $ROOT/releases/$RELEASE"
REMOTE
