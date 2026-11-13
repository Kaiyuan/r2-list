# Portable Releases

Each platform script builds only for its own operating system; the macOS script can also cross-compile for the other Apple architecture. Build prerequisites are Rust stable, the native build libraries required by Slint, and the platform archive tool (`zip` on Linux).

## Artifacts

- Local builds use `r2-list-windows-x64.zip`, `r2-list-macos-<architecture>.zip`, and `r2-list-linux-<architecture>.zip`.
- GitHub Release assets include the version tag in each filename, for example `r2-list-windows-x64-v0.0.3.zip`, `r2-list-macos-arm64-v0.0.3.zip`, `r2-list-macos-x64-10.15-v0.0.3.zip`, `r2-list-linux-x64-v0.0.3.zip`, and `r2-list-linux-arm64-v0.0.3.zip`.
- macOS builds are produced for Apple Silicon (`arm64`) and Intel (`x64`). The Intel release build targets macOS 10.15 when built with the provided workflow.

These are archives containing the native executable and this document, not installers or self-contained application bundles. Slint does not require WebView2 or WebKit; Linux still requires compatible host graphics libraries. macOS builds are unsigned, so Gatekeeper may require approval. Windows SmartScreen may also display a warning for unsigned binaries.

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

## GitHub Releases

Pushing a version tag matching `v*` (for example, `v1.2.3`) starts `.github/workflows/release-portable.yml`. Pushing code to a branch or creating a non-version tag does not start the workflow. It builds Windows x64, macOS arm64, macOS x64 (10.15 Catalina compatible, on an Intel runner), and both Linux architectures (x64 and arm64) in parallel and uploads their outputs to the GitHub Release for that exact tag. The repository Actions workflow needs `contents: write` permission, which the workflow requests explicitly.

## GitHub Pages

The project site is static HTML in `docs/`; it does not need a build workflow. To publish it, set **Settings → Pages → Build and deployment → Source** to **Deploy from a branch**, then select the default branch and `/docs` folder. If the `github-pages` environment has deployment branch/tag restrictions, allow the default branch as well. Pages publishing is separate from the version-tag workflow that builds and releases the desktop packages.