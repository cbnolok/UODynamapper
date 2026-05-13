# scripts/build/windows/build-debug.ps1
# Debug build (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows debug build..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.bat").Path

cargo build --workspace $args

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

# Verify the binary was created
if (!(Test-Path "target/debug/dynamapper.exe")) {
    Write-Error "Build succeeded but dynamapper.exe was not found in target/debug/"
    exit 1
}

Write-Host "Build complete."
