# scripts/build/windows/build-debug.ps1
# Debug build (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows debug build..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = "$scriptDir\..\common\rustc_wrapper.ps1"

cargo build --bin dynamapper --package dynamapper $args

Write-Host "Build complete."
