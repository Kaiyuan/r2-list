#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi
for tool in node npm cargo ditto; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done

case "$(uname -m)" in
  arm64) architecture="arm64" ;;
  x86_64) architecture="x64" ;;
  *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repository_root"
npm ci
npm run tauri build -- --bundles app

app_bundle="$repository_root/src-tauri/target/release/bundle/macos/R2 List.app"
[[ -d "$app_bundle" ]] || { echo "Expected app bundle was not produced: $app_bundle" >&2; exit 1; }

output_root="$repository_root/artifacts/portable"
staging="$output_root/R2-List-macos-$architecture"
archive="$output_root/r2-list-macos-$architecture.zip"
mkdir -p "$output_root"
rm -rf "$staging"
mkdir -p "$staging"
ditto "$app_bundle" "$staging/R2 List.app"
cp "$repository_root/docs/portable-releases.md" "$staging/README-portable.md"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$staging" "$archive"
echo "Portable build ready: $archive"