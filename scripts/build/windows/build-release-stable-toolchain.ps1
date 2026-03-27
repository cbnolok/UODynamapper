# scripts/build/windows/build-release-stable-toolchain.ps1
# Stable toolchain release build (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows stable release build..."

# Stable toolchain release flags.
# Windows uses LLD linker via config.toml
$Env:RUSTFLAGS = "-C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--no-allow-shlib-undefined -C link-arg=-Wl,--icf=all -C link-arg=-Wl,--strip-all $Env:RUSTFLAGS"

cargo build --release --locked --no-default-features `
    --bin dynamapper --package dynamapper `
    $args

Write-Host "Build complete."
