#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="${EDGESWARM_LINUX_RELEASE_MANIFEST:-$ROOT/packaging/linux/release-manifest-v1.5.15.json}"

case "$(uname -m)" in
    x86_64|amd64)
        ARCH="x64"
        ;;
    aarch64|arm64)
        ARCH="arm64"
        ;;
    *)
        echo "Unsupported Linux architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

ID=""
ID_LIKE=""

if [[ -r /etc/os-release ]]; then
    . /etc/os-release
fi

DISTRO="$(
    printf '%s %s' \
        "${ID:-}" \
        "${ID_LIKE:-}" |
    tr '[:upper:]' '[:lower:]'
)"

if [[ -n "${EDGESWARM_PACKAGE_TYPE:-}" ]]; then
    PACKAGE_TYPE="$EDGESWARM_PACKAGE_TYPE"
elif [[ "$DISTRO" =~ debian|ubuntu|linuxmint|pop|elementary|zorin ]]; then
    PACKAGE_TYPE="deb"
elif [[ "$DISTRO" =~ fedora|rhel|centos|rocky|almalinux|oracle|amzn|suse|opensuse|sles ]]; then
    PACKAGE_TYPE="rpm"
else
    PACKAGE_TYPE="tar.gz"
fi

case "$PACKAGE_TYPE" in
    deb|rpm|tar.gz) ;;
    *)
        echo "Unsupported package type: $PACKAGE_TYPE" >&2
        exit 1
        ;;
esac

python3 - "$MANIFEST" "$ARCH" "$PACKAGE_TYPE" <<'PY'
import json
import sys

manifest_path, arch, package_type = sys.argv[1:4]

with open(manifest_path, "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

package = manifest["packages"][arch][package_type]
runtime_sha = manifest["packages"][arch]["runtimeSha256"]

print("PLATFORM=linux")
print("ARCHITECTURE=" + arch)
print("PACKAGE_TYPE=" + package_type)
print("VERSION=" + manifest["version"])
print("FILENAME=" + package["filename"])
print("PACKAGE_SHA256=" + package["sha256"])
print("RUNTIME_SHA256=" + runtime_sha)
print("DOWNLOAD_URL=" + package["downloadUrl"])
PY
