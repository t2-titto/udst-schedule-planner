#!/usr/bin/env python3
"""UDST Schedule Planner -- Windows installer (Python entry point, no admin).

Delegates to install.ps1 so all installers share the same idempotent logic.
Usage:
  python -c "import urllib.request; exec(urllib.request.urlopen('https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.py').read())"
"""
import os
import subprocess
import sys

URL = "https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1"

if os.name != "nt":
    print("This installer is designed for Windows.")
    sys.exit(1)
sys.exit(subprocess.call([
    "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", f"irm {URL} | iex",
]))
