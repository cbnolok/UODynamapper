# scripts/build/windows/bloat.ps1
# Nightly build analysis - Safe Tier (Windows)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Env:RUSTC_WRAPPER = Join-Path $scriptDir "..\common\rustc_wrapper.ps1"

Write-Host "Running Windows Nightly Bloat Analysis..."

$Env:RUSTFLAGS = "-Zshare-generics=y -Zlocation-detail=none -Cforce-unwind-tables=no -Csymbol-mangling-version=v0 -Clink-arg=-Wl,--gc-sections $Env:RUSTFLAGS"

$extraArgs = $args

cargo +nightly bloat --release --no-default-features `
    --config 'profile.release.strip=false' `
    -Z build-std=std,panic_abort `
    -Z build-std-features="optimize_for_size" `
    $extraArgs

Write-Host "Analysis finished."
