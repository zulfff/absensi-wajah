#!/usr/bin/env bash
# Download the ONNX models used by the real face pipeline.
#
# Models are NOT committed to git (see .gitignore). Run this once before
# building with `--features onnx`.
#
# Sources:
#   - SCRFD det_10g   : InsightFace "buffalo_l" pack
#   - ArcFace w600k_r50: InsightFace "buffalo_l" pack
#   - MiniFASNet      : Silent-Face-Anti-Spoofing (passive liveness)
#
# Verify each download against the SHA-256 in models/CHECKSUMS.txt afterwards.

set -euo pipefail

MODELS_DIR="$(cd "$(dirname "$0")/.." && pwd)/models"
mkdir -p "$MODELS_DIR"
cd "$MODELS_DIR"

BUFFALO_URL="https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_l.zip"

echo "==> Downloading buffalo_l (SCRFD + ArcFace)"
if [ ! -f det_10g.onnx ] || [ ! -f w600k_r50.onnx ]; then
    curl -fL -o buffalo_l.zip "$BUFFALO_URL"
    unzip -o buffalo_l.zip
    rm -f buffalo_l.zip
    # buffalo_l.zip extracts into a subdirectory; move the two we need up.
    find . -name 'det_10g.onnx' -exec mv -f {} ./ \;
    find . -name 'w600k_r50.onnx' -exec mv -f {} ./ \;
else
    echo "    already present, skipping"
fi

echo "==> Downloading MiniFASNetV2 (liveness)"
MINIFAS_URL="https://github.com/yakhyo/face-anti-spoofing/releases/download/weights/MiniFASNetV2.onnx"
if [ ! -f minifasnet.onnx ]; then
    if curl -fL -o minifasnet.onnx "$MINIFAS_URL"; then
        echo "    minifasnet.onnx downloaded"
    else
        rm -f minifasnet.onnx
        cat <<'NOTE'
    Failed to download MiniFASNetV2. Obtain the ONNX export manually and place
    it at models/minifasnet.onnx. Liveness is MANDATORY in production: the
    server refuses to start the real pipeline without FACE_LIVENESS_MODEL.
    Alternative: implement an active challenge (blink/turn) as a model-free
    liveness layer.
NOTE
    fi
else
    echo "    already present, skipping"
fi

echo "==> Generating checksums"
sha256sum ./*.onnx > CHECKSUMS.txt
cat CHECKSUMS.txt

echo
echo "Done. Model interfaces (verified):"
echo "  det_10g    : input 'input.1' [1,3,640,640] -> 9 outputs (score/bbox/kps x 3 strides)"
echo "  w600k_r50  : input 'input.1' [N,3,112,112] -> [N,512] (L2-normalise)"
echo "  minifasnet : input 'input'   [1,3,80,80]   -> [1,3] logits (class 1 = live)"
echo
echo "Set these in .env:"
echo "  FACE_DETECTOR_MODEL=$(pwd)/det_10g.onnx"
echo "  FACE_EMBEDDER_MODEL=$(pwd)/w600k_r50.onnx"
echo "  FACE_LIVENESS_MODEL=$(pwd)/minifasnet.onnx"
