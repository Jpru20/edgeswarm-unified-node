#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(awk -F'"' '/^version = "/ {print $2; exit}' src-tauri/Cargo.toml)"
HOST_ARCH="$(uname -m)"

case "$HOST_ARCH" in
    x86_64)
        ARCH="x64"
        DEB_ARCH="amd64"
        RPM_ARCH="x86_64"
        ;;
    aarch64|arm64)
        ARCH="arm64"
        DEB_ARCH="arm64"
        RPM_ARCH="aarch64"
        ;;
    *)
        echo "Unsupported Linux architecture: $HOST_ARCH"
        exit 1
        ;;
esac

NAME="EdgeSwarm_Node_Linux_${ARCH}_v${VERSION}"
OUT="$ROOT/release/linux/${VERSION}/${ARCH}"
BUILD_TARGET="${EDGESWARM_LINUX_BUILD_TARGET_DIR:-/tmp/edgeswarm-unified-node-build-${ARCH}-${VERSION}}"

rm -rf "$OUT" "$BUILD_TARGET"
mkdir -p "$OUT" "$BUILD_TARGET"

echo "VERSION=$VERSION"
echo "ARCH=$ARCH"
echo "DEB_ARCH=$DEB_ARCH"
echo "RPM_ARCH=$RPM_ARCH"

npm run build

(
    cd src-tauri
    CARGO_TARGET_DIR="$BUILD_TARGET" \
        cargo build --release \
        --bin edgeswarm-unified-node \
        --bin edgeswarm-node-headless
)

GUI="$BUILD_TARGET/release/edgeswarm-unified-node"
HEADLESS="$BUILD_TARGET/release/edgeswarm-node-headless"

test -x "$GUI"
test -x "$HEADLESS"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PAYLOAD="$TMP/payload"

install -d "$PAYLOAD/usr/lib/edgeswarm-node"
install -m 0755 "$GUI" "$PAYLOAD/usr/lib/edgeswarm-node/edgeswarm-unified-node"
install -m 0755 "$HEADLESS" "$PAYLOAD/usr/lib/edgeswarm-node/edgeswarm-node-headless"

install -d "$PAYLOAD/usr/bin"
ln -s ../lib/edgeswarm-node/edgeswarm-unified-node "$PAYLOAD/usr/bin/edgeswarm-node"
ln -s ../lib/edgeswarm-node/edgeswarm-node-headless "$PAYLOAD/usr/bin/edgeswarm-node-headless"

install -d "$PAYLOAD/usr/share/applications"
install -m 0644 packaging/linux/edgeswarm-node.desktop \
    "$PAYLOAD/usr/share/applications/com.edgeswarm.node.desktop"

install -d "$PAYLOAD/usr/share/icons/hicolor/128x128/apps"
install -m 0644 src-tauri/icons/128x128.png \
    "$PAYLOAD/usr/share/icons/hicolor/128x128/apps/edgeswarm-node.png"

install -d "$PAYLOAD/usr/lib/systemd/system"
install -m 0644 packaging/linux/edgeswarm-node-headless@.service \
    "$PAYLOAD/usr/lib/systemd/system/edgeswarm-node-headless@.service"

install -d "$PAYLOAD/usr/share/doc/edgeswarm-node"
install -m 0644 packaging/linux/node.env.example \
    "$PAYLOAD/usr/share/doc/edgeswarm-node/node.env.example"
install -m 0644 packaging/linux/README-headless.txt \
    "$PAYLOAD/usr/share/doc/edgeswarm-node/README-headless.txt"

echo
echo "=== TAR.GZ ==="

TARROOT="$TMP/$NAME"
mkdir -p "$TARROOT/bin" "$TARROOT/share"

printf '%s\n' "$ARCH" > "$TARROOT/ARCH"
printf '%s\n' "$VERSION" > "$TARROOT/VERSION"

install -m 0755 "$GUI" "$TARROOT/bin/edgeswarm-unified-node"
install -m 0755 "$HEADLESS" "$TARROOT/bin/edgeswarm-node-headless"
install -m 0755 packaging/linux/install-tar.sh "$TARROOT/install.sh"

install -m 0644 packaging/linux/edgeswarm-node.desktop "$TARROOT/share/edgeswarm-node.desktop"
install -m 0644 packaging/linux/edgeswarm-node-headless@.service "$TARROOT/share/edgeswarm-node-headless@.service"
install -m 0644 packaging/linux/node.env.example "$TARROOT/share/node.env.example"
install -m 0644 packaging/linux/README-headless.txt "$TARROOT/share/README-headless.txt"
install -m 0644 src-tauri/icons/128x128.png "$TARROOT/share/edgeswarm-node.png"

(
    cd "$TMP"
    tar -czf "$OUT/${NAME}.tar.gz" "$NAME"
)

echo
echo "=== DEB ==="

DEBROOT="$TMP/deb"
cp -a "$PAYLOAD" "$DEBROOT"
mkdir -p "$DEBROOT/DEBIAN"

DPKG_WORK="$TMP/dpkg-work"
mkdir -p "$DPKG_WORK/debian/edgeswarm-node/usr/lib/edgeswarm-node"

cp "$GUI" \
    "$DPKG_WORK/debian/edgeswarm-node/usr/lib/edgeswarm-node/edgeswarm-unified-node"

cp "$HEADLESS" \
    "$DPKG_WORK/debian/edgeswarm-node/usr/lib/edgeswarm-node/edgeswarm-node-headless"

