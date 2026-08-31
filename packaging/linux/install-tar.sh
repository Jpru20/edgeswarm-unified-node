#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "$(id -u)" -ne 0 ]]; then
    echo "Run this installer with sudo."
    exit 1
fi

EXPECTED_ARCH="$(tr -d '[:space:]' < "$ROOT/ARCH")"

case "$(uname -m)" in
    x86_64) HOST_ARCH="x64" ;;
    aarch64|arm64) HOST_ARCH="arm64" ;;
    *)
        echo "Unsupported Linux architecture: $(uname -m)"
        exit 1
        ;;
esac

if [[ "$HOST_ARCH" != "$EXPECTED_ARCH" ]]; then
    echo "Architecture mismatch."
    echo "Package: $EXPECTED_ARCH"
    echo "Host:    $HOST_ARCH"
    exit 1
fi

MISSING="$(
    {
        ldd "$ROOT/bin/edgeswarm-node-headless"
        ldd "$ROOT/runtime/current/llama-server"
    } 2>&1 | grep 'not found' || true
)"

if [[ -n "$MISSING" ]]; then
    echo "Required Linux shared libraries are missing:"
    echo "$MISSING"
    exit 1
fi

install -d -m 0755 /usr/lib/edgeswarm-node
install -m 0755 \
    "$ROOT/bin/edgeswarm-node-headless" \
    /usr/lib/edgeswarm-node/edgeswarm-node-headless

install -m 0755 \
    "$ROOT/bin/edgeswarm-node-setup" \
    /usr/lib/edgeswarm-node/edgeswarm-node-setup

install -d -m 0755 /usr/lib/edgeswarm-node/runtime/current
cp -a --no-preserve=ownership "$ROOT/runtime/current/." \
    /usr/lib/edgeswarm-node/runtime/current/

ln -sfn \
    /usr/lib/edgeswarm-node/edgeswarm-node-headless \
    /usr/bin/edgeswarm-node

ln -sfn \
    /usr/lib/edgeswarm-node/edgeswarm-node-headless \
    /usr/bin/edgeswarm-node-headless

ln -sfn \
    /usr/lib/edgeswarm-node/edgeswarm-node-setup \
    /usr/bin/edgeswarm-node-setup

ln -sfn \
    /usr/lib/edgeswarm-node/edgeswarm-node-setup \
    /usr/bin/edgeswarm-node-status

install -d -m 0755 /usr/lib/systemd/system
install -m 0644 \
    "$ROOT/share/edgeswarm-node-headless@.service" \
    /usr/lib/systemd/system/edgeswarm-node-headless@.service

install -d -m 0755 /usr/share/doc/edgeswarm-node
install -m 0644 \
    "$ROOT/share/node.env.example" \
    /usr/share/doc/edgeswarm-node/node.env.example
install -m 0644 \
    "$ROOT/share/README-headless.txt" \
    /usr/share/doc/edgeswarm-node/README-headless.txt

systemctl daemon-reload 2>/dev/null || true

echo
echo "EdgeSwarm Headless Node installed."
echo "Version: $(cat "$ROOT/VERSION")"
echo "Architecture: $EXPECTED_ARCH"
echo
echo "Command:"
echo "  edgeswarm-node"
echo
echo "The systemd service is NOT enabled automatically."
echo "See:"
echo "  /usr/share/doc/edgeswarm-node/README-headless.txt"
