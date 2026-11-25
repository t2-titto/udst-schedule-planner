# UDST Schedule Planner -- launcher (safe to run any time, any number of times)
# Every run self-heals: recreates the .bat launcher, Desktop shortcut and Start Menu shortcut
# (with the Ctrl+Alt+U keyboard shortcut) if any are missing. No reinstall needed.
# Run from any shell:  cmd /c "%LOCALAPPDATA%\udst-schedule-planner\Start-UDST-Planner.bat"
param([switch]$SetupOnly)
$ErrorActionPreference = "Stop"
$AppDir     = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallDir = Split-Path -Parent $AppDir
$Exe        = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
$Launcher   = Join-Path $InstallDir "Start-UDST-Planner.bat"
$Hotkey     = "CTRL+ALT+U"
$Base       = 8787

function Ensure-Shortcuts {
    try {
        $content = "@echo off`r`ntitle UDST Schedule Planner`r`npowershell -NoProfile -ExecutionPolicy Bypass -File `"$AppDir\launch.ps1`"`r`n"
        $current = $null
        if (Test-Path $Launcher) { $current = [IO.File]::ReadAllText($Launcher) }
        if ($current -ne $content) { [IO.File]::WriteAllText($Launcher, $content, [Text.Encoding]::ASCII) }
        $wsh = New-Object -ComObject WScript.Shell
        # Hotkeys only work for shortcuts on the Desktop or in the Start Menu, so keep one in each.
        $dirs = @([Environment]::GetFolderPath("Desktop"), [Environment]::GetFolderPath("Programs"))
        foreach ($d in $dirs) {
            if (-not $d -or -not (Test-Path $d)) { continue }
            $path = Join-Path $d "UDST Schedule Planner.lnk"
            $needs = $true
            if (Test-Path $path) {
                $s = $wsh.CreateShortcut($path)
                if ($s.TargetPath -eq $Launcher -and $s.Hotkey -eq $Hotkey) { $needs = $false }
            }
            if ($needs) {
                $s = $wsh.CreateShortcut($path)
                $s.TargetPath = $Launcher
                $s.WorkingDirectory = Join-Path $AppDir "server"
                $s.Hotkey = $Hotkey
                $s.Description = "UDST Schedule & Degree Planner ($Hotkey)"
                $s.Save()
                Write-Host "  Restored shortcut: $path  (keyboard: Ctrl+Alt+U)" -ForegroundColor DarkGray
            }
        }
    } catch {
        Write-Host "  (Could not create shortcuts: $($_.Exception.Message))" -ForegroundColor DarkYellow
    }
}

function Pause-Exit($code) { Write-Host ""; Read-Host "  Press Enter to close" | Out-Null; exit $code }

Ensure-Shortcuts
if ($SetupOnly) { exit 0 }

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
    try { $l = New-Object System.Net.Sockets.TcpListener -ArgumentList ([Net.IPAddress]::Loopback), $port; $l.Start(); return $false }
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
