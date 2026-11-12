#!/usr/bin/env bash
# Build a Linux AppImage from an already-built release binary.
#
# Usage:
#   packaging/build-appimage.sh [--binary <path>] [--out <dir>]
#
# The AppImage is assembled from the manual AppDir layout, deliberately WITHOUT
# bundling shared libraries. Verified DT_NEEDED closure of the GUI stack is only
# libxkbcommon, libxcb, libfontconfig, libfreetype and glibc/libgcc; everything
# else (Vulkan, EGL, Wayland, D-Bus, portals) is dlopen'd at runtime. The
# AppImage excludelist forbids redistributing glibc, libgcc_s, libstdc++ and the
# graphics-driver libraries, and lists libxcb/libfontconfig too, so `usr/lib`
# stays empty and the host supplies them. Never bundle a Vulkan ICD.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1 | tr -d '\r')
target_dir="${CARGO_TARGET_DIR:-$root/target}"
binary="$target_dir/x86_64-unknown-linux-gnu/release/RTXManager"
out="$root/dist"

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) binary="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    -h|--help) sed -n '2,6p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

echo "==> RTXManager $version AppImage"

if [ ! -x "$binary" ]; then
  echo "error: release binary not found at $binary" >&2
  echo "build it first: cargo build --release --target x86_64-unknown-linux-gnu" >&2
  exit 1
fi

# appimagetool requires these on PATH and exits with a confusing error otherwise.
for tool in file mksquashfs desktop-file-validate; do
  command -v "$tool" >/dev/null || {
    echo "error: '$tool' is required (install squashfs-tools + desktop-file-utils)" >&2
    exit 1
  }
done

appdir=$(mktemp -d)/RTXManager.AppDir
mkdir -p "$appdir/usr/bin" \
         "$appdir/usr/share/applications" \
         "$appdir/usr/share/icons/hicolor/256x256/apps" \
         "$appdir/usr/lib"

install -m 755 "$binary" "$appdir/usr/bin/RTXManager"
install -m 755 "$root/packaging/AppRun" "$appdir/AppRun"
install -m 644 "$root/packaging/rtx-fg-manager.desktop" "$appdir/rtx-fg-manager.desktop"
install -m 644 "$root/packaging/rtx-fg-manager.desktop" \
  "$appdir/usr/share/applications/rtx-fg-manager.desktop"
install -m 644 "$root/packaging/rtx-fg-manager.png" "$appdir/rtx-fg-manager.png"
install -m 644 "$root/packaging/rtx-fg-manager.png" \
  "$appdir/usr/share/icons/hicolor/256x256/apps/rtx-fg-manager.png"

# The root icon file must carry the extension while `Icon=` must not; appimagetool
# also insists on exactly one .desktop file in the root and on a Categories= key.
cp --no-dereference "$appdir/rtx-fg-manager.png" "$appdir/.DirIcon" 2>/dev/null || \
  ln -sf rtx-fg-manager.png "$appdir/.DirIcon"

desktop-file-validate "$appdir/rtx-fg-manager.desktop"

# Report what the binary actually links, so a future dependency change is visible
# rather than silently producing an AppImage that fails to start on a clean host.
echo "==> DT_NEEDED of the release binary:"
readelf -d "$appdir/usr/bin/RTXManager" 2>/dev/null | grep NEEDED || \
  ldd "$appdir/usr/bin/RTXManager" | head -20

mkdir -p "$out"
tool=${APPIMAGETOOL:-$root/.build-env/appimagetool/appimagetool-x86_64.AppImage}
if [ ! -x "$tool" ]; then
  echo "error: appimagetool not found at $tool" >&2
  echo "download it or set APPIMAGETOOL to its path" >&2
  exit 1
fi

# FUSE is commonly unavailable in containers; appimagetool is itself an AppImage.
target="$out/RTXManager-$version-x86_64.AppImage"
ARCH=x86_64 VERSION="$version" APPIMAGE_EXTRACT_AND_RUN=1 \
  "$tool" --no-appstream -v "$appdir" "$target"

echo "==> built $target"
sha256sum "$target"
