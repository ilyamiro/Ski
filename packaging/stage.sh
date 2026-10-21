#!/bin/sh
# Lay out an installed Ski under $1 with the prefix /usr, from a release build (target/release/ski).
# The release tarball, the .deb and the .rpm are all made from this tree.
set -eu
dest=${1:?usage: stage.sh DEST}
cd "$(dirname "$0")/.."

install -Dm755 target/release/ski "$dest/usr/bin/ski"
install -Dm644 assets/ski.desktop "$dest/usr/share/applications/ski.desktop"
install -Dm644 assets/logo.svg "$dest/usr/share/icons/hicolor/scalable/apps/ski.svg"
install -Dm644 assets/ski.portal "$dest/usr/share/xdg-desktop-portal/portals/ski.portal"
for name in org.freedesktop.impl.portal.desktop.ski org.freedesktop.FileManager1; do
  install -Dm644 "assets/$name.service" "$dest/usr/share/dbus-1/services/$name.service"
  sed -i 's|@out@|/usr|' "$dest/usr/share/dbus-1/services/$name.service"
done
install -Dm644 LICENSE "$dest/usr/share/licenses/ski/LICENSE"
