#!/usr/bin/env python3
"""UDST Schedule Planner -- Windows Installer (Python, No Admin Required).

Usage:
  python -c "import urllib.request; exec(urllib.request.urlopen('https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.py').read())"
"""
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

REPO_ZIP = "https://github.com/t2-titto/udst-schedule-planner/archive/refs/heads/main.zip"
MINGW_ZIP = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
LOCAL_APP = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
INSTALL_DIR = os.path.join(LOCAL_APP, "udst-schedule-planner")
APP_DIR = os.path.join(INSTALL_DIR, "app")
MINGW_DIR = os.path.join(INSTALL_DIR, "mingw64")
CARGO_BIN = os.path.join(os.path.expanduser("~"), ".cargo", "bin")


def banner() -> None:
    print("\n  ============================================================")
    print("       UDST Schedule & Degree Planner  --  Windows Setup")
    print("       Local-only | No Admin Required | Builds Locally")
    print("  ============================================================\n")


def step(n: int, msg: str) -> None:
    print(f"  [{n}/5] {msg}")


def ok(msg: str) -> None:
    print(f"        [OK] {msg}")


def download(url: str, dest: str) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": "udst-schedule-planner-installer"})
    with urllib.request.urlopen(req, timeout=180) as r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f)


def main() -> None:
    if os.name != "nt":
        print("This installer is designed for Windows.")
        sys.exit(1)

    banner()
    os.makedirs(INSTALL_DIR, exist_ok=True)
    env = os.environ.copy()
    if os.path.isdir(CARGO_BIN):
        env["PATH"] = CARGO_BIN + os.pathsep + env.get("PATH", "")

    # 1. Download app
    step(1, f"Downloading UDST Schedule Planner into {INSTALL_DIR}...")
    tmp_zip = os.path.join(tempfile.gettempdir(), "udst-schedule-planner-main.zip")
    tmp_ext = os.path.join(tempfile.gettempdir(), "udst-schedule-planner-extract")
    if os.path.exists(tmp_ext):
        shutil.rmtree(tmp_ext)
    download(REPO_ZIP, tmp_zip)
    with zipfile.ZipFile(tmp_zip, "r") as zf:
        zf.extractall(tmp_ext)
    inner = next(os.scandir(tmp_ext)).path
    if os.path.exists(APP_DIR):
        shutil.rmtree(APP_DIR)
    shutil.move(inner, APP_DIR)
    os.remove(tmp_zip)
    shutil.rmtree(tmp_ext, ignore_errors=True)
    ok("Downloaded app and prebuilt web UI.")

    # 2. Check Rust toolchain
    step(2, "Checking Rust toolchain (no admin required)...")
    need_gnu = False
    if not shutil.which("cargo", path=env["PATH"]):
        print("        Installing official signed Rust toolchain (GNU target, user-only)...")
        rustup = os.path.join(tempfile.gettempdir(), "rustup-init.exe")
        download("https://win.rustup.rs/x86_64", rustup)
        subprocess.run(
            [rustup, "-y", "--default-host", "x86_64-pc-windows-gnu", "--profile", "minimal", "--no-modify-path"],
            check=True,
        )
        os.remove(rustup)
        env["PATH"] = CARGO_BIN + os.pathsep + env.get("PATH", "")
        need_gnu = True
        ok(f"Installed Rust in {CARGO_BIN}")
    else:
        proc = subprocess.run(["rustc", "-vV"], env=env, text=True, capture_output=True)
        if proc.returncode != 0:
            print("        Rustup found without a default toolchain; installing stable-x86_64-pc-windows-gnu...")
            subprocess.run(["rustup", "toolchain", "install", "stable-x86_64-pc-windows-gnu", "--profile", "minimal"], env=env, check=True)
            subprocess.run(["rustup", "default", "stable-x86_64-pc-windows-gnu"], env=env, check=True)
            need_gnu = True
            ok("Configured default Rust GNU toolchain.")
        else:
            if "gnu" in proc.stdout.lower():
                need_gnu = True
            ok("Found existing Cargo.")

    if need_gnu:
        mingw_bin = os.path.join(MINGW_DIR, "bin")
        if os.path.isdir(mingw_bin):
            env["PATH"] = mingw_bin + os.pathsep + env["PATH"]
        if not shutil.which("gcc", path=env["PATH"]):
            print("        Downloading portable MinGW-w64 zip (no admin, ~1-2 min)...")
            mzip = os.path.join(tempfile.gettempdir(), "winlibs-mingw.zip")
            download(MINGW_ZIP, mzip)
            with zipfile.ZipFile(mzip, "r") as zf:
                zf.extractall(INSTALL_DIR)
            os.remove(mzip)
            env["PATH"] = mingw_bin + os.pathsep + env["PATH"]
            ok("Extracted portable GCC toolchain.")

    # 3. Build locally
    step(3, "Compiling local server binary (first build takes ~1-2 minutes)...")
    server_dir = os.path.join(APP_DIR, "server")
    subprocess.run(["cargo", "build", "--release"], cwd=server_dir, env=env, check=True)
    exe_path = os.path.join(server_dir, "target", "release", "schedule-planner-server.exe")
    if not os.path.isfile(exe_path):
        raise RuntimeError(f"Built binary not found at {exe_path}")
    ok(f"Built {exe_path} locally.")

    # 4. Create launcher & Desktop shortcut
    step(4, "Creating launcher and Desktop shortcut...")
    launcher = os.path.join(INSTALL_DIR, "Start-UDST-Planner.bat")
    with open(launcher, "w", encoding="ascii") as f:
        f.write(
            "@echo off\r\n"
            "title UDST Schedule Planner (http://localhost:8787)\r\n"
            "color 0B\r\n"
            "echo.\r\n"
            "echo  ============================================================\r\n"
            "echo    UDST Schedule ^& Degree Planner is running!\r\n"
            "echo.\r\n"
            "echo    Click this link (or open in your browser):\r\n"
            "echo      http://localhost:8787\r\n"
            "echo.\r\n"
            "echo    Keep this window open while using the planner.\r\n"
            "echo    Press Ctrl+C or close this window to stop the server.\r\n"
            "echo  ============================================================\r\n"
            "echo.\r\n"
            f'cd /d "{server_dir}"\r\n'
            'start "" "http://localhost:8787"\r\n'
            f'"{exe_path}"\r\n'
            "pause\r\n"
        )
    ok(f"Created launcher: {launcher}")

    # 5. Launch
    step(5, "Launching UDST Schedule Planner...")
    print("\n  ============================================================")
    print("    Installation complete!")
    print("    Open in your browser:  http://localhost:8787")
    print("  ============================================================\n")
    subprocess.Popen(["cmd.exe", "/k", launcher], creationflags=subprocess.CREATE_NEW_CONSOLE)


if __name__ == "__main__":
    main()
