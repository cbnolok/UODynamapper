# scripts/build/common/rustc_wrapper.ps1
# Windows PowerShell wrapper for rustc

$rustc = $args[0]
$remainingArgs = $args[1..($args.Count - 1)]

# Pass through version/query flags directly without interception
if ($remainingArgs -join " " -match "-vV|--version" -or $remainingArgs.Count -eq 0) {
    & $rustc @remainingArgs
    exit $LASTEXITCODE
}

$crateName = ""
for ($i = 0; $i -lt $remainingArgs.Count; $i++) {
    if ($remainingArgs[$i] -eq "--crate-name") {
        $crateName = $remainingArgs[$i + 1]
    }
}

$myCrates = @("dynamapper", "uocf")
$isMyCrate = $myCrates -contains $crateName

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
