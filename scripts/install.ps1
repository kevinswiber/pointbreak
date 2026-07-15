# Install the latest (or a requested) Pointbreak Review release on Windows.
#
# Usage:
#   irm https://raw.githubusercontent.com/withpointbreak/pointbreak/main/scripts/install.ps1 | iex

[CmdletBinding()]
param(
    [string]$Version = "latest",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Pointbreak\bin"),
    [switch]$NoVerify,
    [switch]$NoModifyPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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
    if ($env:OS -ne "Windows_NT") {
        throw "This installer supports Windows; use install.sh on macOS or Linux."
    }

    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
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

    New-Item -ItemType Directory -Path $tempDir | Out-Null
    try {
        Write-Host "Downloading Pointbreak Review $releaseTag for $target..."
        try {
            Copy-ReleaseAsset -Tag $releaseTag -Name $archiveName -Destination $archivePath
        }
        catch {
            throw "Could not download $archiveName for $releaseTag. Check $ReleasesUrl. $($_.Exception.Message)"
        }

        if (-not $NoVerify) {
            $checksumsPath = Join-Path $tempDir "checksums.txt"
            try {
                Copy-ReleaseAsset -Tag $releaseTag -Name "checksums.txt" -Destination $checksumsPath
            }
            catch {
                throw "Could not download checksums.txt. Use -NoVerify only if you accept the risk. $($_.Exception.Message)"
            }

            $expectedChecksum = Get-ExpectedChecksum `
                -ChecksumsPath $checksumsPath `
                -ArchiveName $archiveName
            $actualChecksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualChecksum -ne $expectedChecksum) {
                throw "Checksum mismatch for $archiveName"
            }
            Write-Host "Verified SHA-256 checksum."
        }
        else {
            Write-Warning "Skipping checksum verification because -NoVerify was supplied."
        }

        Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir
        $binaryPath = Join-Path $extractDir "shore.exe"
        if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
            throw "$archiveName does not contain shore.exe"
        }

        $versionOutput = & $binaryPath --version 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Downloaded shore.exe failed its version check"
        }

        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        $destination = Join-Path $InstallDir "shore.exe"
        Copy-Item -LiteralPath $binaryPath -Destination $destination -Force

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

        $versionLine = ($versionOutput | Select-Object -First 1).ToString()
        Write-Host "Installed $versionLine to $destination"
        if (-not $pathConfigured) {
            Write-Host "Add $InstallDir to PATH, then run: shore --help"
        }
        else {
            Write-Host "Run: shore --help"
        }
    }
    finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Pointbreak
