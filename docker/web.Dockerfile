# Build the admin + kiosk SPA and serve it with Caddy/nginx.
# Multi-stage: Node builds the static bundle, a tiny server serves it.

FROM node:22-slim AS build
WORKDIR /app
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM caddy:2-alpine AS runtime
# The Caddyfile in docker/ is mounted by compose; this image only needs the
# built assets at /srv/web (where the Caddyfile expects them).
COPY --from=build /app/dist /srv/web
EXPOSE 80
