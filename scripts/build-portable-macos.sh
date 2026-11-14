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
if [[ "$rust_target" == "x86_64-apple-darwin" ]] && ! command -v nasm >/dev/null 2>&1; then
  echo "Missing prerequisite: nasm (required to build the AVIF decoder on Intel Macs)." >&2
  echo "Install it with: brew install nasm" >&2
  exit 1
fi
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

# Ship a real .app bundle so macOS shows the app icon, a proper name in the Dock
# and menu bar, and Finder can launch it with a double click.
app="$staging/R2 List.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/r2-list"
chmod +x "$app/Contents/MacOS/r2-list"

app_version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*$/\1/p' "$repository_root/desktop/Cargo.toml" | tr -d '\r' | head -n 1)"
[[ -n "$app_version" ]] || app_version="0.0.0"
minimum_macos="${deployment_target:-11.0}"

icon_source="$repository_root/docs/images/r2list-icon.png"
if [[ -f "$icon_source" ]] && command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
  icon_work="$(mktemp -d)"
  iconset="$icon_work/AppIcon.iconset"
  mkdir -p "$iconset"
  sips -z 16 16 "$icon_source" --out "$iconset/icon_16x16.png" >/dev/null
  sips -z 32 32 "$icon_source" --out "$iconset/icon_16x16@2x.png" >/dev/null
  sips -z 32 32 "$icon_source" --out "$iconset/icon_32x32.png" >/dev/null
  sips -z 64 64 "$icon_source" --out "$iconset/icon_32x32@2x.png" >/dev/null
  sips -z 128 128 "$icon_source" --out "$iconset/icon_128x128.png" >/dev/null
  sips -z 256 256 "$icon_source" --out "$iconset/icon_128x128@2x.png" >/dev/null
  sips -z 256 256 "$icon_source" --out "$iconset/icon_256x256.png" >/dev/null
  sips -z 512 512 "$icon_source" --out "$iconset/icon_256x256@2x.png" >/dev/null
  sips -z 512 512 "$icon_source" --out "$iconset/icon_512x512.png" >/dev/null
  sips -z 1024 1024 "$icon_source" --out "$iconset/icon_512x512@2x.png" >/dev/null
  if ! iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns" 2>/dev/null; then
    echo "Warning: could not generate AppIcon.icns; the bundle will use the default icon." >&2
    rm -f "$app/Contents/Resources/AppIcon.icns"
  fi
  rm -rf "$icon_work"
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>R2 List</string>
  <key>CFBundleExecutable</key>
  <string>r2-list</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>com.r2list.desktop</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>R2 List</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${app_version}</string>
  <key>CFBundleVersion</key>
  <string>${app_version}</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>
  <string>${minimum_macos}</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSRequiresAquaSystemAppearance</key>
  <false/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
PLIST

# Ad-hoc signing keeps the bundle valid on Apple Silicon; distribution is still
# unsigned, so Gatekeeper may ask the user for approval on first launch.
codesign --force --sign - "$app" >/dev/null 2>&1 || true

cp "$repository_root/docs/portable-releases.md" "$staging/README-portable.md"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$staging" "$archive"
echo "Portable build ready: $archive"
