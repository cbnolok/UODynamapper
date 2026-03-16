# scripts/build/windows/build.ps1
# Nightly build with extreme size optimizations - Safe Tier (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Env:RUSTC_WRAPPER = Join-Path $scriptDir "..\common\rustc_wrapper.ps1"

Write-Host "Running Windows Nightly Build..."

# Windows (MSVC) works well with gc-sections if using LLD (which you are via config.toml)
$Env:RUSTFLAGS = "-Zshare-generics=y -Zlocation-detail=none -Cforce-unwind-tables=no -Csymbol-mangling-version=v0 -Clink-arg=-Wl,--gc-sections $Env:RUSTFLAGS"

cargo +nightly build --release --locked --no-default-features `
    -Z build-std=std,panic_abort `
    -Z build-std-features="optimize_for_size" `
    $args

Write-Host "Build complete."
