# Portable Releases

Each platform script builds only for the current host. Build prerequisites are Node.js 20.19+ (or 22.12+), Rust stable, and the platform dependencies listed in the Tauri v2 prerequisites.

## Artifacts

- Windows x64: `r2-list-windows-x64.zip`, containing `r2-list.exe`.
- macOS: `r2-list-macos-<architecture>.zip`, containing the `.app` bundle. The architecture is detected from the build runner (`arm64` or `x64`).
- Linux x64: `r2-list-linux-x64.AppImage`.

These are portable outputs, not installers. Windows still requires the WebView2 Runtime; it is included with current Windows releases on most machines. Linux AppImage execution may require FUSE 2 and the host WebKit/GTK libraries. macOS builds are unsigned unless signing credentials are configured, so Gatekeeper may require the user to approve the app manually. Windows SmartScreen may also display a warning for unsigned binaries.

## Local Builds

Run the matching command from the repository root:

```sh
npm run build:portable:windows
npm run build:portable:macos
npm run build:portable:linux
```

Artifacts are written under the ignored `artifacts/portable/` directory. The scripts use `npm ci` and the Tauri CLI; Rust, the native linker, and operating-system libraries must already be installed.

## GitHub Releases

Pushing a version tag matching `v*` (for example, `v1.2.3`) starts `.github/workflows/release-portable.yml`. Pushing code to a branch or creating a non-version tag does not start the workflow. It builds all three platforms in parallel and uploads their outputs to the GitHub Release for that exact tag. The repository Actions workflow needs `contents: write` permission, which the workflow requests explicitly.

## GitHub Pages

The project site is deployed from `docs/` by `.github/workflows/pages.yml`, also only when a `v*` version tag is pushed. Before the first deployment:

1. Set **Settings → Pages → Build and deployment → Source** to **GitHub Actions**.
2. Open **Settings → Environments → github-pages** and make sure its deployment branch/tag rules allow `v*` tags (or allow all branches and tags). Otherwise GitHub rejects the Pages deployment before its job starts with an environment protection error.

The Pages deployment is a separate workflow from the portable build and release workflow. A Pages environment rejection does not cancel or block the portable build jobs.