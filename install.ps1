# CogZ install script for Windows — downloads the latest release binary from GitHub.
#
# Usage:
#   irm https://raw.githubusercontent.com/balaianu/CogZ/main/install.ps1 | iex
#
# Or:
#   .\install.ps1
#
# Installs to $HOME\.local\bin\cogz.exe
# Creates $HOME\AppData\Local\cogz\models\ for the model cache.

$ErrorActionPreference = "Stop"

$Repo = "balaianu/CogZ"
$ApiUrl = "https://api.github.com/repos/$Repo/releases/latest"

# Detect architecture.
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -eq "AMD64") {
    $Asset = "cogz-x86_64-pc-windows-msvc.exe"
} else {
    Write-Error "Unsupported architecture: $Arch. CogZ on Windows requires x86_64."
    exit 1
}

# Determine install directory.
$InstallDir = "$HOME\.local\bin"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

Write-Host "Installing CogZ for Windows x86_64..."

# Fetch the latest release download URL.
Write-Host "Fetching latest release..."
$Release = Invoke-RestMethod -Uri $ApiUrl -Headers @{ "User-Agent" = "cogz-install" }
$AssetObj = $Release.assets | Where-Object { $_.name -eq $Asset } | Select-Object -First 1

if (-not $AssetObj) {
    Write-Error "Could not find $Asset in the latest release."
    Write-Host "Check https://github.com/$Repo/releases for available assets."
    exit 1
}

$DownloadUrl = $AssetObj.browser_download_url

# Download to a temporary file.
$TempFile = [System.IO.Path]::GetTempFileName()
Write-Host "Downloading $Asset..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempFile

# Verify checksum from SHA256SUMS.
$ChecksumAsset = $Release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
if ($ChecksumAsset) {
    $TempSums = [System.IO.Path]::GetTempFileName()
    Invoke-WebRequest -Uri $ChecksumAsset.browser_download_url -OutFile $TempSums
    $SumsContent = Get-Content $TempSums -Raw
    $ExpectedLine = ($SumsContent -split "`n" | Where-Object { $_ -match "  $Asset$" }) -replace "^\s+", ""
    if (-not $ExpectedLine) {
        Write-Error "SHA256SUMS downloaded but no entry for $Asset. Refusing to install unverified binary."
        Remove-Item $TempFile, $TempSums -Force
        exit 1
    }
    $ExpectedHash = ($ExpectedLine -split "\s+")[0]
    $ActualHash = (Get-FileHash $TempFile -Algorithm SHA256).Hash.ToLower()
    if ($ExpectedHash.ToLower() -ne $ActualHash) {
        Write-Error "Checksum mismatch. Expected: $ExpectedHash, Actual: $ActualHash"
        Remove-Item $TempFile, $TempSums -Force
        exit 1
    }
    Write-Host "Checksum verified."
    Remove-Item $TempSums -Force
} else {
    Write-Host "WARNING: SHA256SUMS not found in release. Installing without checksum verification."
}

# Install.
$DestPath = Join-Path $InstallDir "cogz.exe"
Move-Item -Path $TempFile -Destination $DestPath -Force
Write-Host "Installed to $DestPath"

# Create model cache directory.
$ModelDir = Join-Path $env:LOCALAPPDATA "cogz\models"
if (-not (Test-Path $ModelDir)) {
    New-Item -ItemType Directory -Path $ModelDir -Force | Out-Null
}
Write-Host "Model cache: $ModelDir"

# Check PATH.
$PathDirs = $env:PATH -split ";"
if ($PathDirs -notcontains $InstallDir) {
    Write-Host ""
    Write-Host "WARNING: $InstallDir is not on your PATH."
    Write-Host "Add this line to your PowerShell profile:"
    Write-Host "  `$env:PATH += `";$InstallDir`""
}

# Verify.
Write-Host ""
Write-Host "Verification:"
& $DestPath --version

Write-Host ""
Write-Host "Next steps:"
Write-Host "  cd your-project"
Write-Host "  cogz init"
Write-Host "  cogz index"
Write-Host ""
Write-Host "To enable agent hooks (Devin/Claude Code):"
Write-Host "  See docs/integration/hooks.md"
Write-Host ""
Write-Host "Done."
