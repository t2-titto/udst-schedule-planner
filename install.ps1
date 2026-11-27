# UDST Schedule Planner -- Windows Installer (PowerShell, No Admin Required)
# Usage:  irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
# Safe to re-run: checks GitHub commit SHA, verifies archive integrity, keeps build cache, rebuilds if newer.
# Hermes-style commit update: downloads the exact latest GitHub commit archive and rebuilds locally.
# Force re-download + rebuild:  $env:UDST_UPDATE = "1"; irm <url> | iex
# $env:UDST_NO_LAUNCH = "1" installs/updates without opening the planner (used by the launcher).

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo        = "t2-titto/udst-schedule-planner"
$MingwZip    = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
$MingwSha256 = "3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1"
$InstallDir  = Join-Path $env:LOCALAPPDATA "udst-schedule-planner"
$AppDir      = Join-Path $InstallDir "app"
$MingwDir    = Join-Path $InstallDir "mingw64"
$CargoBin    = Join-Path $env:USERPROFILE ".cargo\bin"
$ExePath     = Join-Path $AppDir "server\target\release\schedule-planner-server.exe"
$Launcher    = Join-Path $InstallDir "Start-UDST-Planner.bat"
$Update      = ($env:UDST_UPDATE -eq "1")
$NoLaunch    = ($env:UDST_NO_LAUNCH -eq "1")
$TargetSha   = if ($env:UDST_TARGET_SHA -and ($env:UDST_TARGET_SHA -match "^[0-9a-fA-F]{40}$")) { $env:UDST_TARGET_SHA.Trim().ToLowerInvariant() } else { $null }
$VersionFile = Join-Path $InstallDir "version.txt"
$StageDir    = Join-Path $InstallDir ("tmp-" + [guid]::NewGuid().ToString("N"))

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
    try {
        $h = @{ "Accept" = "application/vnd.github.sha"; "User-Agent" = "udst-schedule-planner-installer" }
        $s = (Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/commits/main" -Headers $h -TimeoutSec 10 -UseBasicParsing)
        $s = ("$s").Trim().ToLowerInvariant()
        if ($s -match "^[0-9a-f]{40}$") { return $s }
    } catch {}
    return $null
}
function Verify-Checksums($rootDir) {
    $manifest = Join-Path $rootDir "CHECKSUMS.sha256"
    if (-not (Test-Path $manifest)) {
        throw "Integrity check failed: missing CHECKSUMS.sha256 in GitHub archive."
    }
    $lines = Get-Content -Path $manifest -Encoding ASCII | Where-Object { $_.Trim() -and -not $_.Trim().StartsWith("#") }
    $expectedFiles = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $checked = 0
    $prefixLen = $rootDir.TrimEnd('\','/').Length + 1
    foreach ($line in $lines) {
        if ($line -match "^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$") {
            $expected = $Matches[1].ToLowerInvariant()
            $relRaw = ($Matches[2] -replace '\\', '/').TrimStart('/')
            if ($relRaw -match "(^|/)\.\.(/|$)") { throw "Integrity check failed: invalid path $relRaw in manifest." }
            $rel = ($relRaw -replace '/', '\')
            $target = Join-Path $rootDir $rel
            if (-not (Test-Path $target -PathType Leaf)) { throw "Integrity check failed: missing $relRaw in GitHub archive." }
            $actual = (Get-FileHash -Path $target -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actual -ne $expected) { throw "Integrity check failed for $relRaw (expected $expected, got $actual)." }
            [void]$expectedFiles.Add($relRaw)
            $checked++
        }
    }
    if ($checked -lt 10) { throw "Integrity check failed: CHECKSUMS.sha256 manifest is unexpectedly small ($checked entries)." }
    $actualFiles = Get-ChildItem -Path $rootDir -Recurse -File
    foreach ($f in $actualFiles) {
        $relPath = ($f.FullName.Substring($prefixLen) -replace '\\', '/')
        if ($relPath -ieq "CHECKSUMS.sha256") { continue }
        if (-not $expectedFiles.Contains($relPath)) {
            throw "Integrity check failed: unexpected unverified file '$relPath' found in archive."
        }
    }
    Ok "Verified $checked file hashes (fail-closed) from GitHub commit manifest."
}
function Test-OwnedPlannerProc($proc) {
    $path = $null
    try { $path = $proc.Path } catch { return $false }
    if (-not $path -or ($path -ne $ExePath)) { return $false }
    try {
        $curSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $wmi = Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction Stop
        $owner = Invoke-CimMethod -InputObject $wmi -MethodName GetOwnerSid -ErrorAction Stop
        return ($owner.ReturnValue -eq 0 -and $owner.Sid -eq $curSid)
    } catch {
        return $false
    }
}
function Stop-Planner {
    $procs = Get-Process -Name "schedule-planner-server" -ErrorAction SilentlyContinue
    $stopped = $false
    foreach ($p in $procs) {
        if (Test-OwnedPlannerProc $p) {
            Info "Stopping running planner (PID $($p.Id))..."
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            $stopped = $true
        }
    }
    if ($stopped) { Start-Sleep -Seconds 1 }
}

Write-Host ""
Write-Host "  ============================================================" -ForegroundColor Cyan
Write-Host "       UDST Schedule & Degree Planner  --  Windows Setup"       -ForegroundColor White
Write-Host "       Local-only | No Admin Required | GitHub Verified"       -ForegroundColor DarkCyan
Write-Host "  ============================================================" -ForegroundColor Cyan
Write-Host ""

$created = $false
$Mutex = New-Object System.Threading.Mutex($true, "Local\UDSTSchedulePlannerInstall", [ref]$created)
if (-not $created) { Write-Host "  Another installer is already running. Wait for it to finish, then retry." -ForegroundColor Yellow; return }

try {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $alreadyBuilt = (Test-Path $ExePath) -and (Test-Path (Join-Path $AppDir "frontend\dist\index.html")) -and (Test-Path (Join-Path $AppDir "launch.ps1"))

    $Downloaded = $false
    $RemoteSha = if ($TargetSha) { $TargetSha } else { Get-RemoteSha }
    $LocalSha = $null
    if (Test-Path $VersionFile) {
        $rawLocal = (Get-Content $VersionFile -ErrorAction SilentlyContinue | Select-Object -First 1)
        if ($rawLocal) { $LocalSha = $rawLocal.Trim().ToLowerInvariant() }
    }
    if ($alreadyBuilt -and $RemoteSha -and $RemoteSha -ne $LocalSha) {
        Info "New GitHub commit available ($($RemoteSha.Substring(0,7))); updating..."
        $Update = $true
    } elseif ($alreadyBuilt -and -not $RemoteSha) {
        Info "Could not check GitHub for updates (offline?); using the installed version."
    }

    # 1. Download exact GitHub commit archive
    Step 1 "Checking app files in $InstallDir..."
    New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
    if ($alreadyBuilt -and -not $Update) {
        Ok "Already downloaded and up to date."
    } else {
        if (-not $RemoteSha) {
            throw "Could not resolve GitHub commit SHA (offline or rate-limited). Refusing unpinned download."
        }
        Stop-Planner
        $RepoZip    = "https://github.com/$Repo/archive/$RemoteSha.zip"
        $ZipFile    = Join-Path $StageDir "commit.zip"
        $ExtractDir = Join-Path $StageDir "extract"
        Info "Fetching exact commit archive ($($RemoteSha.Substring(0,7))) from GitHub..."
        Download $RepoZip $ZipFile
        try { Expand-Archive -Path $ZipFile -DestinationPath $ExtractDir -Force }
        catch { throw "Downloaded file is corrupt ($($_.Exception.Message)). Re-run the installer." }
        $Inner = Get-ChildItem -Path $ExtractDir -Directory | Select-Object -First 1
        if (-not $Inner -or -not (Test-Path (Join-Path $Inner.FullName "server\Cargo.toml")) -or -not (Test-Path (Join-Path $Inner.FullName "server\Cargo.lock"))) {
            throw "Downloaded archive looks incomplete. Re-run the installer."
        }
        Verify-Checksums $Inner.FullName
        New-Item -ItemType Directory -Force -Path $AppDir | Out-Null
        # Purge stale app files while preserving server\target (Cargo build cache) for fast rebuilds
        foreach ($entry in (Get-ChildItem -Path $AppDir -Force -ErrorAction SilentlyContinue)) {
            if ($entry.Name -ieq "server") {
                foreach ($sub in (Get-ChildItem -Path $entry.FullName -Force -ErrorAction SilentlyContinue)) {
                    if ($sub.Name -ine "target") { Remove-Item -Recurse -Force $sub.FullName }
                }
            } else {
                Remove-Item -Recurse -Force $entry.FullName
            }
        }
        & robocopy $Inner.FullName $AppDir /E /XD target /NFL /NDL /NJH /NJS /NP | Out-Null
        if ($LASTEXITCODE -ge 8) { throw "Copying app files failed (robocopy code $LASTEXITCODE). Is a file open in another program?" }
        $Downloaded = $true
        Ok "Downloaded commit and prebuilt web UI from GitHub."
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
        $Rustup = Join-Path $StageDir "rustup-init.exe"
        Download "https://win.rustup.rs/x86_64" $Rustup
        $sig = Get-AuthenticodeSignature -FilePath $Rustup
        if (-not $sig -or $sig.Status -ne 'Valid') {
            throw "Official rustup-init.exe signature verification failed (Status: $($sig.Status))."
        }
        Ok "Verified official Rust Authenticode signature."
        Run-Native $Rustup @("-y","--default-host","x86_64-pc-windows-gnu","--profile","minimal","--no-modify-path") "rustup-init"
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
            $MZip = Join-Path $StageDir "winlibs-mingw.zip"
            Remove-Item -Recurse -Force $MingwDir -ErrorAction SilentlyContinue
            Download $MingwZip $MZip
            $mActual = (Get-FileHash -Path $MZip -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($mActual -ne $MingwSha256) {
                throw "MinGW toolchain SHA-256 verification failed (expected $MingwSha256, got $mActual)."
            }
            Ok "Verified MinGW-w64 toolchain SHA-256 digest."
            Expand-Archive -Path $MZip -DestinationPath $InstallDir -Force
            $env:PATH = "$mingwBin;$env:PATH"
            if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) { throw "GCC not found after extracting MinGW." }
            Ok "Extracted portable GCC toolchain."
        } else { Ok "GCC already available." }
    }

    # 3. Build cleanly against Cargo.lock
    Step 3 "Building hardened release binary..."
    if ($needBuild) {
        Stop-Planner
        Info "Compiling release binary from verified commit..."
        Push-Location (Join-Path $AppDir "server")
        try { Run-Native cargo @("build","--release","--locked") "cargo build" } finally { Pop-Location }
        if (-not (Test-Path $ExePath)) { throw "Built binary not found at $ExePath" }
        Ok "Built $ExePath"
        if ($RemoteSha) { Set-Content -Path $VersionFile -Value $RemoteSha -Encoding ASCII }
    } else {
        Ok "Binary present: $ExePath"
    }

    # 4. Launcher + Desktop & Start Menu shortcuts (always created/repaired cleanly)
    Step 4 "Checking launcher and Desktop shortcut..."
    Run-Native powershell @("-NoProfile","-ExecutionPolicy","Bypass","-File",(Join-Path $AppDir "launch.ps1"),"-SetupOnly") "shortcut setup"
    if (-not (Test-Path $Launcher)) { throw "Launcher was not created at $Launcher" }
    Ok "Launcher + Desktop/Start Menu shortcuts ready. Hotkey: Ctrl+Alt+U"

    # 5. Launch
    Step 5 "Ready..."
    Write-Host ""
    Write-Host "  ============================================================" -ForegroundColor Green
    Write-Host "    Ready! Double-click 'UDST Schedule Planner' on your Desktop," -ForegroundColor White
    Write-Host "    press Ctrl+Alt+U, or run:  cmd /c `"$Launcher`""               -ForegroundColor Cyan
    Write-Host "  ============================================================" -ForegroundColor Green
    Write-Host ""
    if (-not $NoLaunch) { Start-Process -FilePath "cmd.exe" -ArgumentList "/k", "`"$Launcher`"" }
} finally {
    if ($StageDir -and (Test-Path $StageDir)) { Remove-Item -Recurse -Force $StageDir -ErrorAction SilentlyContinue }
    if ($Mutex) { $Mutex.ReleaseMutex(); $Mutex.Dispose() }
}
