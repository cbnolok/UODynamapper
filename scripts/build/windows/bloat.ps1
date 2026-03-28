# scripts/build/windows/bloat.ps1
# Nightly build analysis using cargo-bloat - Safe Tier (Windows)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

# Set RUSTC_WRAPPER to the wrapper script (absolute path)
$Env:RUSTC_WRAPPER = (Resolve-Path "$scriptDir\..\common\rustc_wrapper.ps1").Path

$ARGS = $args
if ($ARGS.Length -eq 0) { $ARGS = @("--crates") }

Write-Host "Running Windows Nightly Bloat Analysis..."

# Windows (MSVC) works well with gc-sections if using LLD (which you are via config.toml)
$Env:RUSTFLAGS = "-Z share-generics=y -Z location-detail=none -C force-unwind-tables=no -C symbol-mangling-version=v0 -C link-arg=-Wl,--gc-sections $Env:RUSTFLAGS"

cargo +nightly bloat --release --no-default-features `
    --config 'profile.release.strip=false' `
    -Z build-std=std,panic_abort `
    -Z build-std-features="optimize_for_size" `
    $ARGS

if ($LASTEXITCODE -ne 0) {
    Write-Error "Bloat analysis failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "Analysis finished."
