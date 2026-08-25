[CmdletBinding()]
param(
    [Parameter()]
    [string] $Version
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

    & cargo build --locked --release --package ragavan
    Assert-LastExitCode 'Building Ragavan'

    $builtExecutable = Join-Path ([string] $metadata.target_directory) 'release\ragavan.exe'
    if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) {
        throw "Built executable was not found at '$builtExecutable'."
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
    $verification = $verificationTemplate.Replace('{{VERSION}}', $packageVersion).Replace('{{SHA256}}', $checksum)
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
