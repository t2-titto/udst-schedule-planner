# Overview (imported)

> Exported from Thoughts Engineering · revision 2

## 1.1 · Overview `F1`

*Status: idea*

A fast, local-only schedule and degree planner for UDST students. Runs entirely on `http://localhost:8787` on your own Windows PC — **no admin rights required**, **no third-party servers**, and **writes to PeopleSoft are OFF by default** until you explicitly review and click Apply.

_Draft outline imported from https://github.com/t2-titto/udst-schedule-planner. Check it against the code before relying on it._

## 1.2 · One-Line Install (Windows, No Admin) `F2`

*Status: idea*

Choose any of the three installers below (all install into `%LOCALAPPDATA%\udst-schedule-planner`, build locally using official signed Rust tools so there are no unsigned `.exe` downloads, create a Desktop shortcut, and open `http://localhost:8787`):

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

_Draft outline imported from https://github.com/t2-titto/udst-schedule-planner. Check it against the code before relying on it._

## 1.3 · Features `F3`

*Status: idea*

- **Local & Private:** Credentials stay in memory only and are never saved to disk or sent anywhere except UDST PeopleSoft.
- **Built-in Tutorial:** Step-by-step walkthrough on first launch (and always available via **Tutorial** in the top bar).
- **Smart Schedule Builder:** Ranks conflict-free schedules by preferred times, free days, and instructor scores; skips `Closed` swap options automatically.
- **Degree Planner + Prerequisites & Co-requisites:** Drag courses across semesters with live 18/12-credit checks, prerequisite warnings, and one-click co-requisite pairing.
- **Multi-Account Support:** Use **Switch account** or open `http://localhost:8787` in a private window for a completely separate session.
- **Safe Apply:** One review screen with fresh seat checks before every write step.

_Draft outline imported from https://github.com/t2-titto/udst-schedule-planner. Check it against the code before relying on it._

## 1.4 · Code layout `F4`

*Status: idea*

Top-level folders of the repository.

_Draft outline imported from https://github.com/t2-titto/udst-schedule-planner. Check it against the code before relying on it._

### 1.4.1 · crates/ `F5`

*Status: idea*

3 entries.

### 1.4.2 · frontend/ `F6`

*Status: idea*

12 entries.

### 1.4.3 · server/ `F7`

*Status: idea*

3 entries.

