<#
.SYNOPSIS
    One-shot setup for annnekkk_random_server on Windows.

.DESCRIPTION
    Checks for a reachable MongoDB and installs one via Chocolatey if there is
    none, downloads the latest server release, seeds the database, and registers
    the server to launch at startup.

    Every step is skipped when it is already done, so re-running is safe.

.EXAMPLE
    irm https://raw.githubusercontent.com/AnCry1596/Annnekkk-DataRandomize/main/setup.ps1 | iex

.EXAMPLE
    .\setup.ps1 -InstallDir D:\tools\randomserver -Port 9000

.EXAMPLE
    # Use an existing MongoDB instead of installing one.
    .\setup.ps1 -MongoUri 'mongodb://user:pass@10.0.0.5:27017/'
#>
[CmdletBinding()]
param(
    # Where the server binaries and .env are installed.
    [string]$InstallDir = "$env:LOCALAPPDATA\AnnnekkkRandomServer",

    # MongoDB to use. If this is the default localhost and nothing is listening,
    # MongoDB is installed locally.
    [string]$MongoUri = 'mongodb://localhost:27017/',

    [string]$MongoDb = 'random_server',

    [string]$ListenHost = '127.0.0.1',
    [int]$Port = 8080,

    # GitHub repo to pull the release from, as "owner/name".
    [string]$Repo = 'AnCry1596/Annnekkk-DataRandomize',

    # Re-seed collections that already hold documents.
    [switch]$Force,

    # Install the binaries but do not register the startup task.
    [switch]$NoAutoStart
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# TLS 1.2 for Windows PowerShell 5.1, whose default would fail against GitHub.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$TaskName = 'AnnnekkkRandomServer'

function Write-Step { param([string]$m) Write-Host "==> $m" -ForegroundColor Cyan }
function Write-Ok   { param([string]$m) Write-Host "    $m" -ForegroundColor Green }
function Write-Note { param([string]$m) Write-Host "    $m" -ForegroundColor DarkGray }

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole(
        [Security.Principal.WindowsBuiltinRole]::Administrator)
}

# Parse host:port out of a mongodb:// URI so we can test the socket directly —
# far faster than waiting out a driver's server-selection timeout.
function Get-MongoEndpoint {
    param([string]$Uri)
    $hostPort = $Uri -replace '^mongodb(\+srv)?://', '' -replace '/.*$', ''
    if ($hostPort -match '@') { $hostPort = ($hostPort -split '@')[-1] }
    # Replica-set URIs list several hosts; the first is enough for a liveness check.
    $first = ($hostPort -split ',')[0]
    $parts = $first -split ':'
    [pscustomobject]@{
        Host = if ($parts[0]) { $parts[0] } else { 'localhost' }
        Port = if ($parts.Count -gt 1 -and $parts[1]) { [int]$parts[1] } else { 27017 }
    }
}

function Test-Mongo {
    param([string]$Uri, [int]$TimeoutMs = 2000)
    $ep = Get-MongoEndpoint $Uri
    try {
        $client = [Net.Sockets.TcpClient]::new()
        $async = $client.BeginConnect($ep.Host, $ep.Port, $null, $null)
        if ($async.AsyncWaitHandle.WaitOne($TimeoutMs)) {
            $client.EndConnect($async)   # throws if the connect actually failed
            return $true
        }
        return $false
    } catch {
        return $false
    } finally {
        if ($client) { $client.Dispose() }
    }
}

# ── 1. MongoDB ────────────────────────────────────────────────────────────────

Write-Step 'Checking MongoDB'
$ep = Get-MongoEndpoint $MongoUri

if (Test-Mongo $MongoUri) {
    Write-Ok "reachable at $($ep.Host):$($ep.Port)"
} elseif ($ep.Host -notin @('localhost', '127.0.0.1', '::1')) {
    # A remote host we cannot reach is a config problem, not something to fix
    # by installing a local server that would not hold the expected data.
    throw "cannot reach MongoDB at $($ep.Host):$($ep.Port). Check the host, port and firewall, or pass -MongoUri for a different server."
} else {
    Write-Note "nothing listening on $($ep.Host):$($ep.Port) - installing MongoDB"

    if (-not (Test-Admin)) {
        throw "installing MongoDB needs an elevated shell. Re-run this script from an Administrator PowerShell, or start MongoDB yourself and re-run."
    }

    if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
        Write-Note 'installing Chocolatey'
        Set-ExecutionPolicy Bypass -Scope Process -Force
        Invoke-Expression ((New-Object Net.WebClient).DownloadString(
            'https://community.chocolatey.org/install.ps1'))
        # choco lands in the machine PATH; refresh this session so it resolves.
        $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                    [Environment]::GetEnvironmentVariable('Path', 'User')
    }

    Write-Note 'choco install mongodb (this takes a few minutes)'
    & choco install mongodb --yes --no-progress
    if ($LASTEXITCODE -ne 0) { throw "choco install mongodb failed (exit $LASTEXITCODE)" }

    # The package registers a Windows service; make sure it is up and automatic.
    $svc = Get-Service -Name 'MongoDB' -ErrorAction SilentlyContinue
    if ($svc) {
        Set-Service -Name 'MongoDB' -StartupType Automatic
        if ($svc.Status -ne 'Running') { Start-Service -Name 'MongoDB' }
    }

    # Service start returns before the socket is accepting connections.
    Write-Note 'waiting for MongoDB to accept connections'
    $deadline = (Get-Date).AddSeconds(90)
    while (-not (Test-Mongo $MongoUri) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 2
    }
    if (-not (Test-Mongo $MongoUri)) {
        throw 'MongoDB was installed but never started listening. Check the MongoDB service.'
    }
    Write-Ok "installed and listening on $($ep.Host):$($ep.Port)"
}

