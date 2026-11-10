#!/usr/bin/env bash
# Prepares a server for the website; safe to run again. Needs nginx, python3
# and sudo on the server (Ubuntu). Run from your machine:
#
#   TRAILSCOPE_SITE_HOST=ubuntu@203.0.113.10 website/deploy/setup-server.sh
#
# It creates /var/www/<domain>/releases, installs the waitlist service and the
# nginx site, and reloads nginx only if the whole nginx config still passes
# `nginx -t` (otherwise it puts the previous config back). Once the domain points
# at the server, add HTTPS with:
#
#   sudo certbot --nginx -d <domain> -d www.<domain>
set -euo pipefail

HOST=${TRAILSCOPE_SITE_HOST:?set TRAILSCOPE_SITE_HOST, e.g. ubuntu@203.0.113.10}
SITE=$(cd "$(dirname "$0")/.." && pwd)
DOMAIN=$(tr -d '[:space:]' < "$SITE/domain.txt")

staging=$(ssh "$HOST" mktemp -d)
scp -q "$SITE/deploy/nginx.conf" "$SITE/deploy/trailscope-waitlist.service" \
  "$SITE/waitlist/waitlist.py" "$HOST:$staging/"

ssh "$HOST" bash -s -- "$DOMAIN" "$staging" <<'REMOTE'
set -euo pipefail
DOMAIN=$1
STAGING=$2
trap 'rm -rf "$STAGING"' EXIT

sudo install -d -o "$USER" -g "$USER" "/var/www/$DOMAIN" "/var/www/$DOMAIN/releases"

sudo install -D -m 0644 "$STAGING/waitlist.py" /opt/trailscope-waitlist/waitlist.py
sed "s/__DOMAIN__/$DOMAIN/g" "$STAGING/trailscope-waitlist.service" | sudo tee /etc/systemd/system/trailscope-waitlist.service > /dev/null
sudo systemctl daemon-reload
sudo systemctl enable --quiet trailscope-waitlist
sudo systemctl restart trailscope-waitlist

conf=/etc/nginx/sites-available/$DOMAIN
link=/etc/nginx/sites-enabled/$DOMAIN
if sudo grep -qs "managed by Certbot" "$conf"; then
  echo "nginx: $conf already has HTTPS from certbot; left unchanged"
else
  backup=""
  if sudo test -f "$conf"; then
    backup=$(mktemp)
    sudo cp "$conf" "$backup"
  fi
  had_link=$(sudo test -L "$link" && echo yes || echo no)
  sed "s/__DOMAIN__/$DOMAIN/g" "$STAGING/nginx.conf" | sudo tee "$conf" > /dev/null
  sudo ln -sfn "$conf" "$link"
  if ! sudo nginx -t 2> "$STAGING/nginx-test.log"; then
    cat "$STAGING/nginx-test.log" >&2
    if [ -n "$backup" ]; then sudo cp "$backup" "$conf"; else sudo rm -f "$conf"; fi
    if [ "$had_link" = no ]; then sudo rm -f "$link"; fi
    echo "nginx: the new site failed nginx -t; restored the previous config" >&2
    exit 1
  fi
  sudo systemctl reload nginx
  echo "nginx: $DOMAIN installed and reloaded"
fi

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fs -o /dev/null http://127.0.0.1:8790/api/waitlist/health; then
    echo "waitlist: running"
    exit 0
  fi
  sleep 0.5
done
echo "waitlist: not answering; see: sudo journalctl -u trailscope-waitlist" >&2
exit 1
REMOTE
