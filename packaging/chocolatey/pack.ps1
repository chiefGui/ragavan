[CmdletBinding()]
param(
    [Parameter()]
    [string] $Version,

    [Parameter()]
    [string] $SourceReference
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-LastExitCode {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Operation
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$Operation failed with exit code $LASTEXITCODE."
    }
}

function Write-Utf8File {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path,

        [Parameter(Mandatory = $true)]
        [string] $Content
    )

    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Find-DumpBin {
    $command = Get-Command 'dumpbin.exe' -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $vswhere = Join-Path $programFilesX86 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
        $candidates = @(& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe')
        if ($LASTEXITCODE -eq 0) {
            $candidate = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
            if ($candidate) {
                return $candidate
            }
        }
    }

    throw 'Could not find dumpbin.exe to verify the release binary dependencies.'
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$targetDirectory = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
$distributionDirectory = [System.IO.Path]::GetFullPath((Join-Path $targetDirectory 'distribution'))
$targetPrefix = $targetDirectory.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar

if (-not $distributionDirectory.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Distribution directory '$distributionDirectory' must be inside '$targetDirectory'."
}

Push-Location $repositoryRoot
try {
    $metadataJson = & cargo metadata --locked --no-deps --format-version 1
    Assert-LastExitCode 'Reading Cargo metadata'
    $metadata = ($metadataJson -join [Environment]::NewLine) | ConvertFrom-Json
    $packages = @($metadata.packages | Where-Object { $_.name -eq 'ragavan' })

    if ($packages.Count -ne 1) {
        throw "Expected exactly one Cargo package named 'ragavan'; found $($packages.Count)."
    }

    $packageVersion = [string] $packages[0].version
    if ($Version -and $Version -ne $packageVersion) {
        throw "Requested version '$Version' does not match Cargo version '$packageVersion'."
    }

    $resolvedSourceReference = if ($SourceReference) {
        $SourceReference
    }
    else {
        "v$packageVersion"
    }
    if ($resolvedSourceReference -notmatch '^[A-Za-z0-9][A-Za-z0-9._/-]*$') {
        throw "Source reference '$resolvedSourceReference' contains unsupported characters."
    }

    & cargo build --locked --release --package ragavan
    Assert-LastExitCode 'Building Ragavan'

    $builtExecutable = Join-Path ([string] $metadata.target_directory) 'release\ragavan.exe'
    if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) {
        throw "Built executable was not found at '$builtExecutable'."
    }

    $dumpbin = Find-DumpBin
    $dependencies = & $dumpbin /nologo /dependents $builtExecutable
    Assert-LastExitCode 'Inspecting Ragavan dependencies'
    $externalRuntime = $dependencies | Select-String -Pattern '(?i)\b(?:vcruntime\d*|msvcp\d*|ucrtbase|api-ms-win-crt-[^\s]+)\.dll\b'
    if ($externalRuntime) {
        throw "Ragavan must statically link the Microsoft C runtime; found $($externalRuntime.Matches.Value -join ', ')."
    }

    if (Test-Path -LiteralPath $distributionDirectory) {
        Remove-Item -LiteralPath $distributionDirectory -Recurse -Force
    }

    $stageDirectory = Join-Path $distributionDirectory "chocolatey\ragavan.$packageVersion"
    $stageToolsDirectory = Join-Path $stageDirectory 'tools'
    New-Item -ItemType Directory -Path $stageToolsDirectory -Force | Out-Null

    $releaseExecutable = Join-Path $distributionDirectory 'ragavan.exe'
    Copy-Item -LiteralPath $builtExecutable -Destination $releaseExecutable

    $checksum = (Get-FileHash -LiteralPath $releaseExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8File -Path (Join-Path $distributionDirectory 'ragavan.exe.sha256') -Content "$checksum  ragavan.exe`n"

    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'ragavan.nuspec') -Destination $stageDirectory
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tools\chocolateyInstall.ps1') -Destination $stageToolsDirectory
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tools\chocolateyUninstall.ps1') -Destination $stageToolsDirectory
    Copy-Item -LiteralPath $releaseExecutable -Destination $stageToolsDirectory
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination (Join-Path $stageToolsDirectory 'LICENSE.txt')

    $verificationTemplate = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'VERIFICATION.txt.in')
    $verification = $verificationTemplate.Replace('{{SOURCE_REFERENCE}}', $resolvedSourceReference).Replace('{{SHA256}}', $checksum)
    Write-Utf8File -Path (Join-Path $stageToolsDirectory 'VERIFICATION.txt') -Content $verification

    $nuspec = Join-Path $stageDirectory 'ragavan.nuspec'
    & choco pack $nuspec --version $packageVersion --output-directory $distributionDirectory
    Assert-LastExitCode 'Packing the Chocolatey package'

    $package = Join-Path $distributionDirectory "ragavan.$packageVersion.nupkg"
    if (-not (Test-Path -LiteralPath $package -PathType Leaf)) {
        throw "Chocolatey package was not found at '$package'."
    }

    Write-Host "Created release artifacts in '$distributionDirectory'."
}
finally {
    Pop-Location
}
