<#
.SYNOPSIS
    Install the snitch probe on a Windows host (Block 61H — multi-platform builds).

.DESCRIPTION
    Downloads the windows/<arch> binary from the collector and registers a
    Windows Service named "SnitchProbe" that probes every 60s and reports
    to the collector. Idempotent: safe to re-run; updates binary in place
    and recreates the service definition.

    The service runs as LocalSystem (default) so it can ICMP-ping the
    gateway and read the default route via Get-NetRoute. The probe does
    not need elevated rights for relay TCP / TLS probes — those use Go's
    net.DialTimeout. Operators who want to drop privileges can pass
    -ServiceAccount LocalService; ICMP / interface enumeration may
    silently degrade in that mode.

.PARAMETER Collector
    Snitch collector base URL (required; or set SNITCH_COLLECTOR).

.PARAMETER OperatorKey
    Operator-role API key for the binary download. Required.

.PARAMETER ProbeKey
    Probe-role API key the daemon uses to POST reports. Recommended.

.PARAMETER Interval
    Probe interval in seconds (default 60).

.PARAMETER ServiceAccount
    Account under which the service runs: "LocalSystem" (default) or "LocalService".

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File install-probe-windows.ps1 `
      -OperatorKey "<operator key>" `
      -ProbeKey "<probe key>"
#>

[CmdletBinding()]
param(
    [string]$Collector    = $env:SNITCH_COLLECTOR,
    [string]$OperatorKey  = $env:SNITCH_OPERATOR_KEY,
    [string]$ProbeKey     = $env:SNITCH_PROBE_KEY,
    [int]$Interval        = 60,
    [ValidateSet("LocalSystem", "LocalService")]
    [string]$ServiceAccount = "LocalSystem"
)

$ErrorActionPreference = "Stop"
if (-not $Collector) { Write-Error "snitch: no collector URL: pass -Collector <url> or set SNITCH_COLLECTOR"; exit 1 }

function Write-Log { param($Message) Write-Host "[snitch-windows] $(Get-Date -Format HH:mm:ss) $Message" }

# Require admin
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error "ERROR: This installer must run as Administrator."
    exit 1
}

if (-not $OperatorKey) {
    Write-Error "ERROR: -OperatorKey or `$env:SNITCH_OPERATOR_KEY is required to download the binary."
    exit 1
}

# Detect architecture
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "amd64" }
    "ARM64" { "arm64" }
    default { Write-Error "ERROR: Unsupported Windows architecture: $($env:PROCESSOR_ARCHITECTURE)"; exit 1 }
}

$serviceName  = "SnitchProbe"
$installDir   = "C:\Program Files\Snitch"
$binaryPath   = Join-Path $installDir "snitch.exe"
$logPath      = Join-Path $installDir "snitch-probe.log"

# Create install dir
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir | Out-Null
    Write-Log "Created $installDir"
}

# Download binary
Write-Log "Downloading snitch.exe for windows/$arch..."
$tmp = Join-Path $env:TEMP "snitch-$(Get-Random).exe"
try {
    $headers = @{ "Authorization" = "Bearer $OperatorKey" }
    $url = "$Collector/bin/snitch?os=windows&arch=$arch"
    Invoke-WebRequest -Uri $url -Headers $headers -OutFile $tmp -UseBasicParsing
} catch {
    Write-Error "ERROR: Could not download binary from $url : $_"
    if (Test-Path $tmp) { Remove-Item $tmp -Force }
    exit 1
}

# If the service is running, stop it before we overwrite the binary.
$svc = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($svc -and $svc.Status -eq "Running") {
    Write-Log "Stopping existing $serviceName service..."
    Stop-Service -Name $serviceName -Force
}

Move-Item -Path $tmp -Destination $binaryPath -Force
Write-Log "Installed $binaryPath"

# Pre-flight self-test (Block 61H). Verifies the binary runs before we
# register the service. Non-fatal — operators review and decide.
Write-Log "Running pre-flight self-test (10s)..."
try {
    & $binaryPath self-test --timeout 10s --pretty=false 2>&1 | Out-Host
} catch {
    Write-Log "WARNING: self-test errored ($_). Service will be registered anyway."
}

# Build the service ExecStart-equivalent. sc.exe binPath= takes a single
# string, so we quote the binary + args. Probe-key is passed via env var
# to avoid logging it in `sc qc` output.
$svcArgs = "`"$binaryPath`" probe --collector `"$Collector`" --interval $Interval"

# Remove old service definition if present
if ($svc) {
    Write-Log "Removing existing $serviceName service definition..."
    sc.exe delete $serviceName | Out-Null
    Start-Sleep -Seconds 2
}

# Create the service. obj= and password= determine the run-as account.
$svcAccount = switch ($ServiceAccount) {
    "LocalSystem"   { "LocalSystem" }
    "LocalService"  { "NT AUTHORITY\LocalService" }
}
Write-Log "Registering service $serviceName (run-as: $ServiceAccount)..."
$null = sc.exe create $serviceName `
    binPath= $svcArgs `
    DisplayName= "SNITCH mesh performance probe" `
    start= auto `
    obj= $svcAccount
sc.exe description $serviceName "Mesh-health probe daemon (snitch / Block 61H multi-platform builds). Reports every $Interval seconds to $Collector." | Out-Null
sc.exe failure $serviceName reset= 86400 actions= restart/30000/restart/30000/restart/30000 | Out-Null

# Set environment for the service via the service-environment registry key.
# REG_MULTI_SZ list with one entry per VAR=value line.
$svcRegPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName"
$envEntries = @(
    "SNITCH_COLLECTOR=$Collector",
    "SNITCH_PROBE_KEY=$ProbeKey"
)
Set-ItemProperty -Path $svcRegPath -Name "Environment" -Value $envEntries -Type MultiString
Write-Log "Registered service environment ($($envEntries.Count) variables)"

# Start it
Start-Service -Name $serviceName
Start-Sleep -Seconds 2
$svc = Get-Service -Name $serviceName
Write-Log "Service status: $($svc.Status)"

Write-Log "snitch-probe installed and running on Windows."
Write-Log "  Collector: $Collector"
Write-Log "  Interval:  ${Interval}s"
Write-Log "  Auth:      $(if ($ProbeKey) { 'configured' } else { 'NONE' })"
Write-Log "  Logs:      Get-Service $serviceName ; eventvwr.msc (Windows Logs > Application)"
Write-Log "  Stop:      Stop-Service $serviceName"
Write-Log "  Verify:    & '$binaryPath' self-test"
