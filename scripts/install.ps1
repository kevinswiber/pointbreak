<#
.SYNOPSIS
Installs Pointbreak Review on Windows.

.DESCRIPTION
Downloads, verifies, and atomically installs a Pointbreak Review release.

.PARAMETER Version
The release tag or version to install. The default is the latest release.

.PARAMETER InstallDir
The directory where pointbreak.exe is installed.

.PARAMETER NoModifyPath
Do not add the installation directory to the user PATH.
#>
[CmdletBinding()]
param(
    [string]$Version = "latest",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Pointbreak\bin"),
    [switch]$NoModifyPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression.FileSystem

$Repository = "withpointbreak/pointbreak"
$ApiRoot = "https://api.github.com/repos"
$DownloadRoot = "https://github.com/$Repository/releases/download"
$ReleasesUrl = "https://github.com/$Repository/releases"

function Resolve-ReleaseTag {
    if ($Version -eq "latest") {
        Write-Host "Finding the latest Pointbreak Review release..."
        $release = Invoke-RestMethod -Uri "$ApiRoot/$Repository/releases/latest" -Method Get
        $tag = [string]$release.tag_name
    }
    elseif ($Version.StartsWith("v")) {
        $tag = $Version
    }
    else {
        $tag = "v$Version"
    }

    if ($tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$') {
        throw "Unsupported release version: $tag"
    }
    return $tag
}

function Get-PointbreakTarget {
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    if ($env:POINTBREAK_INSTALLER_FIXTURE_ROOT) {
        switch ($architecture) {
            "X64" { return "win32-x64" }
            "Arm64" { return "win32-arm64" }
            default { throw "Unsupported Windows architecture: $architecture" }
        }
    }

    if ($env:OS -ne "Windows_NT") {
        throw "This installer supports Windows; use install.sh on macOS or Linux."
    }

    switch ($architecture) {
        "X64" { return "win32-x64" }
        "Arm64" { return "win32-arm64" }
        default { throw "Unsupported Windows architecture: $architecture" }
    }
}

function Copy-ReleaseAsset {
    param(
        [Parameter(Mandatory = $true)][string]$Tag,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    # Used by the repository's hermetic installer self-tests.
    if ($env:POINTBREAK_INSTALLER_FIXTURE_ROOT) {
        $fixture = Join-Path (Join-Path $env:POINTBREAK_INSTALLER_FIXTURE_ROOT $Tag) $Name
        Copy-Item -LiteralPath $fixture -Destination $Destination
    }
    else {
        Invoke-WebRequest -Uri "$DownloadRoot/$Tag/$Name" -OutFile $Destination
    }
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

function Get-ExpectedChecksum {
    param(
        [Parameter(Mandatory = $true)][string]$ChecksumsPath,
        [Parameter(Mandatory = $true)][string]$ArchiveName
    )

    $pattern = '^(?<hash>[0-9A-Fa-f]{64})\s+\*?' + [Regex]::Escape($ArchiveName) + '$'
    $checksumMatches = @(
        Get-Content -LiteralPath $ChecksumsPath | ForEach-Object {
            if ($_ -match $pattern) {
                $Matches.hash.ToLowerInvariant()
            }
        }
    )

    if ($checksumMatches.Count -ne 1) {
        throw "checksums.txt must contain exactly one valid SHA-256 entry for $ArchiveName"
    }
    return $checksumMatches[0]
}

function Assert-ArchiveLayout {
    param(
        [Parameter(Mandatory = $true)][string]$ArchivePath,
        [Parameter(Mandatory = $true)][string]$ArchiveName
    )

    $expectedEntries = @("LICENSE", "NOTICE", "pointbreak.exe")
    $zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $actualEntries = @($zip.Entries | ForEach-Object { $_.FullName } | Sort-Object)
        $difference = @(Compare-Object -ReferenceObject $expectedEntries -DifferenceObject $actualEntries)
        if ($difference.Count -ne 0) {
            throw "$ArchiveName has an invalid archive layout"
        }
        foreach ($entry in $zip.Entries) {
            $unixFileType = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ([string]::IsNullOrEmpty($entry.Name) -or $unixFileType -eq 0xA000) {
                throw "$ArchiveName has an invalid archive layout"
            }
        }
    }
    finally {
        $zip.Dispose()
    }
}

function Test-RegularFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    $attributes = (Get-Item -LiteralPath $Path -Force).Attributes
    return ($attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0
}

function Read-PointbreakVersionDocument {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion
    )

    if ($env:POINTBREAK_INSTALLER_FIXTURE_ROOT -and $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER) {
        $output = @(& $env:POINTBREAK_INSTALLER_FIXTURE_RUNNER $Path version --format json 2>&1)
    }
    else {
        $output = @(& $Path version --format json 2>&1)
    }
    if (-not $?) {
        throw "Pointbreak version command failed"
    }
    if ($output.Count -ne 1) {
        throw "Pointbreak version command did not emit exactly one document"
    }

    try {
        $document = $output[0].ToString() | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw "Pointbreak version command did not emit JSON"
    }

    $expectedProperties = @("cliVersion", "diagnostics", "documents", "schema", "version")
    $actualProperties = @($document.PSObject.Properties.Name | Sort-Object)
    if (@(Compare-Object -ReferenceObject $expectedProperties -DifferenceObject $actualProperties).Count -ne 0) {
        throw "Pointbreak version document has unexpected fields"
    }
    if ($document.schema -cne "pointbreak.version" -or
        $document.version -ne 1 -or
        $document.cliVersion -cne $ExpectedVersion -or
        @($document.diagnostics).Count -ne 0) {
        throw "Pointbreak version document has an unexpected identity"
    }

    $versionMember = $document.documents.PSObject.Properties["pointbreak.version"]
    if ($null -eq $versionMember -or $versionMember.Value -ne 1) {
        throw "Pointbreak version document has an unexpected registry"
    }
    return $document
}

function Get-NormalizedPathEntry {
    param([Parameter(Mandatory = $true)][string]$Entry)

    return [Environment]::ExpandEnvironmentVariables($Entry).Trim().Trim('"').TrimEnd("\")
}

function Add-InstallDirToUserPath {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $currentUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = if ([string]::IsNullOrWhiteSpace($currentUserPath)) {
        @()
    }
    else {
        @($currentUserPath.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries))
    }

    $normalizedDirectory = Get-NormalizedPathEntry -Entry $Directory
    $alreadyPresent = $entries | Where-Object {
        (Get-NormalizedPathEntry -Entry $_) -eq $normalizedDirectory
    }
    if (-not $alreadyPresent) {
        $newUserPath = if ([string]::IsNullOrWhiteSpace($currentUserPath)) {
            $Directory
        }
        else {
            "$currentUserPath;$Directory"
        }
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
        Write-Host "Added $Directory to your user PATH."
    }

    $processHasEntry = @($env:Path.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries)) |
        Where-Object { (Get-NormalizedPathEntry -Entry $_) -eq $normalizedDirectory }
    if (-not $processHasEntry) {
        $env:Path = "$Directory;$env:Path"
    }
}

