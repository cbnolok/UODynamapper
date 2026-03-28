# scripts/build/windows/build-release-nightly-toolchain.ps1
# Nightly release build with size optimizations (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows nightly release build..."

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.ps1").Path

# Stable release flags plus nightly-only size/build-std flags.
# Windows (MSVC) works well with gc-sections if using LLD (which you are via config.toml)
$Env:RUSTFLAGS = "-C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--icf=safe -C link-arg=-Wl,--no-allow-shlib-undefined -C force-unwind-tables=no -C symbol-mangling-version=v0 -Z share-generics=y -Z location-detail=none $Env:RUSTFLAGS"

cargo +nightly build --release --locked --no-default-features `
    --bin dynamapper --package dynamapper `
    -Z build-std=std,panic_abort `
    -Z build-std-features=optimize_for_size `
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
