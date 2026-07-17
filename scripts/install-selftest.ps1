# Hermetic contract tests for the withpointbreak/pointbreak Windows installer.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $repoRoot "scripts/install.ps1"
$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("pointbreak-installer-test-" + [Guid]::NewGuid())
$savedEnvironment = @{
    FixtureRoot = $env:POINTBREAK_INSTALLER_FIXTURE_ROOT
    FixtureRunner = $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER
    FixtureVersion = $env:POINTBREAK_INSTALLER_FIXTURE_VERSION
    InstalledVersion = $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION
    FailReplace = $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE
}

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "").ToLowerInvariant()
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function New-ReleaseArchive {
    param(
        [Parameter(Mandatory = $true)][string]$CandidateVersion,
        [Parameter(Mandatory = $true)][string]$InstalledVersion,
        [switch]$ExtraEntry
    )

    if (Test-Path -LiteralPath $payloadDir) {
        Remove-Item -LiteralPath $payloadDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $payloadDir | Out-Null

    $fixtureExecutable = (Get-Process -Id $PID).Path
    Copy-Item -LiteralPath $fixtureExecutable -Destination (Join-Path $payloadDir "pointbreak.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $payloadDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "NOTICE") -Destination $payloadDir
    $paths = @(
        (Join-Path $payloadDir "pointbreak.exe"),
        (Join-Path $payloadDir "LICENSE"),
        (Join-Path $payloadDir "NOTICE")
    )
    if ($ExtraEntry) {
        $extra = Join-Path $payloadDir "unexpected.txt"
        Set-Content -LiteralPath $extra -Value "unexpected payload" -Encoding utf8
        $paths += $extra
    }

    $archivePath = Join-Path $releaseDir $archive
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    Compress-Archive -LiteralPath $paths -DestinationPath $archivePath
    $env:POINTBREAK_INSTALLER_FIXTURE_VERSION = $CandidateVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION = $InstalledVersion
}

function Set-ValidChecksum {
    $archivePath = Join-Path $releaseDir $archive
    $checksum = Get-FileSha256 -Path $archivePath
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$checksum  $archive" `
        -Encoding ascii
}

function Set-InvalidChecksum {
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$('0' * 64)  $archive" `
        -Encoding ascii
}

function Reset-UpgradeFixture {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Set-Content -LiteralPath $destination -Value "previous pointbreak bytes" -Encoding utf8
    [IO.File]::WriteAllBytes($neighbor, [byte[]](0, 255, 17, 83, 0, 104, 111, 114, 101))
    $script:previousHash = Get-FileSha256 -Path $destination
    $script:neighborHash = Get-FileSha256 -Path $neighbor
}

function Assert-NeighborUnchanged {
    if ((Get-FileSha256 -Path $neighbor) -ne $neighborHash) {
        throw "installer changed the neighboring file"
    }
}

function Assert-PreviousRestored {
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
        throw "installer stranded a missing Pointbreak destination"
    }
    if ((Get-FileSha256 -Path $destination) -ne $previousHash) {
        throw "installer did not restore the previous Pointbreak destination"
    }
    Assert-NeighborUnchanged
    $transactionFiles = @(Get-ChildItem -LiteralPath $installDir -Force | Where-Object {
        $_.Name -like ".pointbreak-install-*" -or $_.Name -like ".pointbreak-backup-*"
    })
    if ($transactionFiles.Count -ne 0) {
        throw "installer left transaction files behind"
    }
}

function Invoke-Installer {
    return & $installer -Version $tag -InstallDir $installDir -NoModifyPath 6>&1
}

function Assert-InstallerFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Scenario,
        [Parameter(Mandatory = $true)][string]$MessagePattern
    )

    $failed = $false
    try {
        Invoke-Installer | Out-Null
    }
    catch {
        if ($_.Exception.Message -notmatch $MessagePattern) {
            throw
        }
        $failed = $true
    }
    if (-not $failed) {
        throw "installer accepted $Scenario"
    }
    Assert-PreviousRestored
}

