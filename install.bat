@echo off
setlocal EnableExtensions
title UDST Schedule Planner -- Windows Setup
color 0B
echo.
echo  ============================================================
echo       UDST Schedule ^& Degree Planner  --  Windows Setup
echo       Local-only ^| No Admin Required ^| Builds Locally
echo  ============================================================
echo.
echo  Delegating to the PowerShell installer (ExecutionPolicy Bypass, user-only)...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/t2-titto/udst-schedule-planner/main/install.ps1 | iex"
if errorlevel 1 (
    echo.
    echo  [ERROR] Installation did not finish. See the message above.
    pause
    exit /b 1
)
endlocal
