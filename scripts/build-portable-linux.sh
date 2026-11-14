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
    rust_target="x86_64-unknown-linux-gnu"
    ;;
  arm64|aarch64|armv8)
    architecture="arm64"
    rust_target="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "Unsupported Linux architecture: ${requested_arch:-$(uname -m)}. Supported values: x64, arm64." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64|amd64) host_architecture="x64" ;;
  aarch64|armv8*) host_architecture="arm64" ;;
  *) echo "Unsupported Linux host architecture: $(uname -m)" >&2; exit 1 ;;
esac
if [[ "$architecture" != "$host_architecture" ]]; then
  echo "Requested $architecture build does not match the $host_architecture host." >&2
  exit 1
fi
for tool in cargo zip; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done
if [[ "$rust_target" == "x86_64-unknown-linux-gnu" ]] && ! command -v nasm >/dev/null 2>&1; then
  echo "Missing prerequisite: nasm (required to build the AVIF decoder on x86/x64)." >&2
  echo "Install it with: sudo apt-get install nasm   (or your distribution's package manager)" >&2
  exit 1
fi

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repository_root"
rustup target add "$rust_target"
cargo build --manifest-path desktop/Cargo.toml --release --target "$rust_target"

output_directory="$repository_root/artifacts/portable"
staging="$output_directory/R2-List-linux-$architecture"
archive="$output_directory/r2-list-linux-$architecture.zip"
binary="$repository_root/desktop/target/$rust_target/release/r2-list"
[[ -f "$binary" ]] || { echo "Expected executable was not produced: $binary" >&2; exit 1; }

mkdir -p "$output_directory"
rm -rf "$staging"
mkdir -p "$staging"
cp "$binary" "$staging/r2-list"
chmod +x "$staging/r2-list"
cp "$repository_root/docs/portable-releases.md" "$staging/README-portable.md"
rm -f "$archive"
(
  cd "$output_directory"
  zip -q -r "$(basename "$archive")" "$(basename "$staging")"
)
echo "Portable build ready: $archive"
