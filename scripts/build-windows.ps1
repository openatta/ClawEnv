# ClawEnv — Windows Build Script
#
# Builds the full Tauri app (GUI + CLI sidecar) for Windows.
# Outputs: tauri\target\release\bundle\nsis\*.exe
#
# Prerequisites:
#   - Visual Studio Build Tools (C++ workload): https://visualstudio.microsoft.com/visual-cpp-build-tools/
#   - Rust (rustup): https://rustup.rs
#   - Node.js 20+: https://nodejs.org
#   - WebView2 Runtime (Windows 10+): usually pre-installed
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -Mode dev
#   powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -CliOnly
#
param(
    [ValidateSet("release", "dev")]
    [string]$Mode = "release",
    [switch]$CliOnly,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

if ($Help) {
    Write-Host "Usage: build-windows.ps1 [-Mode release|dev] [-CliOnly]"
    Write-Host "  -Mode release   Full optimized build (default)"
    Write-Host "  -Mode dev       Debug build (faster compilation)"
    Write-Host "  -CliOnly        Build CLI binary only, skip Tauri GUI"
    exit 0
}

$ProjectDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $ProjectDir

Write-Host "============================================"
Write-Host "  ClawEnv Windows Build ($Mode)"
Write-Host "============================================"
Write-Host ""

# --- 1. Check prerequisites ---
Write-Host "--- Checking prerequisites ---"

function Test-Command($cmd, $hint) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Host "ERROR: $cmd not found. $hint" -ForegroundColor Red
        exit 1
    }
}

Test-Command "rustc" "Install Rust: https://rustup.rs"
Test-Command "cargo" "Install Rust: https://rustup.rs"
Test-Command "node"  "Install Node.js 20+: https://nodejs.org"
Test-Command "npm"   "Comes with Node.js"

# Check rustc version for Tauri (needs 1.88+)
$rustcVer = (rustc --version) -replace '.*?(\d+\.\d+\.\d+).*', '$1'
$rustcMinor = [int]($rustcVer.Split('.')[1])
if ($rustcMinor -lt 88 -and -not $CliOnly) {
    # Try rustup's rustc
    $cargoRustc = Join-Path $env:USERPROFILE ".cargo\bin\rustc.exe"
    if (Test-Path $cargoRustc) {
        $cargoVer = (& $cargoRustc --version) -replace '.*?(\d+\.\d+\.\d+).*', '$1'
        Write-Host "  System rustc $rustcVer too old, using $cargoVer from ~/.cargo/bin/"
        $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    } else {
        Write-Host "ERROR: Need rustc 1.88+ for Tauri. Run: rustup update stable" -ForegroundColor Red
        exit 1
    }
}

Write-Host "  rustc: $(rustc --version)"
Write-Host "  cargo: $(cargo --version)"
Write-Host "  node:  $(node --version)"
Write-Host "  npm:   $(npm --version)"
Write-Host ""

# --- 2. Install frontend dependencies ---
if (-not $CliOnly) {
    Write-Host "--- Installing frontend dependencies ---"
    npm install --no-audit --no-fund
    Write-Host ""
}

# --- 3. Build ---
if ($CliOnly) {
    Write-Host "--- Building CLI only ---"
    if ($Mode -eq "release") {
        cargo build -p clawcli --release
        $bin = "target\release\clawcli.exe"
    } else {
        cargo build -p clawcli
        $bin = "target\debug\clawcli.exe"
    }
    Write-Host ""
    Write-Host "  CLI binary: $bin"
    & ".\$bin" --version
    Get-Item $bin | Select-Object Length, LastWriteTime
} else {
    Write-Host "--- Building Tauri app ($Mode) ---"
    if ($Mode -eq "release") {
        npx tauri build
    } else {
        cargo build -p clawcli
        node scripts\copy-cli-sidecar.cjs debug
        npm run build
        cargo build -p clawgui
        Write-Host ""
        Write-Host "  Dev build complete. Run with: npx tauri dev"
    }
}

Write-Host ""

# --- 4. Output ---
if ($Mode -eq "release" -and -not $CliOnly) {
    Write-Host "--- Build artifacts ---"
    $nsis = Get-ChildItem -Path "tauri\target\release\bundle\nsis" -Filter "*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($nsis) {
        Write-Host "  Installer: $($nsis.FullName)"
        Write-Host "  Size: $([math]::Round($nsis.Length / 1MB, 1)) MB"
    } else {
        Write-Host "  WARNING: NSIS installer not found. Check tauri\target\release\bundle\"
        Get-ChildItem "tauri\target\release\bundle\" -ErrorAction SilentlyContinue
    }
}

Write-Host ""
Write-Host "Build complete."
