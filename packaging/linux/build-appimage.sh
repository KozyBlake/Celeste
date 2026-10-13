#!/usr/bin/env bash
#
# Packages an already-built Celeste binary as an AppImage.
#
# Run `cargo build --release` first; this script only packages. It expects
# to be run from anywhere and resolves paths from its own location.
#
# How portable the result is depends on the machine this runs on, not on
# anything here: the binary records the glibc it was linked against, and a
# system with an older glibc refuses to start it. Build on the oldest
# distribution you intend to support. The release workflow uses Ubuntu
# 22.04 (glibc 2.35) for that reason.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rust_dir="$repo/rust"
app_id="io.github.KozyBlake.Celeste"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$rust_dir/Cargo.toml" | head -1)"
if [ -z "$version" ]; then
    echo "Could not read the version from $rust_dir/Cargo.toml" >&2
    exit 1
fi

binary="$rust_dir/target/release/celeste"
if [ ! -x "$binary" ]; then
    echo "No release binary at $binary - run 'cargo build --release' first." >&2
    exit 1
fi

build="$rust_dir/target/appimage"
appdir="$build/Celeste.AppDir"
output="${APPIMAGE_OUTPUT:-$build/Celeste-$version-x86_64.AppImage}"

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/applications"

install -m 755 "$binary" "$appdir/usr/bin/celeste"
install -m 644 "$rust_dir/assets/linux/$app_id.desktop" \
    "$appdir/usr/share/applications/$app_id.desktop"
cp "$rust_dir/assets/linux/$app_id.desktop" "$appdir/$app_id.desktop"

for size in 16 24 32 48 64 128 256 512; do
    icon="$rust_dir/assets/icons/celeste-$size.png"
    [ -f "$icon" ] || continue
    dir="$appdir/usr/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    install -m 644 "$icon" "$dir/$app_id.png"
done
install -m 644 "$rust_dir/assets/icons/celeste-256.png" "$appdir/$app_id.png"
cp "$appdir/$app_id.png" "$appdir/.DirIcon"

# OpenSSL is the only system library Celeste links that is not part of the
# base C runtime. Every distribution new enough to run this binary ships
# OpenSSL 3, so the host's copy is used when it exists; these are carried
# only as a fallback. Preferring the host matters because forcing our
# libcrypto on everything through LD_LIBRARY_PATH can break the graphics
# and audio libraries the app loads at runtime.
for lib in libssl.so.3 libcrypto.so.3; do
    # Do not exit awk after the first match. With `pipefail`, that closes the
    # pipe while ldconfig is still writing and turns a successful lookup into
    # SIGPIPE (exit 141), aborting the entire AppImage build.
    path="$(ldconfig -p 2>/dev/null | awk -v l="$lib" '$1 == l && /x86-64/ && !found {print $NF; found=1}')"
    if [ -n "$path" ] && [ -f "$path" ]; then
        install -m 644 "$path" "$appdir/usr/lib/$lib"
    else
        echo "warning: $lib not found on this machine; not bundling it" >&2
    fi
done

cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
# Celeste's AppImage entry point.
here="$(dirname "$(readlink -f "$0")")"

# Use the host's OpenSSL when it has one, so the system certificate store
# and any distribution patches apply, and so the libraries Celeste loads
# at runtime (graphics, audio) keep using the versions they were built
# against. Fall back to the bundled copies only when the host has none.
if ! ldconfig -p 2>/dev/null | grep -q 'libssl\.so\.3'; then
    LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export LD_LIBRARY_PATH
    # A bundled OpenSSL looks for certificates where it was compiled to,
    # which is not where this host keeps them. Point it at whichever
    # bundle is actually present.
    if [ -z "${SSL_CERT_FILE:-}" ]; then
        for candidate in \
            /etc/ssl/certs/ca-certificates.crt \
            /etc/pki/tls/certs/ca-bundle.crt \
            /etc/ssl/ca-bundle.pem \
            /etc/ssl/cert.pem
        do
            if [ -r "$candidate" ]; then
                SSL_CERT_FILE="$candidate"
                export SSL_CERT_FILE
                break
            fi
        done
    fi
fi

exec "$here/usr/bin/celeste" "$@"
APPRUN
chmod 755 "$appdir/AppRun"

mkdir -p "$(dirname "$output")"
rm -f "$output"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL:-appimagetool}" \
    --no-appstream "$appdir" "$output"

echo "$output"
