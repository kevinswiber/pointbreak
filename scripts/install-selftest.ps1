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
    FixtureIdentity = $env:POINTBREAK_INSTALLER_FIXTURE_IDENTITY
    InstalledIdentity = $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_IDENTITY
    FailReplace = $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE
}

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "").ToLowerInvariant() }
        finally { $sha256.Dispose() }
    }
    finally { $stream.Dispose() }
}

function New-ReleaseArchive {
    param(
        [Parameter(Mandatory = $true)][string]$CandidateVersion,
        [Parameter(Mandatory = $true)][string]$InstalledVersion,
        [Parameter(Mandatory = $true)][string]$CandidateIdentity,
        [Parameter(Mandatory = $true)][string]$InstalledIdentity,
        [switch]$ExtraEntry
    )
    if (Test-Path -LiteralPath $payloadDir) { Remove-Item -LiteralPath $payloadDir -Recurse -Force }
    New-Item -ItemType Directory -Path $payloadDir -Force | Out-Null
    New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null

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
    if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath -Force }
    Compress-Archive -LiteralPath $paths -DestinationPath $archivePath
    $env:POINTBREAK_INSTALLER_FIXTURE_VERSION = $CandidateVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION = $InstalledVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_IDENTITY = $CandidateIdentity
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_IDENTITY = $InstalledIdentity
}

