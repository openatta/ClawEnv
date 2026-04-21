# ClawLite — Windows Build Script
#
# ClawLite shares the SAME frontend bundle and SAME Rust binary as
# ClawEnv. The only difference is the Tauri config override in
# `lite\clawlite.tauri.conf.json` which changes productName, identifier,
# version, and window dimensions. At runtime, `src/App.tsx` detects the
# app name and swaps the install component.
#
# Output:
#   target\release\bundle\nsis\ClawLite_<version>_<arch>-setup.exe
#   target\release\bundle\msi\ClawLite_<version>_<arch>_en-US.msi
param(
    [string]$CopyTo = "",
    [switch]$Help
)

$ErrorActionPreference = "Stop"

if ($Help) {
    Write-Host "Usage: build-lite-windows.ps1 [-CopyTo <dir>]"
    exit 0
}

$ProjectDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $ProjectDir

Write-Host "=========================================="
Write-Host "  ClawLite Windows Build"
Write-Host "=========================================="

$rustcVer = (rustc --version) -replace '.*?(\d+\.\d+\.\d+).*', '$1'
$rustcMinor = [int]($rustcVer.Split('.')[1])
if ($rustcMinor -lt 88) {
    $cargoRustc = Join-Path $env:USERPROFILE ".cargo\bin\rustc.exe"
    if (Test-Path $cargoRustc) {
        $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    } else {
        Write-Host "ERROR: Need rustc 1.88+ for Tauri. Run: rustup update stable" -ForegroundColor Red
        exit 1
    }
}

# Install npm deps (main — lite shares the same frontend build).
npm install --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# See macOS script comment — same "invoke from tauri/ dir" requirement.
Push-Location tauri
try {
    cargo tauri build --config ..\lite\clawlite.tauri.conf.json
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

$nsis = Get-ChildItem -Path "target\release\bundle\nsis" -Filter "ClawLite_*-setup.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
$msi  = Get-ChildItem -Path "target\release\bundle\msi"  -Filter "ClawLite_*.msi"       -ErrorAction SilentlyContinue | Select-Object -First 1

Write-Host ""
Write-Host "--- Build artifacts ---"
if ($nsis) { Write-Host "  NSIS: $($nsis.FullName)" }
if ($msi)  { Write-Host "  MSI:  $($msi.FullName)" }

if ($CopyTo -ne "") {
    if (-not (Test-Path $CopyTo)) { New-Item -ItemType Directory -Path $CopyTo | Out-Null }
    if ($nsis) { Copy-Item $nsis.FullName $CopyTo -Force }
    if ($msi)  { Copy-Item $msi.FullName  $CopyTo -Force }
    Write-Host ""
    Write-Host "  Copied to: $CopyTo"
}

Write-Host ""
Write-Host "Build complete."
