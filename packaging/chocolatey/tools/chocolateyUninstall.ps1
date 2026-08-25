$ErrorActionPreference = 'Stop'

$toolsDirectory = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ragavan = Join-Path $toolsDirectory 'ragavan.exe'

if (-not (Test-Path -LiteralPath $ragavan -PathType Leaf)) {
    Write-Warning "Ragavan executable was not found at '$ragavan'; skipping PowerShell integration removal."
    return
}

& $ragavan uninstall powershell
if ($LASTEXITCODE -ne 0) {
    throw "Ragavan PowerShell integration removal failed with exit code $LASTEXITCODE."
}
