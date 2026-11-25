# UDST Schedule Planner -- Windows Installer (PowerShell, No Admin Required)
# Usage:  irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
# Safe to re-run: skips anything already installed, re-verifies, and just launches.
# Auto-updates: compares the installed version with the latest on GitHub and updates + rebuilds only when newer.
# Force re-download + rebuild:  $env:UDST_UPDATE = "1"; irm <url> | iex
# $env:UDST_NO_LAUNCH = "1" installs/updates without opening the planner (used by the launcher).

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$RepoZip    = "https://github.com/t2-titto/udst-schedule-planner/archive/refs/heads/main.zip"
$MingwZip   = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
$InstallDir = Join-Path $env:LOCALAPPDATA "udst-schedule-planner"
$AppDir     = Join-Path $InstallDir "app"
$MingwDir   = Join-Path $InstallDir "mingw64"
$CargoBin   = Join-Path $env:USERPROFILE ".cargo\bin"
$ExePath    = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
$Launcher   = Join-Path $InstallDir "Start-UDST-Planner.bat"
$Update     = ($env:UDST_UPDATE -eq "1")
$NoLaunch   = ($env:UDST_NO_LAUNCH -eq "1")
$VersionFile = Join-Path $InstallDir "version.txt"

function Step($n, $msg) { Write-Host "  [$n/5] " -ForegroundColor Cyan -NoNewline; Write-Host $msg -ForegroundColor White }
function Ok($msg)   { Write-Host "        [OK] $msg" -ForegroundColor Green }
function Info($msg) { Write-Host "        $msg" -ForegroundColor Gray }
function Run-Native($file, $argList, $what) {
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    & $file @argList
    $rc = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($rc -ne 0) { throw "$what failed with exit code $rc" }
}
function Download($url, $dest) {
    for ($i = 1; $i -le 3; $i++) {
        try { Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing; return }
        catch { if ($i -eq 3) { throw "Download failed ($url): $($_.Exception.Message). Check your internet connection / VPN / proxy." }; Start-Sleep -Seconds (2 * $i) }
    }
}
function Get-RemoteSha {
    # Latest commit SHA of main on GitHub; $null if offline / rate-limited (then we just keep what is installed).
    try {
        $h = @{ "Accept" = "application/vnd.github.sha"; "User-Agent" = "udst-schedule-planner-installer" }
        $s = (Invoke-RestMethod -Uri "https://api.github.com/repos/t2-titto/udst-schedule-planner/commits/main" -Headers $h -TimeoutSec 10 -UseBasicParsing)
        $s = ("$s").Trim()
        if ($s -match "^[0-9a-f]{40}$") { return $s }
    } catch {}
    return $null
}
function Stop-Planner {
    # Stop only OUR server (running from $ExePath) so the .exe is not locked.
    $procs = Get-Process -Name "schedule-planner-server" -ErrorAction SilentlyContinue
    foreach ($p in $procs) {
        $path = $null; try { $path = $p.Path } catch {}
        if (-not $path -or $path -eq $ExePath) { Info "Stopping running planner (PID $($p.Id))..."; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    }
    if ($procs) { Start-Sleep -Seconds 1 }
}

Write-Host ""
Write-Host "  ============================================================" -ForegroundColor Cyan
Write-Host "       UDST Schedule & Degree Planner  --  Windows Setup"       -ForegroundColor White
Write-Host "       Local-only | No Admin Required | Safe to re-run"          -ForegroundColor DarkCyan
Write-Host "  ============================================================" -ForegroundColor Cyan
Write-Host ""

# Only one installer at a time
$created = $false
$Mutex = New-Object System.Threading.Mutex($true, "Local\UDSTSchedulePlannerInstall", [ref]$created)
if (-not $created) { Write-Host "  Another installer is already running. Wait for it to finish, then retry." -ForegroundColor Yellow; return }

try {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $alreadyBuilt = (Test-Path $ExePath) -and (Test-Path (Join-Path $AppDir "frontend\dist\index.html")) -and (Test-Path (Join-Path $AppDir "launch.ps1"))

    $Downloaded = $false
    $RemoteSha = Get-RemoteSha
    $LocalSha = $null
    if (Test-Path $VersionFile) { $LocalSha = (Get-Content $VersionFile -ErrorAction SilentlyContinue | Select-Object -First 1) }
    if ($alreadyBuilt -and $RemoteSha -and $RemoteSha -ne $LocalSha) {
        Info "New version available ($($RemoteSha.Substring(0,7))); updating automatically..."
        $Update = $true
    } elseif ($alreadyBuilt -and -not $RemoteSha) {
        Info "Could not check for updates (offline?); using the installed version."
    }

    # 1. Download source
    Step 1 "Checking app files in $InstallDir..."
    if ($alreadyBuilt -and -not $Update) {
        Ok "Already downloaded and up to date."
    } else {
        Stop-Planner
        $ZipFile    = Join-Path $env:TEMP "udst-schedule-planner-main.zip"
        $ExtractDir = Join-Path $env:TEMP "udst-schedule-planner-extract"
        Remove-Item -Recurse -Force $ExtractDir -ErrorAction SilentlyContinue
        Remove-Item -Force $ZipFile -ErrorAction SilentlyContinue
        Download $RepoZip $ZipFile
        try { Expand-Archive -Path $ZipFile -DestinationPath $ExtractDir -Force }
        catch { throw "Downloaded file is corrupt ($($_.Exception.Message)). Re-run the installer." }
        $Inner = Get-ChildItem -Path $ExtractDir -Directory | Select-Object -First 1
        if (-not $Inner -or -not (Test-Path (Join-Path $Inner.FullName "server\Cargo.toml"))) { throw "Downloaded archive looks incomplete. Re-run the installer." }
        New-Item -ItemType Directory -Force -Path $AppDir | Out-Null
        # copy over existing app but KEEP server\target (build cache) so updates rebuild fast
        & robocopy $Inner.FullName $AppDir /E /XD target /NFL /NDL /NJH /NJS /NP | Out-Null
        if ($LASTEXITCODE -ge 8) { throw "Copying app files failed (robocopy code $LASTEXITCODE). Is a file open in another program?" }
        Remove-Item -Force $ZipFile -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $ExtractDir -ErrorAction SilentlyContinue
        $Downloaded = $true
        Ok "Downloaded app and prebuilt web UI."
    }

    # 2. Rust toolchain
    Step 2 "Checking Rust toolchain..."
    if (Test-Path $CargoBin) { $env:PATH = "$CargoBin;$env:PATH" }
    $needBuild = $Update -or $Downloaded -or -not (Test-Path $ExePath)
    $hasCargo = Get-Command cargo -ErrorAction SilentlyContinue
    $hostInfo = ""
    if ($hasCargo) {
        $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
        $out = (& rustc -vV 2>&1 | Out-String); $rc = $LASTEXITCODE
        $ErrorActionPreference = $prev
        if ($rc -eq 0 -and $out -match "host:\s*(\S+)") { $hostInfo = $Matches[1] }
    }
    $needGnu = $false
    if (-not $needBuild) {
        Ok "Binary already built; toolchain not needed."
    } elseif (-not $hasCargo) {
        Info "Installing official signed Rust toolchain (GNU target, user-only)..."
        $Rustup = Join-Path $env:TEMP "rustup-init.exe"
        Download "https://win.rustup.rs/x86_64" $Rustup
        Run-Native $Rustup @("-y","--default-host","x86_64-pc-windows-gnu","--profile","minimal","--no-modify-path") "rustup-init"
        Remove-Item -Force $Rustup -ErrorAction SilentlyContinue
        $env:PATH = "$CargoBin;$env:PATH"; $needGnu = $true
        Ok "Installed Rust in $env:USERPROFILE\.cargo"
    } elseif (-not $hostInfo) {
        Info "Rustup found without a working toolchain; installing stable-x86_64-pc-windows-gnu..."
        Run-Native rustup @("toolchain","install","stable-x86_64-pc-windows-gnu","--profile","minimal") "rustup toolchain install"
        Run-Native rustup @("default","stable-x86_64-pc-windows-gnu") "rustup default"
        $needGnu = $true
        Ok "Configured default Rust GNU toolchain."
    } else {
        if ($hostInfo -match "gnu") { $needGnu = $true }
        Ok "Found existing Cargo ($hostInfo)."
    }
    if ($needBuild -and $needGnu) {
        $mingwBin = Join-Path $MingwDir "bin"
        if (Test-Path $mingwBin) { $env:PATH = "$mingwBin;$env:PATH" }
        if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
            Info "Downloading portable MinGW-w64 (no admin, ~1-2 min)..."
            $MZip = Join-Path $env:TEMP "winlibs-mingw.zip"
            Remove-Item -Recurse -Force $MingwDir -ErrorAction SilentlyContinue   # drop any half-extracted copy
            Download $MingwZip $MZip
            Expand-Archive -Path $MZip -DestinationPath $InstallDir -Force
            Remove-Item -Force $MZip -ErrorAction SilentlyContinue
            $env:PATH = "$mingwBin;$env:PATH"
            if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) { throw "GCC not found after extracting MinGW." }
            Ok "Extracted portable GCC toolchain."
        } else { Ok "GCC already available." }
    }

    # 3. Build
    Step 3 "Checking server binary..."
    if ($needBuild) {
        Stop-Planner
        Info "Compiling (first build ~1-2 minutes)..."
        Push-Location (Join-Path $AppDir "server")
        try { Run-Native cargo @("build","--release") "cargo build" } finally { Pop-Location }
        if (-not (Test-Path $ExePath)) { throw "Built binary not found at $ExePath" }
        Ok "Built $ExePath"
        if ($RemoteSha) { Set-Content -Path $VersionFile -Value $RemoteSha -Encoding ASCII }
    } else {
        Ok "Binary present: $ExePath"
    }

    # 4. Launcher + shortcut (always rewritten: cheap, repairs deleted/broken ones, never duplicates)
    Step 4 "Checking launcher and Desktop shortcut..."
    # launch.ps1 owns launcher + shortcut creation (same code the launcher uses to self-heal)
    Run-Native powershell @("-NoProfile","-ExecutionPolicy","Bypass","-File",(Join-Path $AppDir "launch.ps1"),"-SetupOnly") "shortcut setup"
    if (-not (Test-Path $Launcher)) { throw "Launcher was not created at $Launcher" }
    Ok "Launcher + Desktop/Start Menu shortcuts ready. Keyboard shortcut: Ctrl+Alt+U"

    # 5. Launch (launcher handles 'already running' and busy ports)
    Step 5 "Launching..."
    Write-Host ""
    Write-Host "  ============================================================" -ForegroundColor Green
    Write-Host "    Ready!  Next time press Ctrl+Alt+U, or double-click the Desktop shortcut,"   -ForegroundColor White
    Write-Host "    or run:  cmd /c `"$Launcher`""                                -ForegroundColor Cyan
    Write-Host "  ============================================================" -ForegroundColor Green
    Write-Host ""
    if (-not $NoLaunch) { Start-Process -FilePath "cmd.exe" -ArgumentList "/k", "`"$Launcher`"" }
} finally {
    if ($Mutex) { $Mutex.ReleaseMutex(); $Mutex.Dispose() }
}
