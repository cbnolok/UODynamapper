# scripts/build/windows/build-flamegraph.ps1
# Flamegraph build: release optimizations with debug symbols and frame pointers (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows flamegraph build (release optimizations + debug symbols, frame pointers)..."

# Flamegraph build: release optimizations with debug symbols and frame pointers.
# - No LTO & No Strip: essential for stack walking
# - force-frame-pointers: essential for profilers
$Env:RUSTFLAGS = "-C force-frame-pointers=yes $Env:RUSTFLAGS"

cargo flamegraph --profile profiling --no-default-features --features profiling `
    --bin dynamapper --package dynamapper `
    $args

Write-Host "Flamegraph complete."
