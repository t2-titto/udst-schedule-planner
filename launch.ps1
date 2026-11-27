# UDST Schedule Planner -- launcher (safe to run any time, any number of times)
# Every run self-heals: recreates the .bat launcher, Desktop shortcut and Start Menu shortcut
# (with the Ctrl+Alt+U keyboard shortcut) if any are missing. No reinstall needed.
# Every run checks GitHub for a newer commit and prompts inside this CMD window before updating.
# Run from any shell:  cmd /c "%LOCALAPPDATA%\udst-schedule-planner\Start-UDST-Planner.bat"
param([switch]$SetupOnly)
$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$AppDir     = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallDir = Split-Path -Parent $AppDir
$Exe        = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
$Launcher   = Join-Path $InstallDir "Start-UDST-Planner.bat"
$VersionFile = Join-Path $InstallDir "version.txt"
$Hotkey     = "CTRL+ALT+U"
$Base       = 6767
$Repo       = "t2-titto/udst-schedule-planner"

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
                if ($s.TargetPath -eq $Launcher -and -not $s.Arguments -and $s.Hotkey -eq $Hotkey -and $s.WorkingDirectory -eq (Join-Path $AppDir "server")) {
                    $needs = $false
                }
            }
            if ($needs) {
                $s = $wsh.CreateShortcut($path)
                $s.TargetPath = $Launcher
                $s.Arguments = ""
                $s.WorkingDirectory = Join-Path $AppDir "server"
                $s.Hotkey = $Hotkey
                $s.Description = "UDST Schedule & Degree Planner ($Hotkey)"
                $s.Save()
                Write-Host "  [OK] Desktop/Menu shortcut ready: $path ($Hotkey)" -ForegroundColor DarkGray
            }
        }
    } catch {
        Write-Host "  (Could not create shortcuts: $($_.Exception.Message))" -ForegroundColor DarkYellow
    }
}

function Test-OwnedPlannerProc($proc) {
    if (-not $proc) { return $false }
    $path = $null
    try { $path = $proc.Path } catch { return $false }
    if (-not $path -or ($path -ne $Exe)) { return $false }
    try {
        $curSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $wmi = Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction Stop
        $owner = Invoke-CimMethod -InputObject $wmi -MethodName GetOwnerSid -ErrorAction Stop
        return ($owner.ReturnValue -eq 0 -and $owner.Sid -eq $curSid)
    } catch {
        return $false
    }
}

function Stop-RunningPlanner {
    $procs = Get-Process -Name "schedule-planner-server" -ErrorAction SilentlyContinue
    $stopped = $false
    foreach ($p in $procs) {
        if (Test-OwnedPlannerProc $p) {
            Write-Host "  Stopping running server (PID $($p.Id)) to apply update..." -ForegroundColor Yellow
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            $stopped = $true
        }
    }
    if ($stopped) { Start-Sleep -Seconds 1 }
}

function Get-RemoteCommitInfo {
    try {
        $h = @{ "Accept" = "application/vnd.github+json"; "User-Agent" = "udst-schedule-planner-launcher" }
        $resp = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/commits/main" -Headers $h -TimeoutSec 8 -UseBasicParsing
        if (-not $resp -or -not $resp.sha -or ($resp.sha -notmatch "^[0-9a-f]{40}$")) { return $null }
        $msg = ""
        if ($resp.commit -and $resp.commit.message) {
            $msg = (($resp.commit.message -split "`r?`n")[0]).Trim()
            if ($msg.Length -gt 68) { $msg = $msg.Substring(0, 65) + "..." }
        }
        $verified = $false
        if ($resp.commit -and $resp.commit.verification) { $verified = [bool]$resp.commit.verification.verified }
        return @{
            Sha      = ([string]$resp.sha).Trim().ToLowerInvariant()
            Message  = $msg
            Verified = $verified
        }
    } catch {
        return $null
    }
}

