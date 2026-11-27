# UDST Schedule & Degree Planner

A fast, local-only schedule and degree planner for UDST students. Runs entirely on `http://localhost:6767` on your own Windows PC — **no admin rights required**, **no third-party servers**, and **writes to PeopleSoft are OFF by default** until you explicitly review and click Apply.

## One-Line Install (Windows, No Admin)

Choose any of the three installers below (all share the same re-runnable logic and install into `C:\Users\<you>\AppData\Local\udst-schedule-planner`, build locally using official signed Rust tools so there are no unsigned `.exe` downloads, create a Desktop shortcut, and open `http://localhost:6767`):

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

1. **Press `Ctrl+Alt+U`** (keyboard shortcut), or **double-click** the **UDST Schedule Planner** shortcut on your Desktop / Start Menu.
2. **One command** - works the same in **PowerShell and Command Prompt** (copy it exactly, including `cmd /c`):
   ```
   cmd /c "%LOCALAPPDATA%\udst-schedule-planner\Start-UDST-Planner.bat"
   ```
   > Do not type just the folder path, and do not use `%LOCALAPPDATA%` by itself in PowerShell (it only expands in Command Prompt; the `cmd /c` above handles it for you).
3. **Lost the shortcut?** Just launch the planner any way above: every launch automatically recreates a missing Desktop/Start Menu shortcut and the `Ctrl+Alt+U` key. No reinstall needed. **If something else looks broken**, re-run the install command above. It is safe to run any number of times: it detects what is already downloaded and built, re-checks it, repairs the launcher/shortcut, and launches. It does **not** re-download or recompile.

Keep the black window open while using the planner; closing it (or Ctrl+C) stops the server.

### Updates check automatically with a CMD prompt
Every time you double-click the Desktop shortcut, press `Ctrl+Alt+U`, or run the command above (including after restarting Windows):
1. It makes sure your Desktop/Start Menu shortcut exists, checks GitHub for a newer commit, and tells you in the CMD window if an update is available.
2. If you type **Y** (or press Enter), it stops any old server, downloads the verified GitHub commit, checks `CHECKSUMS.sha256`, incrementally rebuilds the local binary cleanly from `Cargo.lock`, and starts the updated server + opens `http://localhost:6767`.
3. If you type **N** (or are offline), it skips the update and launches/opens your installed version.

**One-command install / update right now:**
```powershell
irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
```
To force a full re-download and rebuild even when already up to date:
```powershell
$env:UDST_UPDATE = "1"; irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex
```

### Edge cases handled
| Situation | What happens |
|---|---|
| Already installed and up to date | Installer skips download/build, repairs shortcut, launches |
| Newer version on GitHub | Downloaded, rebuilt and launched automatically (skipped if offline) |
| Planner already running when a launch/update is requested | Just opens the running one; updates apply at the next start |
| Planner already running | Launcher just opens your browser to it (no duplicate server) |
| Port 6767 used by another program | Launcher automatically uses the next free port (6768-6776) and opens that URL |
| Updating while the server is running | Installer stops only this app's server first so the `.exe` isn't locked |
| Installer started twice at once | Second one exits with a message |
| Interrupted / corrupt download, half-extracted MinGW | Detected and re-downloaded on next run; downloads retry 3 times |
| Rust already installed (any toolchain) | Reused; only installed if missing |
| Deleted Desktop/Start Menu shortcut or `Ctrl+Alt+U` | Recreated automatically on the next launch (never duplicated) |

### Troubleshooting
- **"Not installed yet"**, or you installed before `Ctrl+Alt+U` existed: run the install command once (it skips the build if already built).
- **Windows Firewall / antivirus prompt**: the server only listens on `127.0.0.1` (this PC); it is safe to allow or ignore.
- **Fresh start**: delete the folder `C:\Users\<you>\AppData\Local\udst-schedule-planner` and the Desktop shortcut, then re-run the install command.

## Features
- **Local & Private:** Credentials stay in memory only and are never saved to disk or sent anywhere except UDST PeopleSoft.
- **Built-in Tutorial:** Step-by-step walkthrough on first launch (and always available via **Tutorial** in the top bar).
- **Smart Schedule Builder:** Ranks conflict-free schedules by preferred times, free days, and instructor scores; skips `Closed` swap options automatically.
- **Degree Planner + Prerequisites & Co-requisites:** Drag courses across semesters with live 18/12-credit checks, prerequisite warnings, and one-click co-requisite pairing.
- **Multi-Account Support:** Use **Switch account** or open `http://localhost:6767` in a private window for a completely separate session.
- **Safe Apply:** One review screen with fresh seat checks before every write step.

## Code hardening (what it does and doesn't do)

The shipped binary is built with `strip = true`, `lto = true`, `codegen-units = 1`
(stripped debug symbols, heavier inlining) and the web UI is minified with Terser
(mangled names, no comments). This raises the effort needed to read the *shipped*
binary/JS back into something meaningful.

**It does not, and cannot, stop someone from reading the source on GitHub itself** --
the repo is public for transparency, and no amount of binary/JS hardening changes
that. The actual protection against copying, reverse engineering, or reuse is the
[LICENSE](LICENSE): it is legally required, hardening above is just a technical
speed bump on top of it.

## Disclaimer

This is an independent, student-built tool, **not affiliated with, endorsed by, or supported by UDST or Oracle/PeopleSoft**. It is a planning aid, not official academic advice -- always verify your final schedule and degree plan in PeopleSoft and with your advisor. Nothing is written to PeopleSoft until you explicitly review and approve it; once approved, you are responsible for that action and for following UDST's own policies on using it. Provided "as is," with no warranty, used at your own risk. See [DISCLAIMER.md](DISCLAIMER.md) for the full terms.

## License

**Proprietary - all rights reserved.** Free for personal, non-commercial use only. No commercial use, no reverse engineering, and no reuse of the code in any other app (commercial or not) without the developer's prior **written consent** (contact [t2-titto](https://github.com/t2-titto) on GitHub). You may share the unmodified installer link with other students. See [LICENSE](LICENSE) for the full terms.
