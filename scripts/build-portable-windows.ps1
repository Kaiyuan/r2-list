$ErrorActionPreference = "Stop"

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repositoryRoot

foreach ($tool in @("cargo")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Missing prerequisite '$tool'. Install the Rust stable toolchain first."
    }
}
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
    throw "This script currently builds Windows x64 only (found $env:PROCESSOR_ARCHITECTURE)."
}

cargo build --manifest-path desktop\Cargo.toml --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$binary = Join-Path $repositoryRoot "desktop\target\release\r2-list.exe"
if (-not (Test-Path $binary)) { throw "Expected portable executable was not produced: $binary" }

$outputDirectory = Join-Path $repositoryRoot "artifacts\portable\windows-x64"
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $outputDirectory "r2-list.exe") -Force
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\portable-releases.md") -Destination (Join-Path $outputDirectory "README-portable.md") -Force

$archive = Join-Path $repositoryRoot "artifacts\portable\r2-list-windows-x64.zip"
$staging = Join-Path ([System.IO.Path]::GetTempPath()) ("r2-list-portable-" + [guid]::NewGuid().ToString("N"))
$stagingFiles = Join-Path $staging "windows-x64"
New-Item -ItemType Directory -Force -Path $stagingFiles | Out-Null
try {
    Copy-Item -Path (Join-Path $outputDirectory "*") -Destination $stagingFiles -Force
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $stagingArchive = Join-Path $staging "r2-list-windows-x64.zip"
    [System.IO.Compression.ZipFile]::CreateFromDirectory($stagingFiles, $stagingArchive)
    Copy-Item -LiteralPath $stagingArchive -Destination $archive -Force
}
finally {
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "Portable build ready: $archive"