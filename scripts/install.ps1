# Pylon installer for Windows — served at https://www.pylonsync.com/install.ps1
#
#   powershell -c "irm https://www.pylonsync.com/install.ps1 | iex"
#
# Options (environment variables):
#   PYLON_VERSION=v0.7.0   install a specific release (default: latest)
#   PYLON_INSTALL=C:\tools install prefix; the binary lands at
#                          $PYLON_INSTALL\bin\pylon.exe
#                          (default: $env:USERPROFILE\.pylon)
#
# Canonical source: https://github.com/pylonsync/pylon/blob/main/scripts/install.ps1
# The copy served by www.pylonsync.com lives in the control-plane app's
# public/ dir — keep the two in sync, and in sync with install.sh.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo = 'pylonsync/pylon'
$InstallPrefix = if ($env:PYLON_INSTALL) { $env:PYLON_INSTALL } else { Join-Path $env:USERPROFILE '.pylon' }
$BinDir = Join-Path $InstallPrefix 'bin'

function Fail($message) {
    Write-Error "install.ps1: $message"
    exit 1
}

# --- Detect platform ---------------------------------------------------------
# PROCESSOR_ARCHITECTURE reports the calling process's architecture, so a
# 32-bit PowerShell on a 64-bit machine says x86. PROCESSOR_ARCHITEW6432 is
# set only in that case and names the real hardware.
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' {
        # Windows on ARM emulates x64, in both directions unlike Rosetta, so
        # the x64 build runs here. It is emulated, so say so.
        $target = 'x86_64-pc-windows-msvc'
        Write-Host 'Note: no native ARM64 build yet — installing the x64 binary, which Windows runs under emulation.'
    }
    default { Fail "unsupported architecture: $arch (Pylon ships an x64 Windows binary)" }
}

# --- Resolve version ---------------------------------------------------------
if ($env:PYLON_VERSION) {
    $version = $env:PYLON_VERSION
    if (-not $version.StartsWith('v')) { $version = "v$version" }
} else {
    # /releases/latest redirects to /releases/tag/<tag>; read the tag off the
    # Location header. No GitHub API call, so no rate-limit trouble in CI.
    # HttpWebRequest rather than Invoke-WebRequest because the property holding
    # the final URI differs between Windows PowerShell 5.1 and PowerShell 7.
    $request = [System.Net.WebRequest]::Create("https://github.com/$Repo/releases/latest")
    $request.AllowAutoRedirect = $false
    $request.Method = 'HEAD'
    try {
        $response = $request.GetResponse()
        $location = $response.Headers['Location']
        $response.Close()
    } catch {
        Fail "could not resolve the latest release from github.com/$Repo"
    }
    if (-not $location) { Fail "could not resolve the latest release from github.com/$Repo" }
    $version = $location.Split('/')[-1]
}
if (-not $version) { Fail 'could not parse the latest release tag' }

$asset = "pylon-$version-$target.zip"
$url = "https://github.com/$Repo/releases/download/$version/$asset"

Write-Host "Installing pylon $version ($target) to $BinDir"

# --- Download + verify -------------------------------------------------------
$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("pylon-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
try {
    $archive = Join-Path $tmpDir $asset
    try {
        # Invoke-WebRequest's progress bar makes a piped install crawl in
        # Windows PowerShell; suppressing it is worth the extra two lines.
        $previousProgress = $ProgressPreference
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing
    } catch {
        Fail "download failed: $url`n  If this is a brand-new release, the binary may still be building — retry in a few minutes."
    } finally {
        $ProgressPreference = $previousProgress
    }

    $checksumFile = "$archive.sha256"
    $haveChecksum = $true
    try {
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $checksumFile -UseBasicParsing
    } catch {
        $haveChecksum = $false
    } finally {
        $ProgressPreference = $previousProgress
    }

    if ($haveChecksum) {
        # The published file is `<hash>  <name>`, matching sha256sum.
        $expected = ((Get-Content -Raw $checksumFile).Trim() -split '\s+')[0].ToLower()
        $actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
        if ($expected -ne $actual) {
            Fail "checksum mismatch for $asset (expected $expected, got $actual)"
        }
    } else {
        Write-Host "warning: no checksum published for $asset; skipping verification"
    }

    # --- Install -------------------------------------------------------------
    $extracted = Join-Path $tmpDir 'unpacked'
    Expand-Archive -Path $archive -DestinationPath $extracted -Force
    $binary = Join-Path $extracted 'pylon.exe'
    if (-not (Test-Path $binary)) { Fail "archive did not contain a 'pylon.exe' binary" }

    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
    # A running pylon.exe holds a lock on its own image, so replacing it while
    # `pylon dev` is up fails with a sharing violation rather than a clear
    # message. Say which process to stop.
    try {
        Move-Item -Path $binary -Destination (Join-Path $BinDir 'pylon.exe') -Force
    } catch {
        Fail "could not replace $BinDir\pylon.exe — stop any running pylon process and re-run."
    }
} finally {
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}

Write-Host "OK  pylon $version installed at $BinDir\pylon.exe"

# --- PATH --------------------------------------------------------------------
# Read and write the raw registry value rather than going through
# [Environment]::GetEnvironmentVariable(..., 'User'). That API expands
# %VARIABLES% on read, so writing the result back replaces every reference in
# the user's PATH with whatever it happened to expand to — a well-known way to
# corrupt someone's environment.
$envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
try {
    $userPath = $envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $entries = $userPath -split ';' | Where-Object { $_ -ne '' }
    if ($entries -notcontains $BinDir) {
        $newPath = (@($entries) + $BinDir) -join ';'
        $envKey.SetValue('Path', $newPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        Write-Host ''
        Write-Host "Added $BinDir to your user PATH. Open a new terminal for it to take effect."
    }
} finally {
    $envKey.Close()
}

# Make pylon usable in THIS session too, so the next line of a piped install
# works without opening a new terminal.
if (($env:Path -split ';') -notcontains $BinDir) {
    $env:Path = "$env:Path;$BinDir"
}

Write-Host ''
Write-Host 'Get started:'
Write-Host '  npm create @pylonsync/pylon@latest my-app'
Write-Host '  cd my-app; pylon dev'
