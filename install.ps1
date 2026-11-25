# UDST Schedule Planner — Windows Installer (PowerShell, No Admin Required)
# Usage: irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$RepoZip   = "https://github.com/t2-titto/udst-schedule-planner/archive/refs/heads/main.zip"
$MingwZip  = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
$InstallDir = Join-Path $env:LOCALAPPDATA "udst-schedule-planner"
$AppDir     = Join-Path $InstallDir "app"
$MingwDir   = Join-Path $InstallDir "mingw64"
$CargoBin   = Join-Path $env:USERPROFILE ".cargo\bin"

function Banner {
    Write-Host ""
    Write-Host "  ============================================================" -ForegroundColor Cyan
    Write-Host "       UDST Schedule & Degree Planner  --  Windows Setup"       -ForegroundColor White
    Write-Host "       Local-only | No Admin Required | Builds Locally"         -ForegroundColor DarkCyan
    Write-Host "  ============================================================" -ForegroundColor Cyan
    Write-Host ""
}

function Step($n, $msg) {
    Write-Host "  [$n/5] " -ForegroundColor Cyan -NoNewline
    Write-Host $msg -ForegroundColor White
}

function Ok($msg) {
    Write-Host "        [OK] $msg" -ForegroundColor Green
}

function Info($msg) {
    Write-Host "        $msg" -ForegroundColor Gray
}

Banner

# 1. Download source + prebuilt frontend
Step 1 "Downloading UDST Schedule Planner into $InstallDir..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$ZipFile = Join-Path $env:TEMP "udst-schedule-planner-main.zip"
$ExtractDir = Join-Path $env:TEMP "udst-schedule-planner-extract"
if (Test-Path $ExtractDir) { Remove-Item -Recurse -Force $ExtractDir }
Invoke-WebRequest -Uri $RepoZip -OutFile $ZipFile -UseBasicParsing
Expand-Archive -Path $ZipFile -DestinationPath $ExtractDir -Force
$Inner = Get-ChildItem -Path $ExtractDir -Directory | Select-Object -First 1
if (Test-Path $AppDir) { Remove-Item -Recurse -Force $AppDir }
Move-Item -Path $Inner.FullName -Destination $AppDir
Remove-Item -Force $ZipFile -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $ExtractDir -ErrorAction SilentlyContinue
Ok "Downloaded app and prebuilt web UI."

# 2. Check Rust toolchain (no admin)
Step 2 "Checking Rust toolchain (no admin required)..."
if (Test-Path $CargoBin) { $env:PATH = "$CargoBin;$env:PATH" }
$hasCargo = Get-Command cargo -ErrorAction SilentlyContinue
$needGnu = $false
if (-not $hasCargo) {
    Info "Installing official signed Rust toolchain (GNU target, user-only)..."
    $Rustup = Join-Path $env:TEMP "rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $Rustup -UseBasicParsing
    & $Rustup -y --default-host x86_64-pc-windows-gnu --profile minimal --no-modify-path | Out-Null
    Remove-Item -Force $Rustup -ErrorAction SilentlyContinue
    $env:PATH = "$CargoBin;$env:PATH"
    $needGnu = $true
    Ok "Installed Rust in $env:USERPROFILE\.cargo"
} else {
    $hostInfo = (& rustc -vV 2>$null | Select-String "host:").ToString()
    if ($hostInfo -match "gnu") { $needGnu = $true }
    Ok "Found existing Cargo ($hostInfo)."
}

if ($needGnu) {
    $mingwBin = Join-Path $MingwDir "bin"
    if (Test-Path $mingwBin) { $env:PATH = "$mingwBin;$env:PATH" }
    if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
        Info "Downloading portable MinGW-w64 zip (no admin, ~1-2 min)..."
        $MZip = Join-Path $env:TEMP "winlibs-mingw.zip"
        Invoke-WebRequest -Uri $MingwZip -OutFile $MZip -UseBasicParsing
        Expand-Archive -Path $MZip -DestinationPath $InstallDir -Force
        Remove-Item -Force $MZip -ErrorAction SilentlyContinue
        $env:PATH = "$mingwBin;$env:PATH"
        Ok "Extracted portable GCC toolchain."
    }
}

# 3. Build locally (avoids unsigned .exe SmartScreen warnings)
Step 3 "Compiling local server binary (first build takes ~1-2 minutes)..."
Push-Location (Join-Path $AppDir "server")
try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}
$ExePath = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
if (-not (Test-Path $ExePath)) { throw "Built binary not found at $ExePath" }
Ok "Built $ExePath locally."

# 4. Create launcher & Desktop shortcut
Step 4 "Creating launcher and Desktop shortcut..."
$Launcher = Join-Path $InstallDir "Start-UDST-Planner.bat"
$LauncherContent = @"
@echo off
title UDST Schedule Planner (http://localhost:8787)
color 0B
echo.
echo  ============================================================
echo    UDST Schedule ^& Degree Planner is running!
echo.
echo    Click this link (or open in your browser):
echo      http://localhost:8787
echo.
echo    Keep this window open while using the planner.
echo    Press Ctrl+C or close this window to stop the server.
echo  ============================================================
echo.
cd /d "$AppDir\server"
start "" "http://localhost:8787"
"$ExePath"
pause
"@
Set-Content -Path $Launcher -Value $LauncherContent -Encoding ASCII

try {
    $Wsh = New-Object -ComObject WScript.Shell
    $Desktop = [Environment]::GetFolderPath("Desktop")
    $Shortcut = $Wsh.CreateShortcut((Join-Path $Desktop "UDST Schedule Planner.lnk"))
    $Shortcut.TargetPath = $Launcher
    $Shortcut.WorkingDirectory = Join-Path $AppDir "server"
    $Shortcut.Description = "UDST Schedule & Degree Planner (http://localhost:8787)"
    $Shortcut.Save()
    Ok "Created Desktop shortcut: UDST Schedule Planner"
} catch {
    Info "Skipped Desktop shortcut (you can run $Launcher directly)."
}

# 5. Launch CMD window with localhost link
Step 5 "Launching UDST Schedule Planner..."
Write-Host ""
Write-Host "  ============================================================" -ForegroundColor Green
Write-Host "    Installation complete!"                                     -ForegroundColor White
Write-Host "    Open in your browser:  http://localhost:8787"              -ForegroundColor Cyan
Write-Host "    Desktop shortcut created: UDST Schedule Planner"           -ForegroundColor Gray
Write-Host "  ============================================================" -ForegroundColor Green
Write-Host ""
Start-Process -FilePath "cmd.exe" -ArgumentList "/k", "`"$Launcher`""
