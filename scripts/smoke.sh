#!/usr/bin/env bash
# End-to-end smoke test against a running server (plan Section 14, step 3).
#
# Prerequisites:
#   - server running with USE_DETERMINISTIC_PIPELINE=true
#   - a JPEG frame at FRAME_FILE (default /tmp/frame.jpg)
#
# It exercises: login -> create student -> grant consent -> enroll -> activate
# -> create device -> kiosk WebSocket accept, and prints each step's result.

set -euo pipefail

BASE="${BASE:-http://127.0.0.1:8080}"
FRAME_FILE="${FRAME_FILE:-/tmp/frame.jpg}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASS="${ADMIN_PASS:-admin}"
NIS="${NIS:-9001}"

say() { printf '\n== %s\n' "$*"; }
jqp() { python3 -c "import sys,json;d=json.load(sys.stdin);print(d$1)"; }

if [ ! -f "$FRAME_FILE" ]; then
    echo "no frame at $FRAME_FILE; generating one with ImageMagick..."
    convert -size 640x480 plasma:fractal -blur 0x1 "$FRAME_FILE"
fi

# Each smoke run must use a *distinct* frame, or the anti-duplicate check
# correctly blocks the new enrollment against a previous student who was
# enrolled from the same image. Generate unique, high-variance content so the
# deterministic embedder (used in dev; it is NOT discriminative like ArcFace)
# still separates students.
if [ -z "${NO_UNIQUE_FRAME:-}" ] && command -v convert >/dev/null; then
    FRAME_FILE="$(mktemp --suffix=.jpg)"
    SEED=$((RANDOM * 32768 + RANDOM))
    HUE=$((SEED % 360))
    convert -size 320x240 "xc:hsl($HUE,60%,45%)" \
        -seed "$SEED" +noise Gaussian -attenuate 1.2 -blur 0x0.6 \
        -normalize -quality 70 "$FRAME_FILE"
fi

say "login"
TOKEN=$(curl -sf -X POST "$BASE/api/auth/login" \
    -H 'content-type: application/json' \
    -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" | jqp "['token']")
echo "token acquired (${#TOKEN} chars)"

say "create student $NIS"
SID=$(curl -sf -X POST "$BASE/api/students" \
    -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
    -d "{\"nis\":\"$NIS\",\"nama\":\"Smoke Test\"}" | jqp "['id']")
echo "student $SID"

say "grant consent (direct DB — normally done in the admin UI)"
DOCKER="${DOCKER:-docker}"
DB_CONTAINER="${DB_CONTAINER:-docker-db-1}"
$DOCKER exec "$DB_CONTAINER" psql -U absensi -d absensi \
    -c "UPDATE students SET consent_granted=TRUE, consent_by='smoke' WHERE id='$SID';" >/dev/null

say "enroll: assess one frame"
IMG=$(base64 -w0 "$FRAME_FILE")
SMOKE_SID="$SID" SMOKE_TOKEN="$TOKEN" SMOKE_BASE="$BASE" SMOKE_IMG="$IMG" python3 - <<'PY'
import json, os, urllib.request
sid = os.environ["SMOKE_SID"]; token = os.environ["SMOKE_TOKEN"]
base = os.environ["SMOKE_BASE"]; img = os.environ["SMOKE_IMG"]
body = json.dumps({"image": img, "step": 0}).encode()
req = urllib.request.Request(f"{base}/api/students/{sid}/enroll/frame", data=body,
    headers={"authorization": f"Bearer {token}", "content-type": "application/json"})
print(urllib.request.urlopen(req).read().decode())
PY

say "enroll: commit 7 frames"
python3 - "$SID" "$TOKEN" "$BASE" "$FRAME_FILE" <<'PY'
import base64, json, sys, urllib.request
sid, token, base, frame = sys.argv[1:5]
with open(frame, "rb") as fh:
    b64 = base64.b64encode(fh.read()).decode()
body = json.dumps({"frames": [b64] * 7}).encode()
req = urllib.request.Request(f"{base}/api/students/{sid}/enroll/commit", data=body,
    headers={"authorization": f"Bearer {token}", "content-type": "application/json"})
print(urllib.request.urlopen(req).read().decode())
PY

say "activate"
curl -sf -X POST "$BASE/api/students/$SID/activate" -H "authorization: Bearer $TOKEN"
echo

say "create device"
DEVJSON=$(curl -sf -X POST "$BASE/api/devices" \
    -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
    -d '{"nama":"Smoke Kiosk","lokasi":"CI"}')
DEVTOKEN=$(echo "$DEVJSON" | jqp "['token']")
echo "device token acquired"

say "kiosk WebSocket accept"
ABSENSI_TEST_URL="$BASE" ABSENSI_TEST_DEVICE_TOKEN="$DEVTOKEN" \
ABSENSI_TEST_FRAME="$IMG" \
    cargo test -p absensi-server --test kiosk_flow -- --nocapture

echo
echo "smoke test passed"
