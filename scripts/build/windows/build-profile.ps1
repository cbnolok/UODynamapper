# scripts/build/windows/build-profile.ps1
# Profile build: release optimizations with debug symbols for profilers (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows profile build (release optimizations + debug symbols, no LTO)..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = "$scriptDir\..\common\rustc_wrapper.ps1"

# Profile build: release optimizations with debug symbols for profilers.
# - No LTO: makes profilers more precise
# - Debug symbols: included for profiling
# - No strip: keep symbols
# - force-frame-pointers: essential for profilers
$Env:RUSTFLAGS = "-C force-frame-pointers=yes $Env:RUSTFLAGS"

cargo build --profile profiling --locked --no-default-features --features profiling `
    --bin dynamapper --package dynamapper `
    $args

Write-Host "Profile build complete."
