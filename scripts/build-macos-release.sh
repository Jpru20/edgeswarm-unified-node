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

npm run tauri build -- --bundles app

APP="$TARGET/release/bundle/macos/EdgeSwarm Node.app"
APP_EXE="$APP/Contents/MacOS/edgeswarm-unified-node"
RAW_EXE="$TARGET/release/edgeswarm-unified-node"

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
printf 'CANONICAL_RUNTIME_SHA256='
shasum -a 256 "$APP_EXE" | awk '{print $1}'

if [ -f "$RAW_EXE" ]; then
  printf 'PRE_BUNDLE_RUNTIME_SHA256='
  shasum -a 256 "$RAW_EXE" | awk '{print $1}'
fi

DMG="$TARGET/release/bundle/dmg/EdgeSwarm-Node_1.5.15_arm64.dmg"
"$ROOT/scripts/package-macos-dmg.sh" "$APP" "$DMG"

echo "MACOS_RELEASE_BUILD_COMPLETE=PASS"
