#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CONFIG="${1:-$ROOT/.env.release.local}"
TARGET="${2:-}"

if [ ! -f "$CONFIG" ]; then
  echo "ERROR=release_config_missing" >&2
  exit 1
fi

set -a
. "$CONFIG"
set +a

URL="${EDGESWARM_DEFAULT_SUPABASE_URL:-}"
KEY="${EDGESWARM_DEFAULT_SUPABASE_ANON_KEY:-}"

if [[ "$URL" != https://* ]]; then
  echo "ERROR=release_supabase_url_invalid" >&2
  exit 1
fi

if [ "${#KEY}" -lt 50 ]; then
  echo "ERROR=release_supabase_anon_key_invalid" >&2
  exit 1
fi

unset SUPABASE_URL
unset SUPABASE_ANON_KEY
unset EDGESWARM_SUPABASE_URL
unset EDGESWARM_SUPABASE_ANON_KEY

HEAD="$(git rev-parse HEAD)"
ORIGIN="$(git rev-parse origin/main)"

if [ "$HEAD" != "$ORIGIN" ]; then
  echo "ERROR=release_source_not_at_origin_main" >&2
  exit 1
fi

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "ERROR=release_worktree_not_clean" >&2
  exit 1
fi

if [ -z "$TARGET" ]; then
  SHORT="$(git rev-parse --short=7 HEAD)"
  STAMP="$(date +%Y%m%d-%H%M%S)"
  TARGET="$HOME/edgeswarm-macos-release-build-$SHORT-$STAMP"
fi

export CARGO_TARGET_DIR="$TARGET"

echo "RELEASE_CONFIG_VALID=PASS"
echo "SOURCE_COMMIT=$HEAD"
echo "TARGET_DIR=$TARGET"

# MACOS_BACKGROUND_HELPERS_V1
cargo build \
  --manifest-path "$ROOT/src-tauri/Cargo.toml" \
  --release \
  --no-default-features \
  --bin edgeswarm-node-headless \
  --bin edgeswarm-node-supervisor-macos \
  --bin edgeswarm-credential-broker-macos

echo "MACOS_BACKGROUND_HELPERS_BUILT=PASS"

npm run tauri build -- --bundles app

APP="$TARGET/release/bundle/macos/EdgeSwarm Node.app"
APP_EXE="$APP/Contents/MacOS/edgeswarm-unified-node"
RAW_EXE="$TARGET/release/edgeswarm-unified-node"

RAW_HEADLESS="$TARGET/release/edgeswarm-node-headless"
RAW_SUPERVISOR="$TARGET/release/edgeswarm-node-supervisor-macos"
RAW_BROKER="$TARGET/release/edgeswarm-credential-broker-macos"

APP_HEADLESS="$APP/Contents/MacOS/edgeswarm-node-headless"
APP_SUPERVISOR="$APP/Contents/MacOS/edgeswarm-node-supervisor-macos"
APP_BROKER="$APP/Contents/MacOS/edgeswarm-credential-broker-macos"

test -x "$RAW_HEADLESS" || {
  echo "ERROR=macos_headless_helper_missing" >&2
  exit 1
}

test -x "$RAW_SUPERVISOR" || {
  echo "ERROR=macos_supervisor_helper_missing" >&2
  exit 1
}

test -x "$RAW_BROKER" || {
  echo "ERROR=macos_credential_broker_missing" >&2
  exit 1
}

cp "$RAW_HEADLESS" "$APP_HEADLESS"
cp "$RAW_SUPERVISOR" "$APP_SUPERVISOR"
cp "$RAW_BROKER" "$APP_BROKER"

chmod +x   "$APP_HEADLESS"   "$APP_SUPERVISOR"   "$APP_BROKER"

echo "MACOS_HEADLESS_HELPER_PACKAGED=PASS"
echo "MACOS_SUPERVISOR_HELPER_PACKAGED=PASS"
echo "MACOS_CREDENTIAL_BROKER_PACKAGED=PASS"

if [ ! -f "$APP_EXE" ]; then
  echo "ERROR=macos_app_payload_missing" >&2
  exit 1
fi

LC_ALL=C grep -aFq -- "$URL" "$APP_EXE" || {
  echo "ERROR=app_payload_supabase_url_not_found" >&2
  exit 1
}

LC_ALL=C grep -aFq -- "$KEY" "$APP_EXE" || {
  echo "ERROR=app_payload_supabase_anon_key_not_found" >&2
  exit 1
}

echo "APP_PAYLOAD_CONFIG_VERIFIED=PASS"
echo "CANONICAL_RUNTIME_PATH=$APP_EXE"

# UNIFIED_BUNDLED_LLAMA_RUNTIME_V1
LLAMA_RUNTIME_SOURCE="${EDGESWARM_MACOS_LLAMA_RUNTIME_DIR:-$HOME/edgeswarm-runtime-build/release/macos-arm64/current}"
LLAMA_RUNTIME_EXPECTED_SHA="8a4c0a23355af2ba40c56c2d7b60a441c289fc8b33e2baeda1cc5ff2af126cce"
LLAMA_RUNTIME_DEST="$APP/Contents/MacOS/runtime/current"

test -x "$LLAMA_RUNTIME_SOURCE/llama-server" || {
    echo "ERROR=macos_llama_runtime_missing" >&2
    exit 1
}

ACTUAL_LLAMA_SHA="$(shasum -a 256 "$LLAMA_RUNTIME_SOURCE/llama-server" | awk "{print \$1}")"
test "$ACTUAL_LLAMA_SHA" = "$LLAMA_RUNTIME_EXPECTED_SHA" || {
    echo "ERROR=macos_llama_runtime_sha_mismatch" >&2
    echo "EXPECTED=$LLAMA_RUNTIME_EXPECTED_SHA" >&2
    echo "ACTUAL=$ACTUAL_LLAMA_SHA" >&2
    exit 1
}

rm -rf "$LLAMA_RUNTIME_DEST"
mkdir -p "$LLAMA_RUNTIME_DEST"
cp -a "$LLAMA_RUNTIME_SOURCE/." "$LLAMA_RUNTIME_DEST/"

find "$LLAMA_RUNTIME_DEST" -type f \
    \( -name "llama-server" -o -name "*.dylib" \) \
    -exec codesign --force --sign - --timestamp=none {} \;

echo "MACOS_LLAMA_RUNTIME_SHA256=$ACTUAL_LLAMA_SHA"
"$LLAMA_RUNTIME_DEST/llama-server" --version 2>&1 | head -3
echo "MACOS_BUNDLED_LLAMA_RUNTIME=PASS"

# Seal the completed beta app bundle before hashing/packaging.
# This is ad-hoc signing only; Developer ID/notarization remains future work.
APP_HELPER="$APP/Contents/MacOS/edgeswarm-node-headless"
APP_SUPERVISOR="$APP/Contents/MacOS/edgeswarm-node-supervisor-macos"
APP_BROKER="$APP/Contents/MacOS/edgeswarm-credential-broker-macos"

test -x "$APP_HELPER" || {
    echo "ERROR=macos_headless_payload_missing" >&2
    exit 1
}

test -x "$APP_SUPERVISOR" || {
    echo "ERROR=macos_supervisor_payload_missing" >&2
    exit 1
}

test -x "$APP_BROKER" || {
    echo "ERROR=macos_credential_broker_payload_missing" >&2
    exit 1
}

codesign --force --sign - --timestamp=none "$APP_HELPER"
codesign --force --sign - --timestamp=none "$APP_SUPERVISOR"
codesign --force --sign - --timestamp=none "$APP_BROKER"

codesign --verify --strict --verbose=2 "$APP_HELPER"
codesign --verify --strict --verbose=2 "$APP_SUPERVISOR"
codesign --verify --strict --verbose=2 "$APP_BROKER"

echo "MACOS_HEADLESS_HELPER_SIGNATURE=PASS"
echo "MACOS_SUPERVISOR_HELPER_SIGNATURE=PASS"
echo "MACOS_CREDENTIAL_BROKER_SIGNATURE=PASS"

codesign --force --sign - --timestamp=none "$APP"

codesign --verify --deep --strict --verbose=2 "$APP"

test -f "$APP/Contents/_CodeSignature/CodeResources"

echo "MACOS_APP_BUNDLE_SIGNATURE=PASS"

printf 'CANONICAL_RUNTIME_SHA256='
shasum -a 256 "$APP_EXE" | awk '{print $1}'

if [ -f "$RAW_EXE" ]; then
  printf 'PRE_BUNDLE_RUNTIME_SHA256='
  shasum -a 256 "$RAW_EXE" | awk '{print $1}'
fi

VERSION="$(awk -F'"' '/^version = "/ {print $2; exit}' src-tauri/Cargo.toml)"
DMG="$TARGET/release/bundle/dmg/EdgeSwarm-Node_${VERSION}_arm64.dmg"
"$ROOT/scripts/package-macos-dmg.sh" "$APP" "$DMG"

echo "MACOS_RELEASE_BUILD_COMPLETE=PASS"
