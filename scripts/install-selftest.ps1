# Hermetic smoke tests for scripts/install.ps1. Run on Windows.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("pointbreak-installer-test-" + [Guid]::NewGuid())
$previousFixtureRoot = $env:POINTBREAK_INSTALLER_FIXTURE_ROOT

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
    $releaseDir = Join-Path $tempDir "releases\$tag"
    $payloadDir = Join-Path $tempDir "payload"
    $installDir = Join-Path $tempDir "bin"
    New-Item -ItemType Directory -Path $releaseDir, $payloadDir | Out-Null

    $standaloneExecutable = (Get-Command curl.exe -ErrorAction Stop).Source
    Copy-Item -LiteralPath $standaloneExecutable -Destination (Join-Path $payloadDir "shore.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $payloadDir
    Copy-Item -LiteralPath (Join-Path $repoRoot "NOTICE") -Destination $payloadDir
    Compress-Archive -Path (Join-Path $payloadDir "*") -DestinationPath (Join-Path $releaseDir $archive)

    $archivePath = Join-Path $releaseDir $archive
    $checksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$checksum  $archive" `
        -Encoding ascii

    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = Join-Path $tempDir "releases"
    & (Join-Path $repoRoot "scripts\install.ps1") `
        -Version $tag `
        -InstallDir $installDir `
        -NoModifyPath
    if (-not (Test-Path -LiteralPath (Join-Path $installDir "shore.exe") -PathType Leaf)) {
        throw "installer did not create shore.exe"
    }

    $missingReleaseFailed = $false
    try {
        & (Join-Path $repoRoot "scripts\install.ps1") `
            -Version "v9.8.6-missing" `
            -InstallDir (Join-Path $tempDir "missing-bin") `
            -NoModifyPath
    }
    catch {
        if ($_.Exception.Message -notmatch "Check https://github.com/withpointbreak/pointbreak/releases") {
            throw
        }
        $missingReleaseFailed = $true
    }
    if (-not $missingReleaseFailed) {
        throw "installer accepted a missing release"
    }

    Remove-Item -LiteralPath (Join-Path $installDir "shore.exe")
    Set-Content -LiteralPath (Join-Path $releaseDir "checksums.txt") `
        -Value "$('0' * 64)  $archive" `
        -Encoding ascii
    $checksumFailed = $false
    try {
        & (Join-Path $repoRoot "scripts\install.ps1") `
            -Version $tag `
            -InstallDir $installDir `
            -NoModifyPath
    }
    catch {
        if ($_.Exception.Message -notmatch "Checksum mismatch") {
            throw
        }
        $checksumFailed = $true
    }
    if (-not $checksumFailed) {
        throw "installer accepted an invalid checksum"
    }
    if (Test-Path -LiteralPath (Join-Path $installDir "shore.exe")) {
        throw "installer wrote shore.exe after checksum failure"
    }

    & (Join-Path $repoRoot "scripts\install.ps1") `
        -Version $tag `
        -InstallDir $installDir `
        -NoVerify `
        -NoModifyPath
    if (-not (Test-Path -LiteralPath (Join-Path $installDir "shore.exe") -PathType Leaf)) {
        throw "-NoVerify install did not create shore.exe"
    }

    Write-Host "install.ps1 self-test ok"
}
finally {
    $env:POINTBREAK_INSTALLER_FIXTURE_ROOT = $previousFixtureRoot
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
