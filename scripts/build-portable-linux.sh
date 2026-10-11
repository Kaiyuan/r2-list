#!/usr/bin/env bash
set -euo pipefail

requested_arch="${1:-${TARGET_ARCH:-}}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script must run on Linux." >&2
  exit 1
fi

case "${requested_arch:-$(uname -m)}" in
  x64|x86_64|amd64)
    architecture="x64"
    ;;
  arm64|aarch64|armv8)
    architecture="arm64"
    ;;
  *)
    echo "Unsupported Linux architecture: ${requested_arch:-$(uname -m)}. Supported values: x64, arm64." >&2
    exit 1
    ;;
esac

for tool in node npm cargo; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repository_root"

npm ci
export TAURI_BUILD_ARCH="$architecture"
npm run tauri build -- --bundles appimage

bundle_directory="$repository_root/src-tauri/target/release/bundle/appimage"
appimage="$(find "$bundle_directory" -maxdepth 1 -type f -name '*.AppImage' -print -quit)"
[[ -n "$appimage" ]] || { echo "No AppImage was produced in $bundle_directory" >&2; exit 1; }

output_directory="$repository_root/artifacts/portable"
mkdir -p "$output_directory"
portable_app="$output_directory/r2-list-linux-${architecture}.AppImage"
cp "$appimage" "$portable_app"
chmod +x "$portable_app"
cp "$repository_root/docs/portable-releases.md" "$output_directory/r2-list-linux-${architecture}-README.md"
echo "Portable build ready: $portable_app"