cat > "$DPKG_WORK/debian/control" <<CONTROL
Source: edgeswarm-node
Section: utils
Priority: optional
Maintainer: EdgeSwarm <hello@joinswarm.io>
Standards-Version: 4.6.0

Package: edgeswarm-node
Architecture: $DEB_ARCH
Description: EdgeSwarm unified provider node
CONTROL

if ! SHLIB_OUTPUT="$(
    cd "$DPKG_WORK" &&
    dpkg-shlibdeps -O \
        -e"debian/edgeswarm-node/usr/lib/edgeswarm-node/edgeswarm-unified-node" \
        -e"debian/edgeswarm-node/usr/lib/edgeswarm-node/edgeswarm-node-headless" \
        2>"$TMP/dpkg-shlibdeps.log"
)"; then
    cat "$TMP/dpkg-shlibdeps.log"
    echo "ERROR: dpkg-shlibdeps failed."
    exit 1
fi

DEB_DEPENDS="$(
    printf '%s\n' "$SHLIB_OUTPUT" |
    sed -n 's/^shlibs:Depends=//p' |
    head -n 1
)"

if [[ -z "$DEB_DEPENDS" ]]; then
    cat "$TMP/dpkg-shlibdeps.log"
    echo "ERROR: Debian dependencies were not generated."
    exit 1
fi

echo "DEB_DEPENDS=$DEB_DEPENDS"

cat > "$DEBROOT/DEBIAN/control" <<CONTROL
Package: edgeswarm-node
Version: $VERSION
Section: utils
Priority: optional
Architecture: $DEB_ARCH
Maintainer: EdgeSwarm <hello@joinswarm.io>
Depends: $DEB_DEPENDS
Description: EdgeSwarm unified provider node
 Unified graphical desktop and headless/server EdgeSwarm node.
CONTROL

cat > "$DEBROOT/DEBIAN/postinst" <<'POST'
#!/bin/sh
systemctl daemon-reload >/dev/null 2>&1 || true
exit 0
POST

cat > "$DEBROOT/DEBIAN/postrm" <<'POST'
#!/bin/sh
systemctl daemon-reload >/dev/null 2>&1 || true
exit 0
POST

chmod 0755 "$DEBROOT/DEBIAN/postinst" "$DEBROOT/DEBIAN/postrm"

dpkg-deb --build \
    --root-owner-group \
    "$DEBROOT" \
    "$OUT/${NAME}.deb"

echo
echo "=== RPM ==="

if command -v rpmbuild >/dev/null 2>&1; then
    TOP="$TMP/rpmbuild"
    mkdir -p "$TOP"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

    (
        cd "$PAYLOAD"
        tar -czf "$TOP/SOURCES/edgeswarm-node-payload.tar.gz" .
    )

    cat > "$TOP/SPECS/edgeswarm-node.spec" <<SPEC
Name: edgeswarm-node
Version: $VERSION
Release: 1
Summary: EdgeSwarm unified provider node
License: Proprietary
BuildArch: $RPM_ARCH
Source0: edgeswarm-node-payload.tar.gz

%description
Unified graphical desktop and headless/server EdgeSwarm provider node.

%prep

%build

%install
mkdir -p %{buildroot}
tar -xzf %{SOURCE0} -C %{buildroot}

%files
/usr/lib/edgeswarm-node
/usr/bin/edgeswarm-node
/usr/bin/edgeswarm-node-headless
/usr/share/applications/com.edgeswarm.node.desktop
/usr/share/icons/hicolor/128x128/apps/edgeswarm-node.png
/usr/lib/systemd/system/edgeswarm-node-headless@.service
/usr/share/doc/edgeswarm-node

%post
/bin/systemctl daemon-reload >/dev/null 2>&1 || :

%postun
/bin/systemctl daemon-reload >/dev/null 2>&1 || :

%changelog
* Mon Aug 24 2026 EdgeSwarm <hello@joinswarm.io> - $VERSION-1
- Unified Linux graphical and headless package
SPEC

    rpmbuild -bb \
        --define "_topdir $TOP" \
        "$TOP/SPECS/edgeswarm-node.spec"

    RPM_FILE="$(find "$TOP/RPMS" -type f -name '*.rpm' -print -quit)"
    test -n "$RPM_FILE"
    cp "$RPM_FILE" "$OUT/${NAME}.rpm"

    echo "RPM_BUILD=PASS"
else
    echo "RPM_BUILD=SKIPPED_RPMBUILD_NOT_INSTALLED"
fi

echo
echo "=== RELEASE HASHES ==="

(
    cd "$OUT"
    sha256sum "$NAME".tar.gz "$NAME".deb > SHA256SUMS.txt

    if [[ -f "$NAME.rpm" ]]; then
        sha256sum "$NAME.rpm" >> SHA256SUMS.txt
    fi

    cat SHA256SUMS.txt
)

echo
echo "=== PACKAGE CONTENT CHECK ==="

dpkg-deb -f "$OUT/${NAME}.deb" Package Version Architecture

dpkg-deb -c "$OUT/${NAME}.deb" |
    grep -E 'edgeswarm-unified-node|edgeswarm-node-headless|\.desktop|systemd'

tar -tzf "$OUT/${NAME}.tar.gz" |
    grep -E 'edgeswarm-unified-node|edgeswarm-node-headless|install\.sh'

echo
echo "LINUX_PACKAGE_BUILD_COMPLETE=true"
echo "OUTPUT_DIR=$OUT"
