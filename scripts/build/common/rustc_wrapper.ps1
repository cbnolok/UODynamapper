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

if ($isMyCrate) {
    & $rustc @remainingArgs
} else {
    # Prune dependencies with no-fmt-debug.
    & $rustc @remainingArgs -Zfmt-debug=none
}
