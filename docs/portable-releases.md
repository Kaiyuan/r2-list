# Portable Releases

Each platform script builds only for its own operating system; the macOS script can also cross-compile for the other Apple architecture. Build prerequisites are Rust 1.98 or newer, NASM (only for x86/x64 targets, where the bundled AVIF decoder assembles code), the native build libraries required by Slint, and the platform archive tool (`zip` on Linux, built-in `ditto` on macOS). The Windows script downloads a portable NASM automatically when it is missing.

## Artifacts

- Local builds use `r2-list-windows-x64.zip`, `r2-list-macos-<architecture>.zip`, and `r2-list-linux-<architecture>.zip`.
- GitHub Release assets include the version tag in each filename, for example `r2-list-windows-x64-v0.0.4.zip`, `r2-list-macos-arm64-v0.0.4.zip`, `r2-list-macos-x64-10.15-v0.0.4.zip`, `r2-list-linux-x64-v0.0.4.zip`, and `r2-list-linux-arm64-v0.0.4.zip`.
- macOS builds are produced for Apple Silicon (`arm64`) and Intel (`x64`). The Intel release build targets macOS 10.15 when built with the provided workflow.
- The macOS archive contains `R2 List.app` (with `Info.plist` and the application icon), so it can be started from Finder like a normal Mac application.

These are archives containing the native executable and this document, not installers. Slint does not require WebView2 or WebKit.

## Runtime requirements

- **Windows:** Windows 10 or later. SmartScreen may display a warning for unsigned binaries.
- **macOS:** macOS 11 or later (macOS 10.15 for the Intel build produced by the workflow). Builds are unsigned, so Gatekeeper blocks the first launch. Either right-click `R2 List.app` and choose **Open**, or run `xattr -dr com.apple.quarantine "/Applications/R2 List.app"` once.
- **Linux:** Ubuntu 24.04 or newer, Debian 13 or newer, or another distribution with glibc 2.39+. The executable uses the host's X11/Wayland, fontconfig, xkbcommon and OpenGL/EGL libraries, which are installed on any desktop Ubuntu. Linux archives are built on Ubuntu 24.04; for Ubuntu 22.04 or older, build from source on that release (see below).

## Local Builds

Run the matching command from the repository root:

```sh
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-portable-windows.ps1
bash scripts/build-portable-macos.sh
bash scripts/build-portable-macos.sh x64 10.15
bash scripts/build-portable-linux.sh x64
bash scripts/build-portable-linux.sh arm64
```

Artifacts are written under the ignored `artifacts/portable/` directory. The scripts use Cargo directly; Rust, the native linker, and operating-system libraries must already be installed. Linux builds must run on a host matching the selected architecture.

- **macOS:** Xcode command line tools plus `brew install pkg-config` (`brew install nasm` is only needed for Intel builds). The script creates `R2 List.app`, converts `docs/images/r2list-icon.png` into `AppIcon.icns` with the built-in `sips`/`iconutil`, and ad-hoc signs the bundle.
- **Linux:** `build-essential pkg-config zip libssl-dev libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libxinerama-dev libxfixes-dev libxrender-dev libgl1-mesa-dev libegl1-mesa-dev` plus `nasm` for x64 (Debian/Ubuntu names).

## GitHub Releases

Pushing a version tag matching `v*` (for example, `v1.2.3`) starts `.github/workflows/release-portable.yml`. Pushing code to a branch or creating a non-version tag does not start the workflow. It builds Windows x64, macOS arm64, macOS x64 (10.15 Catalina compatible, on an Intel runner), and both Linux architectures (x64 and arm64) in parallel and uploads their outputs to the GitHub Release for that exact tag. The repository Actions workflow needs `contents: write` permission, which the workflow requests explicitly.

## GitHub Pages

The project site is static HTML in `docs/`; it does not need a build workflow. To publish it, set **Settings → Pages → Build and deployment → Source** to **Deploy from a branch**, then select the default branch and `/docs` folder. If the `github-pages` environment has deployment branch/tag restrictions, allow the default branch as well. Pages publishing is separate from the version-tag workflow that builds and releases the desktop packages.