function Prompt-And-Update {
    Write-Host "  Checking GitHub for updates..." -ForegroundColor DarkGray
    $info = Get-RemoteCommitInfo
    if (-not $info) {
        Write-Host "  (Offline or GitHub unreachable -- using installed version.)" -ForegroundColor DarkYellow
        return
    }
    $local = $null
    if (Test-Path $VersionFile) {
        $rawLocal = (Get-Content $VersionFile -ErrorAction SilentlyContinue | Select-Object -First 1)
        if ($rawLocal) { $local = $rawLocal.Trim().ToLowerInvariant() }
    }
    if ($local -eq $info.Sha) {
        Write-Host "  [OK] Up to date (GitHub commit $($local.Substring(0,7)))." -ForegroundColor DarkGreen
        return
    }

    $localShort = if ($local -and $local.Length -ge 7) { $local.Substring(0, 7) } else { "unknown" }
    $remoteShort = $info.Sha.Substring(0, 7)
    $sigNote = if ($info.Verified) { "GitHub-verified signed commit" } else { "unsigned GitHub commit; SHA-256 manifest checked" }
    Write-Host ""
    Write-Host "  ============================================================" -ForegroundColor Yellow
    Write-Host "    UPDATE AVAILABLE FROM GITHUB ($sigNote)" -ForegroundColor White
    Write-Host "    Installed : $localShort" -ForegroundColor Gray
    Write-Host "    Latest    : $remoteShort  --  $($info.Message)" -ForegroundColor Cyan
    Write-Host "  ============================================================" -ForegroundColor Yellow
    Write-Host ""
    $ans = Read-Host "  Do you want to update from GitHub and rebuild now? [y/N]"
    if (-not $ans -or ($ans.Trim() -notmatch "^(y|yes)$")) {
        Write-Host "  Update skipped. Launching currently installed version..." -ForegroundColor Gray
        return
    }

    Stop-RunningPlanner
    Write-Host "  Downloading verified updater for commit $remoteShort from GitHub..." -ForegroundColor Cyan
    $updStage = Join-Path $InstallDir ("upd-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Force -Path $updStage | Out-Null
        $manifestPath = Join-Path $updStage "CHECKSUMS.sha256"
        $instPath     = Join-Path $updStage "install.ps1"
        Invoke-WebRequest -Uri "https://raw.githubusercontent.com/$Repo/$($info.Sha)/CHECKSUMS.sha256" -OutFile $manifestPath -UseBasicParsing
        Invoke-WebRequest -Uri "https://raw.githubusercontent.com/$Repo/$($info.Sha)/install.ps1" -OutFile $instPath -UseBasicParsing
        $expectedInstHash = $null
        foreach ($line in (Get-Content -Path $manifestPath -Encoding ASCII)) {
            if ($line -match "^\s*([0-9a-fA-F]{64})\s+\*?install\.ps1\s*$") {
                $expectedInstHash = $Matches[1].ToLowerInvariant()
                break
            }
        }
        if (-not $expectedInstHash) { throw "Missing install.ps1 entry in commit CHECKSUMS.sha256." }
        $actualInstHash = (Get-FileHash -Path $instPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualInstHash -ne $expectedInstHash) {
            throw "install.ps1 SHA-256 mismatch (expected $expectedInstHash, got $actualInstHash)."
        }

        $env:UDST_UPDATE = "1"
        $env:UDST_NO_LAUNCH = "1"
        $env:UDST_TARGET_SHA = $info.Sha
        & powershell -NoProfile -ExecutionPolicy Bypass -File $instPath
        if ($LASTEXITCODE -ne 0) {
            Write-Host "  [WARN] Update failed (exit $LASTEXITCODE); starting the installed version." -ForegroundColor Yellow
        } else {
            Write-Host "  [OK] Updated cleanly to commit $remoteShort." -ForegroundColor Green
        }
    } catch {
        Write-Host "  [WARN] Update failed ($($_.Exception.Message)); starting the installed version." -ForegroundColor Yellow
    } finally {
        if (Test-Path $updStage) { Remove-Item -Recurse -Force $updStage -ErrorAction SilentlyContinue }
        Remove-Item Env:\UDST_UPDATE, Env:\UDST_NO_LAUNCH, Env:\UDST_TARGET_SHA -ErrorAction SilentlyContinue
    }
}

function Pause-Exit($code) { Write-Host ""; Read-Host "  Press Enter to close" | Out-Null; exit $code }

Ensure-Shortcuts
if ($SetupOnly) { exit 0 }

if (-not (Test-Path $Exe)) {
    Write-Host "  [ERROR] Not installed yet (missing $Exe)." -ForegroundColor Red
    Write-Host "  Run the installer once:" -ForegroundColor Yellow
    Write-Host "    irm https://raw.githubusercontent.com/$Repo/main/install.ps1 | iex"
    Pause-Exit 1
}

# Always check for updates first (even if an older server instance is already running)
Prompt-And-Update
if (-not (Test-Path $Exe)) {
    Write-Host "  [ERROR] Missing $Exe after update. Re-run the installer." -ForegroundColor Red
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
        if (Test-OwnedPlannerProc $proc) {
            Write-Host "  Planner is already running on http://127.0.0.1:$p -- opening browser." -ForegroundColor Green
            Start-Process "http://127.0.0.1:$p"
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

$Host.UI.RawUI.WindowTitle = "UDST Schedule Planner (http://127.0.0.1:$port)"
Write-Host ""
Write-Host "  UDST Schedule & Degree Planner is running at  http://127.0.0.1:$port" -ForegroundColor Cyan
Write-Host "  Keep this window open. Press Ctrl+C or close it to stop." -ForegroundColor Gray
Write-Host ""
$env:PORT = "$port"
Set-Location (Join-Path $AppDir "server")
Start-Process "http://127.0.0.1:$port"
& $Exe
if ($LASTEXITCODE -ne 0) { Write-Host "  Server exited with code $LASTEXITCODE" -ForegroundColor Red; Pause-Exit $LASTEXITCODE }
