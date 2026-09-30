#!/usr/bin/env bash
# Deploy / update Absensi Wajah behind the existing FortressWAF Caddy.
#
# Prerequisites (already true on this host):
#   - deploy-caddy-1 owns :80/:443 and terminates TLS for *.tkjt3yapera.my.id
#   - a `fortress-net` Docker network exists
#   - DNS for panel.* and absensi.* resolves here, DNS-only (grey cloud)
#   - .env in the repo root has JWT_SECRET, ABSENSI_DB_PASSWORD,
#     BOOTSTRAP_ADMIN_PASSWORD
#
# Run:  ./scripts/deploy.sh            (build + up)
#       ./scripts/deploy.sh web        (rebuild just the web image)
#       ./scripts/deploy.sh logs       (follow logs)
#       ./scripts/deploy.sh down       (stop the absensi stack)

set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE="docker compose -f docker/docker-compose.deploy.yml --env-file .env"
# Docker needs sudo on this host; override with DOCKER="docker" if your user is
# in the docker group.
DOCKER="${DOCKER:-sudo -n docker}"

case "${1:-up}" in
  up)
    $DOCKER compose -f docker/docker-compose.deploy.yml --env-file .env up -d --build
    ;;
  web)
    $DOCKER compose -f docker/docker-compose.deploy.yml --env-file .env up -d --build absensi-web
    ;;
  server)
    $DOCKER compose -f docker/docker-compose.deploy.yml --env-file .env up -d --build absensi-server
    ;;
  logs)
    $DOCKER compose -f docker/docker-compose.deploy.yml logs -f "${2:-}"
    ;;
  down)
    $DOCKER compose -f docker/docker-compose.deploy.yml down
    ;;
  reload-caddy)
    $DOCKER exec deploy-caddy-1 caddy reload --config /etc/caddy/Caddyfile
    ;;
  status)
    $DOCKER ps --format '{{.Names}}\t{{.Status}}' | grep -E 'absensi' || true
    ;;
  *)
    echo "usage: $0 [up|web|server|logs|down|reload-caddy|status]" >&2
    exit 2
    ;;
esac