# ── 2. Server binaries ────────────────────────────────────────────────────────

Write-Step 'Installing server'
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$asset = 'x86_64-pc-windows-msvc.zip'
$api = "https://api.github.com/repos/$Repo/releases/latest"
Write-Note "fetching latest release of $Repo"
$release = Invoke-RestMethod -Uri $api -Headers @{
    'User-Agent' = 'annnekkk-setup'
    'Accept'     = 'application/vnd.github+json'
}
$url = ($release.assets | Where-Object name -eq $asset | Select-Object -First 1).browser_download_url
if (-not $url) {
    $have = ($release.assets | ForEach-Object name) -join ', '
    throw "release $($release.tag_name) has no $asset (assets: $have)"
}

$zip = Join-Path ([IO.Path]::GetTempPath()) $asset
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

# A running instance from an earlier run would lock the .exe against overwrite.
Get-Process -Name 'annnekkk_random_server' -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue

Expand-Archive -Path $zip -DestinationPath $InstallDir -Force
Remove-Item $zip -Force
Write-Ok "$($release.tag_name) -> $InstallDir"

$exe = Join-Path $InstallDir 'annnekkk_random_server.exe'
$seedExe = Join-Path $InstallDir 'seed.exe'
if (-not (Test-Path $exe)) { throw "archive did not contain annnekkk_random_server.exe" }

# ── 3. Configuration ──────────────────────────────────────────────────────────

Write-Step 'Writing .env'
$envPath = Join-Path $InstallDir '.env'
@"
MONGODB_URI=$MongoUri
MONGODB_DB=$MongoDb

RUST_LOG=info

HOST=$ListenHost
PORT=$Port
"@ | Set-Content -Path $envPath -Encoding UTF8
Write-Ok $envPath

# ── 4. Seed ───────────────────────────────────────────────────────────────────

Write-Step 'Seeding database'
if (Test-Path $seedExe) {
    # seed downloads data.zip itself when the data dir is empty, and skips
    # collections that already have documents unless --force is passed.
    $seedArgs = @($MongoUri, $MongoDb)
    if ($Force) { $seedArgs += '--force' }

    Push-Location $InstallDir
    try {
        & $seedExe @seedArgs
        if ($LASTEXITCODE -ne 0) { throw "seed failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    Write-Ok 'seeded'
} else {
    Write-Note 'seed.exe not in the archive - skipping'
}

# ── 5. Launch at startup ──────────────────────────────────────────────────────

if ($NoAutoStart) {
    Write-Step 'Skipping startup registration (-NoAutoStart)'
} else {
    Write-Step 'Registering startup task'
    # A scheduled task at logon, rather than a service: it needs no extra
    # wrapper to host a console binary and installs without admin rights.
    $action = New-ScheduledTaskAction -Execute $exe -WorkingDirectory $InstallDir
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    $settings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -ExecutionTimeLimit ([TimeSpan]::Zero)

    # MongoDB's service may still be starting at logon; retry rather than fail.
    $settings.RestartInterval = 'PT1M'
    $settings.RestartCount = 3

    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
        -Settings $settings -Description 'annnekkk random data server' -Force | Out-Null
    Write-Ok "scheduled task '$TaskName' (runs at logon)"

    Start-ScheduledTask -TaskName $TaskName

    # Confirm it actually serves traffic rather than just reporting success.
    # Startup loads the whole reference-data cache before binding, which against
    # a remote MongoDB can take a couple of minutes on a cold connection.
    $waitSec = 180
    Write-Note "waiting for the server to respond (up to $waitSec s)"
    $url = "http://$ListenHost`:$Port/randomdatav2/new"
    $deadline = (Get-Date).AddSeconds($waitSec)
    $ok = $false
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 5
            if ($r.StatusCode -eq 200) { $ok = $true; break }
        } catch { }
        Start-Sleep -Seconds 2
    }
    if ($ok) {
        Write-Ok "serving on http://$ListenHost`:$Port"
    } else {
        # The task may simply still be warming up; say so rather than implying failure.
        Write-Warning "no response on $url after $waitSec s. It may still be loading - check with:"
        Write-Warning "  Get-ScheduledTaskInfo -TaskName $TaskName"
        Write-Warning "  curl `"$url`""
    }
}

Write-Host ''
Write-Host 'Done.' -ForegroundColor Green
Write-Host "  server    http://$ListenHost`:$Port" -ForegroundColor Gray
Write-Host "  try       curl `"http://$ListenHost`:$Port/randomdatav2/new`"" -ForegroundColor Gray
Write-Host "  config    $envPath" -ForegroundColor Gray
if (-not $NoAutoStart) {
    Write-Host "  uninstall Unregister-ScheduledTask -TaskName $TaskName -Confirm:`$false" -ForegroundColor Gray
}
