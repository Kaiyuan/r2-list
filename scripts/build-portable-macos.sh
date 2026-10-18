#!/usr/bin/env bash
set -euo pipefail

# Usage: build-portable-macos.sh [native|arm64|x64] [deployment-target]
#   native              build for the host architecture (default)
#   arm64               Apple Silicon, requires macOS 11 or later
#   x64                 Intel; pass 10.15 to target macOS 10.15 Catalina
#   deployment-target   optional macOS version, e.g. 10.15. Tauri uses it for both
#                       MACOSX_DEPLOYMENT_TARGET and LSMinimumSystemVersion.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi
for tool in node npm cargo ditto; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done

case "$(uname -m)" in
  arm64) native_architecture="arm64" ;;
  x86_64) native_architecture="x64" ;;
  *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac

requested_architecture="${1:-native}"
case "$requested_architecture" in
  native) architecture="$native_architecture" ;;
  arm64|aarch64) architecture="arm64" ;;
  x64|x86_64|amd64) architecture="x64" ;;
  *) echo "Unsupported macOS architecture: $requested_architecture. Supported values: native, arm64, x64." >&2; exit 1 ;;
esac

deployment_target="${2:-}"
if [[ -n "$deployment_target" && ! "$deployment_target" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
  echo "Invalid macOS deployment target: $deployment_target" >&2
  exit 1
fi
if [[ "$architecture" == "arm64" && "$deployment_target" =~ ^10\. ]]; then
  echo "Apple Silicon builds require a deployment target of 11.0 or later." >&2
  exit 1
fi

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repository_root"
npm ci

case "$architecture" in
  arm64) rust_target="aarch64-apple-darwin" ;;
  x64) rust_target="x86_64-apple-darwin" ;;
esac

target_directory="release"
build_arguments=(--bundles app)
if [[ "$architecture" != "$native_architecture" ]]; then
  command -v rustup >/dev/null 2>&1 || { echo "Missing prerequisite: rustup (required to cross-compile $rust_target)" >&2; exit 1; }
  rustup target add "$rust_target"
  build_arguments+=(--target "$rust_target")
  target_directory="$rust_target/release"
fi
if [[ -n "$deployment_target" ]]; then
  # Tauri derives both MACOSX_DEPLOYMENT_TARGET and LSMinimumSystemVersion from this value.
  export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
  build_arguments+=(--config "{\"bundle\":{\"macOS\":{\"minimumSystemVersion\":\"$deployment_target\"}}}")
fi

npm run tauri build -- "${build_arguments[@]}"

app_bundle="$repository_root/src-tauri/target/$target_directory/bundle/macos/R2 List.app"
[[ -d "$app_bundle" ]] || { echo "Expected app bundle was not produced: $app_bundle" >&2; exit 1; }

if [[ -n "$deployment_target" ]]; then
  built_deployment_target="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$app_bundle/Contents/Info.plist")"
  [[ "$built_deployment_target" == "$deployment_target" ]] || {
    echo "Expected LSMinimumSystemVersion $deployment_target, found $built_deployment_target" >&2
    exit 1
  }
  built_binary_minimum="$(otool -l "$app_bundle/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_bundle/Contents/Info.plist")" | awk '/LC_BUILD_VERSION/{inside=1; next} inside && /minos/{print $2; exit}')"
  [[ "$built_binary_minimum" == "$deployment_target" ]] || {
    echo "Expected a binary deployment target of $deployment_target, found ${built_binary_minimum:-unknown}" >&2
    exit 1
  }
fi

artifact_suffix="$architecture"
if [[ -n "$deployment_target" ]]; then
  artifact_suffix="$artifact_suffix-$deployment_target"
fi

output_root="$repository_root/artifacts/portable"
staging="$output_root/R2-List-macos-$artifact_suffix"
archive="$output_root/r2-list-macos-$artifact_suffix.zip"
mkdir -p "$output_root"
rm -rf "$staging"
mkdir -p "$staging"
ditto "$app_bundle" "$staging/R2 List.app"
cp "$repository_root/docs/portable-releases.md" "$staging/README-portable.md"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$staging" "$archive"
echo "Portable build ready: $archive"