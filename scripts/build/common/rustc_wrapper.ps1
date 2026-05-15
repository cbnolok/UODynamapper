# scripts/build/common/rustc_wrapper.ps1
#
# --- PURPOSE ---
# This script acts as a Windows-native wrapper for 'rustc' (the Rust compiler). 
# It is invoked by Cargo because it is pointed to by the RUSTC_WRAPPER 
# environment variable (via rustc_wrapper.bat).
#
# --- WHY IS THIS NEEDED? ---
# 1. Environment Constraints: RUSTC_WRAPPER requires a single executable or script.
#    Since we often want to toggle between different tools (like sccache) or apply
#    custom logic per-crate on Windows, we use this script as a stable entry point.
# 2. Interception Logic: This script allows us to inspect the compiler arguments 
#    and modify them. This is crucial for applying platform-specific flags 
#    to specific crates without modifying the global Cargo.toml.
# 3. Justfile Integration: The 'justfile' handles the high-level orchestration 
#    and Windows/Linux detection, then points RUSTC_WRAPPER here when it wants 
#    custom interception logic active on Windows.
#
# --- CURRENT STATE ---
# Currently, this script is a pass-through that executes the compiler directly.
# Future interception logic should be added at the end of the script.

$passThroughArgs = @($args)

if (-not $passThroughArgs -or $passThroughArgs.Count -eq 0) {
    Write-Error "rustc_wrapper.ps1 requires rustc path and arguments"
    exit 1
}

$rustc = $passThroughArgs[0]
$remainingArgs = @()
if ($passThroughArgs.Count -gt 1) {
    $remainingArgs = $passThroughArgs[1..($passThroughArgs.Count - 1)]
}

# Pass through version/query flags directly to keep cargo probes clean.
$joinedArgs = $remainingArgs -join " "
if ($remainingArgs.Count -eq 0 -or $joinedArgs -match "(^|\s)(-vV|--version)($|\s)" -or $joinedArgs -match "--print(=|\s)") {
    & $rustc @remainingArgs
    exit $LASTEXITCODE
}

# Execute the actual compiler
& $rustc @remainingArgs
exit $LASTEXITCODE
