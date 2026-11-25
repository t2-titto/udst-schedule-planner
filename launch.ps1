# UDST Schedule Planner -- launcher (safe to run any time, any number of times)
# Run:  powershell -NoProfile -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\udst-schedule-planner\app\launch.ps1"
$ErrorActionPreference = "Stop"
$AppDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Exe    = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
$Base   = 8787

function Pause-Exit($code) { Write-Host ""; Read-Host "  Press Enter to close" | Out-Null; exit $code }

if (-not (Test-Path $Exe)) {
    Write-Host "  [ERROR] Not installed yet (missing $Exe)." -ForegroundColor Red
    Write-Host "  Run the installer once:" -ForegroundColor Yellow
    Write-Host "    irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex"
    Pause-Exit 1
}

function Get-Listener($port) {
    try { return Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction Stop | Select-Object -First 1 } catch { return $null }
}
function Port-Busy($port) {
    if (Get-Listener $port) { return $true }
    $l = $null
    try { $l = New-Object System.Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $port); $l.Start(); return $false }
    catch { return $true } finally { if ($l) { $l.Stop() } }
}

$port = $null
for ($p = $Base; $p -lt $Base + 10; $p++) {
    if (Port-Busy $p) {
        $c = Get-Listener $p
        $proc = if ($c) { Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue } else { $null }
        if ($proc -and $proc.ProcessName -eq "schedule-planner-server") {
            Write-Host "  Planner is already running on port $p -- opening it." -ForegroundColor Green
            Start-Process "http://localhost:$p"
            Start-Sleep -Seconds 2
            exit 0
        }
        Write-Host "  Port $p is used by another program; trying next port..." -ForegroundColor Yellow
        continue
    }
    $port = $p; break
}
if (-not $port) {
    Write-Host "  [ERROR] Ports $Base-$($Base+9) are all busy. Close other programs and retry." -ForegroundColor Red
    Pause-Exit 1
}

$Host.UI.RawUI.WindowTitle = "UDST Schedule Planner (http://localhost:$port)"
Write-Host ""
Write-Host "  UDST Schedule & Degree Planner is running at  http://localhost:$port" -ForegroundColor Cyan
Write-Host "  Keep this window open. Press Ctrl+C or close it to stop." -ForegroundColor Gray
Write-Host ""
$env:PORT = "$port"
Set-Location (Join-Path $AppDir "server")
Start-Process "http://localhost:$port"
& $Exe
if ($LASTEXITCODE -ne 0) { Write-Host "  Server exited with code $LASTEXITCODE" -ForegroundColor Red; Pause-Exit $LASTEXITCODE }
