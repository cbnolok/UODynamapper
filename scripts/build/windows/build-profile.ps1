# scripts/build/windows/build-profile.ps1
# Profile build: release optimizations with debug symbols for profilers (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows profile build (release optimizations + debug symbols, no LTO)..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.bat").Path

# Profile build: release optimizations with debug symbols for profilers.
# - No LTO: makes profilers more precise
# - Debug symbols: included for profiling
# - No strip: keep symbols
# - force-frame-pointers: essential for profilers
$Env:RUSTFLAGS = "-C force-frame-pointers=yes $Env:RUSTFLAGS"

cargo build --profile profiling --locked --workspace --no-default-features --features profiling `
    $args

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

# Verify the binary was created
if (!(Test-Path "target/profiling/dynamapper.exe")) {
    Write-Error "Build succeeded but dynamapper.exe was not found in target/profiling/"
    exit 1
}

Write-Host "Profile build complete."
