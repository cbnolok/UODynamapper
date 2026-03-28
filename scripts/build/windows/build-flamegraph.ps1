# scripts/build/windows/build-flamegraph.ps1
# Flamegraph build: release optimizations with debug symbols and frame pointers (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows flamegraph build (release optimizations + debug symbols, frame pointers)..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.ps1").Path

# Flamegraph build: release optimizations with debug symbols and frame pointers.
# - No LTO & No Strip: essential for stack walking
# - force-frame-pointers: essential for profilers
$Env:RUSTFLAGS = "-C force-frame-pointers=yes $Env:RUSTFLAGS"

cargo flamegraph --profile profiling --no-default-features --features profiling `
    --bin dynamapper --package dynamapper `
    $args

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "Flamegraph complete."
