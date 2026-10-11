$ErrorActionPreference = "Stop"

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repositoryRoot

foreach ($tool in @("node", "npm", "cargo")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Missing prerequisite '$tool'. Install Node.js and the Rust stable toolchain first."
    }
}
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
    throw "This script currently builds Windows x64 only (found $env:PROCESSOR_ARCHITECTURE)."
}

npm.cmd ci
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$generatedIcons = Join-Path $env:TEMP "r2-list-generated-icons"
New-Item -ItemType Directory -Force -Path $generatedIcons | Out-Null
npm.cmd run tauri -- icon src-tauri/icons/icon.png --output $generatedIcons
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item -LiteralPath (Join-Path $generatedIcons "icon.ico") -Destination (Join-Path $repositoryRoot "src-tauri\icons\icon.ico") -Force
npm.cmd run tauri build -- --no-bundle
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$binary = Join-Path $repositoryRoot "src-tauri\target\release\r2-list.exe"
if (-not (Test-Path $binary)) { throw "Expected portable executable was not produced: $binary" }

$outputDirectory = Join-Path $repositoryRoot "artifacts\portable\windows-x64"
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $outputDirectory "r2-list.exe") -Force
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\portable-releases.md") -Destination (Join-Path $outputDirectory "README-portable.md") -Force

$archive = Join-Path $repositoryRoot "artifacts\portable\r2-list-windows-x64.zip"
Compress-Archive -Path (Join-Path $outputDirectory "*") -DestinationPath $archive -Force
Write-Host "Portable build ready: $archive"