#!/usr/bin/env bash
set -euo pipefail

# Usage: build-portable-macos.sh [native|arm64|x64] [deployment-target]

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi
for tool in cargo ditto; do
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

case "$architecture" in
  arm64) rust_target="aarch64-apple-darwin" ;;
  x64) rust_target="x86_64-apple-darwin" ;;
esac
target_directory="$rust_target"
rustup target add "$rust_target"
if [[ -n "$deployment_target" ]]; then
  export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
fi
cargo build --manifest-path desktop/Cargo.toml --release --target "$rust_target"

artifact_suffix="$architecture"
if [[ -n "$deployment_target" ]]; then
  artifact_suffix="$artifact_suffix-$deployment_target"
fi
output_root="$repository_root/artifacts/portable"
staging="$output_root/R2-List-macos-$artifact_suffix"
archive="$output_root/r2-list-macos-$artifact_suffix.zip"
binary="$repository_root/desktop/target/$target_directory/release/r2-list"
[[ -f "$binary" ]] || { echo "Expected executable was not produced: $binary" >&2; exit 1; }

mkdir -p "$output_root"
rm -rf "$staging"
mkdir -p "$staging"
cp "$binary" "$staging/r2-list"
cp "$repository_root/docs/portable-releases.md" "$staging/README-portable.md"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$staging" "$archive"
echo "Portable build ready: $archive"
