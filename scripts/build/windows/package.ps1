# scripts/build/windows/package.ps1 <artifact_name> [<target_triple>]

param(
    [Parameter(Mandatory=$true)][string]$ArtifactName,
    [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Resolve-Path "$scriptDir\..\..\.."
Set-Location $rootDir

$releaseDir = "target/release"
if (![string]::IsNullOrWhiteSpace($TargetTriple)) {
    $releaseDir = "target/$TargetTriple/release"
}

$destDir = "artifact/$ArtifactName"
New-Item -ItemType Directory -Force -Path "$destDir/utils/cli" | Out-Null
New-Item -ItemType Directory -Force -Path "$destDir/utils/gui" | Out-Null
New-Item -ItemType Directory -Force -Path "$destDir/utils/debugging" | Out-Null

Write-Host "Packaging $ArtifactName from $releaseDir..."

# Main binary
Copy-Item -Path "$releaseDir/dynamapper.exe" -Destination "$destDir/"

# CLI Utilities
$cliUtils = @(
    "udd-pack",
    "udd-tool",
    "uop-tool",
    "cc-uop-mul-converter",
    "uop-dict-populator"
)

foreach ($util in $cliUtils) {
    $src = "$releaseDir/$util.exe"
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination "$destDir/utils/cli/"
    } else {
        Write-Warning "CLI Utility $util not found at $src"
    }
}

# GUI Utilities
$guiUtils = @(
    "udd-conv-gui",
    "uddp-inspector-gui",
    "uop-inspector-gui",
    "uop-dict-populator-gui"
)

foreach ($util in $guiUtils) {
    $src = "$releaseDir/$util.exe"
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination "$destDir/utils/gui/"
    } else {
        Write-Warning "GUI Utility $util not found at $src"
    }
}

# Debugging Utilities
$debugUtils = @(
    "texture-scanner"
)

foreach ($util in $debugUtils) {
    $src = "$releaseDir/$util.exe"
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination "$destDir/utils/debugging/"
    } else {
        Write-Warning "Debugging Utility $util not found at $src"
    }
}

# Assets
Copy-Item -Path "assets" -Destination "$destDir/" -Recurse

# Documentation
if (Test-Path "README.md") {
    Copy-Item -Path "README.md" -Destination "$destDir/"
}

Write-Host "Packaging complete: $destDir"
