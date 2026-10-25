@echo off
rem Installs forge + the taskbar menu (API Home) for this user into %LOCALAPPDATA%\Forge.
rem Drag your pipeline API folder onto this file to register it, or just double-click.
setlocal
if "%~1"=="" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
) else (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Pipeline "%~1"
)
echo.
pause
