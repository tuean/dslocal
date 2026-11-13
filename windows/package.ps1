<#
.SYNOPSIS
    Builds the Windows app and packages it as a portable .zip plus an NSIS .exe.

.DESCRIPTION
    Run from windows/ (or anywhere; the script cds itself):
        pwsh -File windows/package.ps1 -Version 0.1.0
    Outputs to ..\dist\ :
        DSLocal-<version>-windows-x64.zip
        DSLocal-<version>-windows-x64-setup.exe
    The NSIS installer is skipped (zip only) when makensis is not on PATH.
#>
param(
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot
try {
    $dist = Join-Path (Resolve-Path "..") "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null

    Write-Host "[package] cargo build --release"
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $exe = "target\release\dslocal.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe" }

    $zip = Join-Path $dist "DSLocal-$Version-windows-x64.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path $exe -DestinationPath $zip -CompressionLevel Optimal
    Write-Host "[package] zip: $zip"

    $makensis = Get-Command makensis -ErrorAction SilentlyContinue
    if (-not $makensis) {
        Write-Warning "[package] makensis not found; skipping the installer (.zip only)"
        return
    }

    $setup = "DSLocal-$Version-windows-x64-setup.exe"
    & makensis "/DAPP_VERSION=$Version" "/DSRC_EXE=$exe" "/DOUT_FILE=$setup" installer.nsi
    if ($LASTEXITCODE -ne 0) { throw "makensis failed" }

    $built = Join-Path $PSScriptRoot $setup
    if (-not (Test-Path $built)) { throw "makensis reported success but $built is missing" }
    Move-Item -Force $built (Join-Path $dist $setup)
    Write-Host "[package] installer: $(Join-Path $dist $setup)"

    Get-ChildItem $dist | ForEach-Object { "{0,10:N1} KB  {1}" -f ($_.Length / 1KB), $_.Name }
}
finally {
    Pop-Location
}
