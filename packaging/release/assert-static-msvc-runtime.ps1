[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Executable
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$executablePath = [System.IO.Path]::GetFullPath($Executable)
if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
    throw "Release executable was not found at '$executablePath'."
}

$dumpBin = Get-Command 'dumpbin.exe' -ErrorAction SilentlyContinue
if ($dumpBin) {
    $dumpBin = $dumpBin.Source
}
else {
    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $vsWhere = Join-Path $programFilesX86 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vsWhere -PathType Leaf)) {
        throw 'Could not find vswhere.exe to locate dumpbin.exe.'
    }

    $dumpBin = @(
        & $vsWhere `
            -latest `
            -products '*' `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe'
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

    if (-not $dumpBin) {
        throw 'Could not find dumpbin.exe to inspect the release executable.'
    }
}

$dependencies = & $dumpBin /nologo /dependents $executablePath
if ($LASTEXITCODE -ne 0) {
    throw "Inspecting '$executablePath' failed with exit code $LASTEXITCODE."
}

$externalRuntime = $dependencies | Select-String -Pattern '(?i)\b(?:vcruntime[a-z0-9_]*|msvcp[a-z0-9_]*|msvcr[a-z0-9_]*|concrt[a-z0-9_]*|vccorlib[a-z0-9_]*|ucrtbase|api-ms-win-crt-[^\s]+)\.dll\b'
if ($externalRuntime) {
    $libraries = $externalRuntime.Matches.Value | Sort-Object -Unique
    throw "The release executable depends on an external Microsoft runtime: $($libraries -join ', ')."
}
