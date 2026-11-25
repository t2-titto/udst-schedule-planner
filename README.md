# UDST Schedule & Degree Planner

A fast, local-only schedule and degree planner for UDST students. Runs entirely on `http://localhost:8787` on your own Windows PC — **no admin rights required**, **no third-party servers**, and **writes to PeopleSoft are OFF by default** until you explicitly review and click Apply.

## One-Line Install (Windows, No Admin)

Choose any of the three installers below (all share the same re-runnable logic and install into `C:\Users\<you>\AppData\Local\udst-schedule-planner`, build locally using official signed Rust tools so there are no unsigned `.exe` downloads, create a Desktop shortcut, and open `http://localhost:8787`):

### Option 1: PowerShell (Recommended)
```powershell
irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
```

### Option 2: Command Prompt (`.bat`)
```cmd
curl -fsSL https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.bat -o "%TEMP%\udst-install.bat" && "%TEMP%\udst-install.bat"
```

### Option 3: Python (`install.py`)
```cmd
python -c "import urllib.request; exec(urllib.request.urlopen('https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.py').read())"
```

## Running It Again (after closing the computer, restarting, etc.)

You do **not** need to reinstall. Everything stays in `C:\Users\<you>\AppData\Local\udst-schedule-planner`. Pick one:

1. **Double-click** the **UDST Schedule Planner** shortcut on your Desktop (easiest).
2. **One command** - works the same in **PowerShell and Command Prompt** (copy it exactly, including `cmd /c`):
   ```
   cmd /c "%LOCALAPPDATA%\udst-schedule-planner\Start-UDST-Planner.bat"
   ```
   > Do not type just the folder path, and do not use `%LOCALAPPDATA%` by itself in PowerShell (it only expands in Command Prompt; the `cmd /c` above handles it for you).
3. **Lost the shortcut or something looks broken?** Re-run the install command above. It is safe to run any number of times: it detects what is already downloaded and built, re-checks it, repairs the launcher/shortcut, and launches. It does **not** re-download or recompile.

Keep the black window open while using the planner; closing it (or Ctrl+C) stops the server.

### Updating to the newest version
```powershell
$env:UDST_UPDATE = "1"; irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
```
This re-downloads the code, stops the old server if it is running, and rebuilds (fast, the build cache is kept).

### Edge cases handled
| Situation | What happens |
|---|---|
| Already installed | Installer skips download/build, repairs shortcut, launches |
| Planner already running | Launcher just opens your browser to it (no duplicate server) |
| Port 8787 used by another program | Launcher automatically uses the next free port (8788-8796) and opens that URL |
| Updating while the server is running | Installer stops only this app's server first so the `.exe` isn't locked |
| Installer started twice at once | Second one exits with a message |
| Interrupted / corrupt download, half-extracted MinGW | Detected and re-downloaded on next run; downloads retry 3 times |
| Rust already installed (any toolchain) | Reused; only installed if missing |
| Deleted Desktop shortcut | Recreated (never duplicated) |

### Troubleshooting
- **"Not installed yet"**: run the install command once.
- **Windows Firewall / antivirus prompt**: the server only listens on `127.0.0.1` (this PC); it is safe to allow or ignore.
- **Fresh start**: delete the folder `C:\Users\<you>\AppData\Local\udst-schedule-planner` and the Desktop shortcut, then re-run the install command.

## Features
- **Local & Private:** Credentials stay in memory only and are never saved to disk or sent anywhere except UDST PeopleSoft.
- **Built-in Tutorial:** Step-by-step walkthrough on first launch (and always available via **Tutorial** in the top bar).
- **Smart Schedule Builder:** Ranks conflict-free schedules by preferred times, free days, and instructor scores; skips `Closed` swap options automatically.
- **Degree Planner + Prerequisites & Co-requisites:** Drag courses across semesters with live 18/12-credit checks, prerequisite warnings, and one-click co-requisite pairing.
- **Multi-Account Support:** Use **Switch account** or open `http://localhost:8787` in a private window for a completely separate session.
- **Safe Apply:** One review screen with fresh seat checks before every write step.