function Set-ValidChecksum {
    $archivePath = Join-Path $releaseDir $archive
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$(Get-FileSha256 -Path $archivePath)  $archive" -Encoding ascii
}
function Set-InvalidChecksum {
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$('0' * 64)  $archive" -Encoding ascii
}
function Reset-UpgradeFixture {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Set-Content -LiteralPath $destination -Value "previous pointbreak bytes" -Encoding utf8
    [IO.File]::WriteAllBytes($neighbor, [byte[]](0, 255, 17, 83, 0, 104, 111, 114, 101))
    $script:previousHash = Get-FileSha256 -Path $destination
    $script:neighborHash = Get-FileSha256 -Path $neighbor
}
function Assert-NeighborUnchanged {
    if ((Get-FileSha256 -Path $neighbor) -ne $neighborHash) { throw "installer changed the neighboring file" }
}
function Assert-PreviousRestored {
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { throw "installer stranded a missing destination" }
    if ((Get-FileSha256 -Path $destination) -ne $previousHash) { throw "installer did not restore the previous destination" }
    Assert-NeighborUnchanged
    $transactionFiles = @(Get-ChildItem -LiteralPath $installDir -Force | Where-Object {
        $_.Name -like ".pointbreak-install-*" -or $_.Name -like ".pointbreak-backup-*"
    })
    if ($transactionFiles.Count -ne 0) { throw "installer left transaction files behind" }
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
    try { Invoke-Installer | Out-Null }
    catch {
        if ($_.Exception.Message -notmatch $MessagePattern) { throw }
        $failed = $true
    }
    if (-not $failed) { throw "installer accepted $Scenario" }
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
$documentVersion = $env:POINTBREAK_INSTALLER_FIXTURE_VERSION
$identity = $env:POINTBREAK_INSTALLER_FIXTURE_IDENTITY
if ([IO.Path]::GetFileName($CandidatePath) -ceq "pointbreak.exe") {
    $documentVersion = $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION
    $identity = $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_IDENTITY
}
$commit = "0123456789abcdef0123456789abcdef01234567"
if ($identity -eq "malformed-document") {
    Write-Output '{not-json'
    return
}
$document = [ordered]@{
    schema = "pointbreak.version"
    version = 1
    cliVersion = $documentVersion
    documents = [ordered]@{ "pointbreak.version" = 1 }
    diagnostics = @()
}
switch ($identity) {
    "exact" {
        $document["build"] = [ordered]@{ source = "git"; commit = $commit; describe = "v$documentVersion"; dirty = $false }
    }
    "additive-fields-and-order" {
        $document = [ordered]@{
            extra = [ordered]@{ ignored = $true }
            diagnostics = @()
            build = [ordered]@{ dirty = $false; describe = "v$documentVersion"; extraBuild = "ignored"; commit = $commit; source = "git" }
            documents = [ordered]@{ "unrelated.document" = 7; "pointbreak.version" = 1 }
            cliVersion = $documentVersion
            version = 1
            schema = "pointbreak.version"
        }
    }
    "wrong-tag" { $document["build"] = [ordered]@{ source = "git"; commit = $commit; describe = "v0.0.0"; dirty = $false } }
    "dirty-build" { $document["build"] = [ordered]@{ source = "git"; commit = $commit; describe = "v$documentVersion-dirty"; dirty = $true } }
    "package-build" { $document["build"] = [ordered]@{ source = "package"; commit = $null; describe = "package:$documentVersion"; dirty = $false } }
    "short-commit" { $document["build"] = [ordered]@{ source = "git"; commit = "0123456"; describe = "v$documentVersion"; dirty = $false } }
    "missing-build" { }
    "null-build" { $document["build"] = $null }
    default { throw "unknown fixture identity: $identity" }
}
$document | ConvertTo-Json -Depth 8 -Compress
'@ | Set-Content -LiteralPath $runner -Encoding utf8

    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = Join-Path $tempDir "releases"
    $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER = $runner

    $helpOutput = Get-Help $installer -Full | Out-String
    if ($helpOutput -notmatch "Pointbreak Review" -or $helpOutput -match "(?i)shore") {
        throw "installer help contract failed"
    }

    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-ValidChecksum
    $freshOutput = (Invoke-Installer | Out-String)
    $freshOutputNormalized = ($freshOutput -replace "\s+", " ").Trim()
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { throw "installer did not create pointbreak.exe" }
    if (((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "installer created a pointbreak.exe symlink"
    }
    if (Test-Path -LiteralPath $neighbor) { throw "installer created a second executable" }
    if ($freshOutputNormalized -notmatch [Regex]::Escape("Installed Pointbreak Review $version to")) {
        throw "installer success output omitted version"
    }
    if ($freshOutputNormalized -notmatch "run: pointbreak --help") {
        throw "installer success output omitted Pointbreak help guidance"
    }
    if ($freshOutputNormalized -match "(?i)shore") {
        throw "installer success output teaches a second executable"
    }

    # additive-fields-and-order
    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity additive-fields-and-order -InstalledIdentity additive-fields-and-order
    Set-ValidChecksum
    Invoke-Installer | Out-Null
    Assert-NeighborUnchanged

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-ValidChecksum
    Invoke-Installer | Out-Null
    if ((Get-FileSha256 -Path $destination) -ne (Get-FileSha256 -Path (Join-Path $payloadDir "pointbreak.exe"))) {
        throw "installer did not replace pointbreak.exe"
    }
    Assert-NeighborUnchanged

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-InvalidChecksum
    Assert-InstallerFailure -Scenario "checksum-failure" -MessagePattern "Checksum mismatch"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity exact -InstalledIdentity exact -ExtraEntry
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "archive-layout-failure" -MessagePattern "invalid archive layout"

    foreach ($scenario in @("wrong-tag", "dirty-build", "package-build", "short-commit", "malformed-document", "null-build")) {
        Reset-UpgradeFixture
        New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
            -CandidateIdentity $scenario -InstalledIdentity $scenario
        Set-ValidChecksum
        Assert-InstallerFailure -Scenario $scenario -MessagePattern "version document did not match"
    }

    # missing-build-after-v0.7.0
    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity missing-build -InstalledIdentity missing-build
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "missing-build-after-v0.7.0" -MessagePattern "version document did not match"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion "9.8.6-test" -InstalledVersion "9.8.6-test" `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "version-mismatch" -MessagePattern "version document did not match"

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-ValidChecksum
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = "1"
    Assert-InstallerFailure -Scenario "replacement-failure" -MessagePattern "could not replace"
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = $null

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion "9.8.6-test" `
        -CandidateIdentity exact -InstalledIdentity exact
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "post-replacement-verification-failure" -MessagePattern "version document did not match"

    # legacy-v0.7.0
    $originalTag = $tag
    $originalVersion = $version
    $originalArchive = $archive
    $originalReleaseDir = $releaseDir
    $tag = "v0.7.0"
    $version = "0.7.0"
    $archive = "pointbreak-$version-$target.zip"
    $releaseDir = Join-Path (Join-Path $tempDir "releases") $tag
    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity missing-build -InstalledIdentity missing-build
    Set-ValidChecksum
    Invoke-Installer | Out-Null
    Assert-NeighborUnchanged

    Reset-UpgradeFixture
    New-ReleaseArchive -CandidateVersion $version -InstalledVersion $version `
        -CandidateIdentity null-build -InstalledIdentity null-build
    Set-ValidChecksum
    Assert-InstallerFailure -Scenario "legacy-null-build" -MessagePattern "version document did not match"
    $tag = $originalTag
    $version = $originalVersion
    $archive = $originalArchive
    $releaseDir = $originalReleaseDir

    $installerSource = Get-Content -LiteralPath $installer -Raw
    if ($installerSource -match "(?i)shore") { throw "installer implementation references a neighboring executable" }
    Write-Host "install.ps1 self-test ok"
}
finally {
    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = $savedEnvironment.FixtureRoot
    $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER = $savedEnvironment.FixtureRunner
    $env:POINTBREAK_INSTALLER_FIXTURE_VERSION = $savedEnvironment.FixtureVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_VERSION = $savedEnvironment.InstalledVersion
    $env:POINTBREAK_INSTALLER_FIXTURE_IDENTITY = $savedEnvironment.FixtureIdentity
    $env:POINTBREAK_INSTALLER_FIXTURE_INSTALLED_IDENTITY = $savedEnvironment.InstalledIdentity
    $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE = $savedEnvironment.FailReplace
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
