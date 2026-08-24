#!/bin/bash
set -euo pipefail

APP="${1:?app_bundle_required}"
DMG="${2:?dmg_output_required}"

if [ ! -d "$APP" ]; then
  echo "ERROR=macos_app_bundle_missing" >&2
  exit 1
fi

EXE="$APP/Contents/MacOS/edgeswarm-unified-node"

if [ ! -f "$EXE" ]; then
  echo "ERROR=macos_app_runtime_missing" >&2
  exit 1
fi

EXPECTED="$(shasum -a 256 "$EXE" | awk '{print $1}')"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/edgeswarm-dmg-stage.XXXXXX")"
MOUNT="$(mktemp -d "${TMPDIR:-/tmp}/edgeswarm-dmg-mount.XXXXXX")"
MOUNTED=0

cleanup() {
  if [ "$MOUNTED" -eq 1 ]; then
    hdiutil detach "$MOUNT" >/dev/null 2>&1 || true
  fi
  rm -rf "$STAGE" "$MOUNT"
}
trap cleanup EXIT

mkdir -p "$(dirname "$DMG")"
rm -f "$DMG"

ditto "$APP" "$STAGE/EdgeSwarm Node.app"
ln -s /Applications "$STAGE/Applications"

hdiutil create \
  -volname "EdgeSwarm Node" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  "$DMG" >/dev/null

echo "NATIVE_DMG_CREATED=PASS"
printf 'DMG_SHA256='
shasum -a 256 "$DMG" | awk '{print $1}'
stat -f 'DMG_BYTES=%z' "$DMG"

hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT" "$DMG" >/dev/null
MOUNTED=1

PAYLOAD="$MOUNT/EdgeSwarm Node.app/Contents/MacOS/edgeswarm-unified-node"
ACTUAL="$(shasum -a 256 "$PAYLOAD" | awk '{print $1}')"

echo "DMG_PAYLOAD_SHA256=$ACTUAL"

if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "ERROR=dmg_payload_runtime_mismatch" >&2
  exit 1
fi

echo "DMG_PAYLOAD_RUNTIME_MATCH=PASS"

hdiutil detach "$MOUNT" >/dev/null
MOUNTED=0

echo "MACOS_NATIVE_DMG_PACKAGING=PASS"
