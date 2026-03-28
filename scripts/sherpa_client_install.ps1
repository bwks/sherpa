<#
.SYNOPSIS
    Install or update the Sherpa CLI client on Windows.

.DESCRIPTION
    Downloads and installs the Sherpa CLI client binary from GitHub Releases.
    Installs to $HOME\.sherpa\bin\ and adds it to the user PATH.

    No administrator privileges required.

.PARAMETER Version
    Specific release version to install (e.g., v0.3.35).
    Defaults to the latest release.

.PARAMETER Update
    Update mode: skip first-time setup prompts and replace the existing binary.

.EXAMPLE
    .\sherpa_client_install.ps1

.EXAMPLE
    .\sherpa_client_install.ps1 -Version v0.3.35

.EXAMPLE
    .\sherpa_client_install.ps1 -Update
#>

[CmdletBinding()]
param(
    [string]$Version,
    [switch]$Update
)

$ErrorActionPreference = 'Stop'

# ============================================================================
# Constants
# ============================================================================

$REPO = "bwks/sherpa"
$BINARY_NAME = "sherpa.exe"
$TARGET = "x86_64-pc-windows-msvc"
$ARCHIVE_NAME = "sherpa-${TARGET}.zip"
$INSTALL_DIR = Join-Path (Join-Path $HOME ".sherpa") "bin"
$INSTALL_PATH = Join-Path $INSTALL_DIR $BINARY_NAME
$GITHUB_API_URL = "https://api.github.com/repos/${REPO}/releases/latest"

# ============================================================================
# Helper functions
# ============================================================================

function Write-Status {
    param([string]$Message)
    Write-Host "[*] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[+] $Message" -ForegroundColor Green
}

function Write-Failure {
    param([string]$Message)
    Write-Host "[-] $Message" -ForegroundColor Red
}

function Get-LatestVersion {
    Write-Status "Fetching latest release from GitHub..."
    try {
        $response = Invoke-RestMethod -Uri $GITHUB_API_URL -UseBasicParsing
        $tag = $response.tag_name
        if (-not $tag) {
            Write-Failure "Could not determine latest version from GitHub API"
            exit 1
        }
        return $tag
    }
    catch {
        Write-Failure "Failed to query GitHub API: $($_.Exception.Message)"
        Write-Failure "URL: $GITHUB_API_URL"
        exit 1
    }
}

function Get-InstalledVersion {
    if (Test-Path $INSTALL_PATH) {
        try {
            $output = & $INSTALL_PATH --version 2>&1
            # Parse version from output like "sherpa 0.3.35" -> "v0.3.35"
            $parts = ($output -split '\s+')
            if ($parts.Count -ge 2) {
                return "v$($parts[-1])"
            }
            return $null
        }
        catch {
            return $null
        }
    }
    return $null
}

function Add-ToUserPath {
    param([string]$Directory)

    $currentPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $currentPath) {
        $currentPath = ""
    }

    # Check if already in PATH (case-insensitive)
    $paths = $currentPath -split ';' | Where-Object { $_ -ne '' }
    $normalized = $paths | ForEach-Object { $_.TrimEnd('\') }
    $normalizedDir = $Directory.TrimEnd('\')

    if ($normalized -contains $normalizedDir) {
        Write-Status "PATH already contains $Directory"
        return
    }

    # Append to user PATH
    $newPath = if ($currentPath -and $currentPath[-1] -ne ';') {
        "${currentPath};${Directory}"
    }
    else {
        "${currentPath}${Directory}"
    }

    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')

    # Update current session PATH
    if ($env:PATH -notlike "*$Directory*") {
        $env:PATH = "$env:PATH;$Directory"
    }

    Write-Success "Added $Directory to user PATH"
    Write-Status "Restart your terminal for PATH changes to take effect in new sessions"
}

# ============================================================================
# Pre-flight checks
# ============================================================================

# Check PowerShell version
if ($PSVersionTable.PSVersion.Major -lt 5) {
    Write-Failure "PowerShell 5.1 or later is required (found $($PSVersionTable.PSVersion))"
    exit 1
}

# ============================================================================
# Resolve version
# ============================================================================

$targetVersion = if ($Version) {
    # Ensure version starts with 'v'
    if ($Version -notmatch '^v') { "v$Version" } else { $Version }
}
else {
    Get-LatestVersion
}

Write-Status "Target version: $targetVersion"

# ============================================================================
# Check existing installation
# ============================================================================

$installedVersion = Get-InstalledVersion

if ($Update) {
    if (-not (Test-Path $INSTALL_PATH)) {
        Write-Failure "No existing installation found at $INSTALL_PATH"
        Write-Failure "Run without -Update to perform a fresh install"
        exit 1
    }

    if ($installedVersion -eq $targetVersion) {
        Write-Success "Already on version $targetVersion - nothing to do"
        exit 0
    }

    Write-Status "Updating from $installedVersion to $targetVersion"
}
elseif ($installedVersion) {
    Write-Status "Existing installation found: $installedVersion"
    $confirm = Read-Host "Replace with $targetVersion? [Y/n]"
    if ($confirm -and $confirm -notin @('Y', 'y', 'yes', 'Yes', '')) {
        Write-Status "Aborted"
        exit 0
    }
}

# ============================================================================
# Download
# ============================================================================

$downloadUrl = "https://github.com/${REPO}/releases/download/${targetVersion}/${ARCHIVE_NAME}"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "sherpa-install-$(Get-Random)"
$zipPath = Join-Path $tempDir $ARCHIVE_NAME

Write-Status "Downloading $downloadUrl"

try {
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
}
catch {
    Write-Failure "Download failed: $($_.Exception.Message)"
    Write-Failure "URL: $downloadUrl"
    if (Test-Path $tempDir) { Remove-Item -Recurse -Force $tempDir }
    exit 1
}

# ============================================================================
# Extract and install
# ============================================================================

Write-Status "Extracting archive..."

try {
    $extractDir = Join-Path $tempDir "extracted"
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

    # Create install directory if it doesn't exist
    if (-not (Test-Path $INSTALL_DIR)) {
        New-Item -ItemType Directory -Path $INSTALL_DIR -Force | Out-Null
        Write-Status "Created $INSTALL_DIR"
    }

    # Find the binary in extracted files
    $extractedBinary = Get-ChildItem -Path $extractDir -Filter $BINARY_NAME -Recurse | Select-Object -First 1
    if (-not $extractedBinary) {
        Write-Failure "Could not find $BINARY_NAME in downloaded archive"
        exit 1
    }

    # Copy to install location (overwrite if exists)
    Copy-Item -Path $extractedBinary.FullName -Destination $INSTALL_PATH -Force
    Write-Success "Installed $BINARY_NAME to $INSTALL_PATH"
}
catch {
    Write-Failure "Extraction failed: $($_.Exception.Message)"
    exit 1
}
finally {
    # Clean up temp files
    if (Test-Path $tempDir) {
        Remove-Item -Recurse -Force $tempDir
    }
}

# ============================================================================
# PATH setup (skip in update mode, already done)
# ============================================================================

if (-not $Update) {
    Add-ToUserPath $INSTALL_DIR
}

# ============================================================================
# Verify
# ============================================================================

try {
    $versionOutput = & $INSTALL_PATH --version 2>&1
    Write-Success "Verified: $versionOutput"
}
catch {
    Write-Failure "Verification failed: could not run $INSTALL_PATH"
    exit 1
}

Write-Success "Sherpa client installed successfully"
