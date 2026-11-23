# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# Install the github-backup binary from GitHub Releases (Windows).
#
#   irm https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.ps1 | iex
#
# or download it, read it, then run it:
#
#   .\install.ps1 [-Version X.Y.Z] [-Dir C:\path\to\dir]
#
# Downloads github-backup-windows-x86_64.exe, checks it against the release's
# SHA256SUMS.txt and installs it as <Dir>\github-backup.exe (default
# %LOCALAPPDATA%\Programs\github-backup).  No administrator rights needed.
# With `| iex` set $env:GITHUB_BACKUP_VERSION / $env:GITHUB_BACKUP_INSTALL_DIR
# instead of the parameters.

[CmdletBinding()]
param(
    [string]$Version = $(if ($env:GITHUB_BACKUP_VERSION) { $env:GITHUB_BACKUP_VERSION } else { 'latest' }),
    [string]$Dir = $(if ($env:GITHUB_BACKUP_INSTALL_DIR) { $env:GITHUB_BACKUP_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\github-backup' })
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is far faster without it
# Windows PowerShell 5.1 may not offer TLS 1.2 by default; GitHub requires it.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'tomtom215/github-backup-rust'
$asset = 'github-backup-windows-x86_64.exe'

$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($osArch -ne 'X64' -and $osArch -ne 'Arm64') {
    throw "No release binary for $osArch Windows; build from source instead."
}
# Windows on Arm runs the x86_64 binary under emulation.

$Version = $Version.TrimStart('v')
if ($Version -eq 'latest') {
    $base = "https://github.com/$repo/releases/latest/download"
} else {
    $base = "https://github.com/$repo/releases/download/v$Version"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $exe = Join-Path $tmp $asset
    $sums = Join-Path $tmp 'SHA256SUMS.txt'
    Write-Output "Downloading $asset ($Version) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $exe
    Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS.txt" -OutFile $sums

    $expected = $null
    foreach ($line in Get-Content -LiteralPath $sums) {
        $fields = $line -split '\s+'
        if ($fields.Count -ge 2 -and $fields[1].TrimStart('*') -eq $asset) {
            $expected = $fields[0].ToLowerInvariant()
            break
        }
    }
    if (-not $expected) { throw "$asset is not listed in SHA256SUMS.txt" }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "Checksum mismatch for $asset (expected $expected, got $actual)"
    }
    Write-Output 'Checksum OK.'

    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    $dest = Join-Path $Dir 'github-backup.exe'
    Move-Item -Force -LiteralPath $exe -Destination $dest
    Write-Output "Installed $dest"
    & $dest --version
    if ($LASTEXITCODE -ne 0) { throw "$dest --version failed" }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath -or ($userPath -split ';') -notcontains $Dir) {
        Write-Output "Note: $Dir is not on your PATH.  To add it for your user, run:"
        Write-Output "  [Environment]::SetEnvironmentVariable('Path', `"`$([Environment]::GetEnvironmentVariable('Path','User'));$Dir`", 'User')"
        Write-Output 'and open a new terminal.'
    }
} finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
