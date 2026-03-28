# scripts/build/windows/build-release-stable-toolchain.ps1
# Stable toolchain release build (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows stable release build..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.ps1").Path

# Stable toolchain release flags.
# Windows uses LLD linker via config.toml
$Env:RUSTFLAGS = "-C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--no-allow-shlib-undefined -C link-arg=-Wl,--icf=all -C link-arg=-Wl,--strip-all $Env:RUSTFLAGS"

cargo build --release --locked --no-default-features `
    --bin dynamapper --package dynamapper `
    $args

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

# Verify the binary was created
if (!(Test-Path "target/release/dynamapper.exe")) {
    Write-Error "Build succeeded but dynamapper.exe was not found in target/release/"
    exit 1
}

Write-Host "Build complete."