try {
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    $target = switch ($architecture) {
        "X64" { "win32-x64" }
        "Arm64" { "win32-arm64" }
        default { throw "Unsupported self-test architecture: $architecture" }
    }

    $tag = "v9.8.7-test"
    $version = $tag.Substring(1)
    $archive = "pointbreak-$version-$target.zip"
    $releaseDir = Join-Path (Join-Path $tempDir "releases") $tag
    $payloadDir = Join-Path $tempDir "payload"
    $installDir = Join-Path $tempDir "bin"
    $destination = Join-Path $installDir "pointbreak.exe"
    $neighbor = Join-Path $installDir "shore.exe"
    $runner = Join-Path $tempDir "version-runner.ps1"
    New-Item -ItemType Directory -Path $releaseDir, $installDir | Out-Null

    @'
param(
    [Parameter(Mandatory = $true)][string]$CandidatePath,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$CommandArguments
)
if (($CommandArguments -join " ") -cne "version --format json") {
    throw "candidate was not invoked with exact version arguments"
}
$candidateVersion = $env:POINTBREAK_INSTALLER_FIXTURE_VERSION
if ([IO.Path]::GetFileName($CandidatePath) -ceq "pointbreak.exe") {
    $candidateVersion = $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION
}
[ordered]@{
    schema = "pointbreak.version"
    version = 1
    cliVersion = $candidateVersion
    documents = [ordered]@{ "pointbreak.version" = 1 }
    diagnostics = @()
} | ConvertTo-Json -Compress
'@ | Set-Content -LiteralPath $runner -Encoding utf8

    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = Join-Path $tempDir "releases"
    $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER = $runner

    $helpOutput = Get-Help $installer -Full | Out-String
    if ($helpOutput -notmatch "Pointbreak Review") {
        throw "installer help does not teach Pointbreak Review"
    }
    if ($helpOutput -match "(?i)shore") {
        throw "installer help teaches a second executable"
    }

    # Fresh install: create only a regular Pointbreak executable.
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version
    Set-ValidChecksum
    $freshOutput = (Invoke-Installer | Out-String)
    $freshOutputNormalized = ($freshOutput -replace "\s+", " ").Trim()
    Write-Host $freshOutput
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
        throw "installer did not create pointbreak.exe"
    }
    if (((Get-Item -LiteralPath $destination).Attributes -band
            [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "installer created a pointbreak.exe symlink"
    }
    if (Test-Path -LiteralPath $neighbor) {
        throw "installer created a second executable"
    }
    if ($freshOutputNormalized -notmatch [Regex]::Escape("Installed Pointbreak Review $version to $destination")) {
        throw "installer success output omitted the installed Pointbreak version"
    }
    if ($freshOutputNormalized -notmatch "run: pointbreak --help") {
        throw "installer success output omitted Pointbreak help guidance"
    }
    if ($freshOutputNormalized -match "(?i)shore") {
        throw "installer success output teaches a second executable"
    }

    # Upgrade: replace only Pointbreak and preserve an arbitrary neighbor byte-for-byte.
    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version
    Set-ValidChecksum
    Invoke-Installer | Out-Null
    $payloadHash = Get-FileSha256 -Path (Join-Path $payloadDir "pointbreak.exe")
    if ((Get-FileSha256 -Path $destination) -ne $payloadHash) {
        throw "installer did not replace pointbreak.exe with the archive payload"
    }
    Assert-NeighborUnchanged

    # Every failure must preserve the prior destination and the arbitrary neighbor.
    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version
    Set-InvalidChecksum
    Assert-InstallerFailure -Scenario "checksum failure" -MessagePattern "Checksum mismatch"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version -ExtraEntry
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "archive layout failure" -MessagePattern "invalid archive layout"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion "9.8.6-test" -InstalledVersion "9.8.6-test"
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "version mismatch" -MessagePattern "version document did not match"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version
    Set-ValidChecksum
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = "1"
    Assert-InstallerFailure -Scenario "replacement failure" -MessagePattern "could not replace"
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = $null

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion "9.8.6-test"
    Set-ValidChecksum
    Assert-InstallerFailure `
        -Scenario "post-replacement verification failure" `
        -MessagePattern "installed Pointbreak version document did not match"

    $installerSource = Get-Content -LiteralPath $installer -Raw
    if ($installerSource -match "(?i)shore") {
        throw "installer implementation references a neighboring executable"
    }

    Write-Host "install.ps1 self-test ok"
}
finally {
    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = $savedEnvironment.FixtureRoot
    $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER = $savedEnvironment.FixtureRunner
    $env:POINTBREAK_INSTALLER_FIXTURE_VERSION = $savedEnvironment.FixtureVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION = $savedEnvironment.InstalledVersion
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = $savedEnvironment.FailReplace
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