function Install-Pointbreak {
    $releaseTag = Resolve-ReleaseTag
    $releaseVersion = $releaseTag.Substring(1)
    $target = Get-PointbreakTarget
    $archiveName = "pointbreak-$releaseVersion-$target.zip"
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("pointbreak-install-" + [Guid]::NewGuid())
    $archivePath = Join-Path $tempDir $archiveName
    $extractDir = Join-Path $tempDir "extract"
    $destination = Join-Path $InstallDir "pointbreak.exe"
    $stagedBinary = Join-Path $InstallDir (".pointbreak-install-" + [Guid]::NewGuid() + ".exe")
    $backupBinary = Join-Path $InstallDir (".pointbreak-backup-" + [Guid]::NewGuid() + ".exe")
    $replacementDone = $false
    $hadPrevious = $false

    New-Item -ItemType Directory -Path $tempDir | Out-Null
    try {
        Write-Host "Downloading Pointbreak Review $releaseTag for $target..."
        try {
            Copy-ReleaseAsset -Tag $releaseTag -Name $archiveName -Destination $archivePath
        }
        catch {
            throw "Could not download $archiveName for $releaseTag. Check $ReleasesUrl. $($_.Exception.Message)"
        }

        $checksumsPath = Join-Path $tempDir "checksums.txt"
        try {
            Copy-ReleaseAsset -Tag $releaseTag -Name "checksums.txt" -Destination $checksumsPath
        }
        catch {
            throw "Could not download checksums.txt. $($_.Exception.Message)"
        }

        $expectedChecksum = Get-ExpectedChecksum `
            -ChecksumsPath $checksumsPath `
            -ArchiveName $archiveName
        $actualChecksum = Get-FileSha256 -Path $archivePath
        if ($actualChecksum -ne $expectedChecksum) {
            throw "Checksum mismatch for $archiveName"
        }
        Write-Host "Verified SHA-256 checksum."

        Assert-ArchiveLayout -ArchivePath $archivePath -ArchiveName $archiveName
        Write-Host "Verified release archive layout."
        Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir
        foreach ($entry in @("pointbreak.exe", "LICENSE", "NOTICE")) {
            $entryPath = Join-Path $extractDir $entry
            if (-not (Test-RegularFile -Path $entryPath)) {
                throw "$archiveName contains a non-regular $entry"
            }
        }
        $binaryPath = Join-Path $extractDir "pointbreak.exe"

        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        [IO.File]::Copy($binaryPath, $stagedBinary)
        try {
            Read-PointbreakVersionDocument -Path $stagedBinary -ExpectedVersion $releaseVersion | Out-Null
        }
        catch {
            throw "Staged Pointbreak version document did not match $releaseVersion. $($_.Exception.Message)"
        }

        if (Test-Path -LiteralPath $destination) {
            if (-not (Test-RegularFile -Path $destination)) {
                throw "Pointbreak destination is not a regular file: $destination"
            }
            $hadPrevious = $true
        }

        if ($env:POINTBREAK_INSTALLER_FIXTURE_ROOT -and
            $env:POINTBREAK_INSTALLER_TEST_FAIL_REPLACE -eq "1") {
            throw "Could not replace $destination"
        }
        try {
            if ($hadPrevious) {
                [IO.File]::Replace($stagedBinary, $destination, $backupBinary, $true)
            }
            else {
                [IO.File]::Move($stagedBinary, $destination)
            }
        }
        catch {
            throw "Could not replace $destination. $($_.Exception.Message)"
        }
        $replacementDone = $true

        try {
            Read-PointbreakVersionDocument -Path $destination -ExpectedVersion $releaseVersion | Out-Null
        }
        catch {
            throw "Installed Pointbreak version document did not match $releaseVersion. $($_.Exception.Message)"
        }

        if ($hadPrevious) {
            Remove-Item -LiteralPath $backupBinary -Force
        }
        $replacementDone = $false

        $pathConfigured = $false
        if (-not $NoModifyPath) {
            try {
                Add-InstallDirToUserPath -Directory $InstallDir
                $pathConfigured = $true
            }
            catch {
                Write-Warning "Could not update your user PATH: $($_.Exception.Message)"
            }
        }

        Write-Host "Installed Pointbreak Review $releaseVersion to $destination"
        if (-not $pathConfigured) {
            Write-Host "Add $InstallDir to PATH, then run: pointbreak --help"
        }
        else {
            Write-Host "Run: pointbreak --help"
        }
    }
    catch {
        $installError = $_
        if ($replacementDone) {
            try {
                if ($hadPrevious) {
                    [IO.File]::Replace($backupBinary, $destination, $stagedBinary, $true)
                }
                else {
                    Remove-Item -LiteralPath $destination -Force
                }
            }
            catch {
                throw "Installation failed and the previous Pointbreak could not be restored. $($_.Exception.Message)"
            }
        }
        throw $installError
    }
    finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stagedBinary -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $backupBinary -Force -ErrorAction SilentlyContinue
    }
}

Install-Pointbreak
