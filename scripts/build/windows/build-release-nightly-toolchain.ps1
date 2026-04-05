# scripts/build/windows/build-release-nightly-toolchain.ps1
# Nightly release build with size optimizations (Windows)

param(
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\.."
Set-Location $rootDir

Write-Host "Running Windows nightly release build..."

# Use sccache directly when available. Avoid custom rustc wrappers on Windows
# to keep rustc probe output stable (e.g. --print=cfg / -vV).
if (Get-Command sccache -ErrorAction SilentlyContinue) {
    $Env:RUSTC_WRAPPER = "sccache"
} else {
    Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
}

# Stable release flags plus nightly-only size/build-std flags.
# Windows (MSVC) works well with gc-sections if using LLD (via config.toml)
# Note: MSVC linker uses /OPT:REF instead of --gc-sections, but Cargo handles
# these defaults for MSVC release builds.
#$Env:RUSTFLAGS = "-C force-unwind-tables=no -C symbol-mangling-version=v0 -Z share-generics=y -Z location-detail=none $Env:RUSTFLAGS"

# Stable release flags plus nightly-only size/build-std flags.
# Do not disable unwind tables on Windows targets: x86_64/aarch64 Windows uses
# mandatory unwind metadata for SEH and stack walking.
$Env:RUSTFLAGS = "-C symbol-mangling-version=v0 -Z share-generics=y -Z location-detail=none $Env:RUSTFLAGS"

$targetArgs = @()
$featureArgs = @()
$expectedBinary = "target/release/dynamapper.exe"
if (![string]::IsNullOrWhiteSpace($Target)) {
    $targetArgs = @("--target", $Target)
    $expectedBinary = "target/$Target/release/dynamapper.exe"
}

if (![string]::IsNullOrWhiteSpace($Env:CARGO_FEATURES)) {
    $featureArgs = @("--features", $Env:CARGO_FEATURES)
}

cargo +nightly build --release --locked --no-default-features `
    --bin dynamapper --package dynamapper `
    -Z build-std=std,panic_abort `
    -Z build-std-features=optimize_for_size `
    @featureArgs `
    @targetArgs `
    $args

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

# Verify the binary was created
if (!(Test-Path $expectedBinary)) {
    Write-Error "Build succeeded but dynamapper.exe was not found at '$expectedBinary'"
    exit 1
}

Write-Host "Build complete."
