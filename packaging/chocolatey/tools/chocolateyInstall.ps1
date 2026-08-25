$ErrorActionPreference = 'Stop'

$toolsDirectory = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ragavan = Join-Path $toolsDirectory 'ragavan.exe'

if (-not (Test-Path -LiteralPath $ragavan -PathType Leaf)) {
    throw "Ragavan executable was not found at '$ragavan'."
}

& $ragavan install powershell
if ($LASTEXITCODE -ne 0) {
    throw "Ragavan PowerShell integration failed with exit code $LASTEXITCODE."
}

Write-Host 'Open a new PowerShell session to activate Ragavan.'
