#!/bin/sh
# Packs the Linux build for a release: a .tar.gz and an AppImage.
#
#   packaging/linux/package.sh <path to the built binary> <version> <output folder>
#
# The .tar.gz holds the binary, the icon, the .desktop file and the licenses. The AppImage is
# one file that runs on most distributions (and on a Steam Deck) without installing anything;
# it is made with appimagetool, taken from $APPIMAGETOOL or downloaded once into the output
# folder.
set -eu

binary=$1
version=$2
out=$3
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
name=rimworld-mod-manager
mkdir -p "$out"
out=$(cd "$out" && pwd)

# --- .tar.gz ---
stage="$out/$name-$version-linux-x86_64"
rm -rf "$stage"
mkdir -p "$stage"
install -m 755 "$binary" "$stage/$name"
install -m 644 "$repo/assets/icon.png" "$stage/$name.png"
install -m 644 "$here/$name.desktop" "$stage/$name.desktop"
install -m 644 "$repo/LICENSE" "$stage/LICENSE"
install -m 644 "$repo/assets/fonts/OFL.txt" "$stage/Montserrat-OFL.txt"
tar -C "$out" -czf "$stage.tar.gz" "$(basename "$stage")"
rm -rf "$stage"

# --- AppImage --- (SKIP_APPIMAGE=1: the .tar.gz only, nothing downloaded)
if [ "${SKIP_APPIMAGE:-}" = "1" ]; then
    cd "$out"
    sha256sum "$name-$version-linux-x86_64.tar.gz" > "$name-$version-linux.sha256"
    ls -la "$out"
    exit 0
fi
appdir="$out/AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$binary" "$appdir/usr/bin/$name"
install -m 644 "$here/$name.desktop" "$appdir/usr/share/applications/$name.desktop"
install -m 644 "$repo/assets/icon.png" "$appdir/usr/share/icons/hicolor/256x256/apps/$name.png"
# appimagetool takes the .desktop file, the icon and AppRun from the top of the AppDir.
cp "$here/$name.desktop" "$appdir/$name.desktop"
cp "$repo/assets/icon.png" "$appdir/$name.png"
ln -s "usr/bin/$name" "$appdir/AppRun"

tool=${APPIMAGETOOL:-"$out/appimagetool-x86_64.AppImage"}
if [ ! -x "$tool" ]; then
    curl -sSL -o "$tool" "https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage"
    chmod +x "$tool"
fi
# Without FUSE (CI, containers) the tool unpacks itself instead of mounting.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tool" --no-appstream "$appdir" "$out/$name-$version-x86_64.AppImage" >/dev/null
rm -rf "$appdir"

cd "$out"
sha256sum "$name-$version-linux-x86_64.tar.gz" "$name-$version-x86_64.AppImage" > "$name-$version-linux.sha256"
ls -la "$out"
