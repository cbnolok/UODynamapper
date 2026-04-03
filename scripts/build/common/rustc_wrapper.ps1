# scripts/build/common/rustc_wrapper.ps1
# Windows PowerShell wrapper for rustc

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

# Pass through version/query flags directly without sccache to keep cargo probes clean.
$joinedArgs = $remainingArgs -join " "
if ($remainingArgs.Count -eq 0 -or $joinedArgs -match "(^|\s)(-vV|--version)($|\s)" -or $joinedArgs -match "--print(=|\s)") {
    & $rustc @remainingArgs
    exit $LASTEXITCODE
}

$sccacheBin = Get-Command sccache -ErrorAction SilentlyContinue

# Helper function to run rustc with optional sccache fallback
function Invoke-Rustc {
    param([string[]]$ExtraArgs)
    if ($sccacheBin) {
        # Try sccache, fall back to direct rustc on failure
        # Only suppress stderr for the sccache attempt, not for rustc
        $prevErrorAction = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        & sccache $rustc @ExtraArgs
        $sccacheExitCode = $LASTEXITCODE
        $ErrorActionPreference = $prevErrorAction

        if ($sccacheExitCode -ne 0) {
            # sccache failed, fall back to direct rustc
            & $rustc @ExtraArgs
        }
    } else {
        & $rustc @ExtraArgs
    }
}

Invoke-Rustc -ExtraArgs $remainingArgs
