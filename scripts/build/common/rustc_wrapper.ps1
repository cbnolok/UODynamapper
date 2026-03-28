# scripts/build/common/rustc_wrapper.ps1
# Windows PowerShell wrapper for rustc

$rustc = $args[0]
$remainingArgs = $args[1..($args.Count - 1)]

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
        $ErrorActionPreference = "SilentlyContinue"
        & sccache $rustc @ExtraArgs 2>$null
        if ($LASTEXITCODE -ne 0) {
            & $rustc @ExtraArgs
        }
    } else {
        & $rustc @ExtraArgs
    }
}

if ($isMyCrate) {
    Invoke-Rustc -ExtraArgs $remainingArgs
} else {
    # Prune dependencies with no-fmt-debug.
    Invoke-Rustc -ExtraArgs ($remainingArgs + @("-Zfmt-debug=none"))
}
