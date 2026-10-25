@echo off
rem Starts API Home: the taskbar menu (it starts forge by itself), then opens the home page.
rem Works from the install folder, or from the repo after a build (target\release).
setlocal
set "TRAY=%~dp0forge-tray.exe"
if not exist "%TRAY%" set "TRAY=%~dp0..\target\release\forge-tray.exe"
if not exist "%TRAY%" (
  echo forge-tray.exe not found. Run INSTALL.bat first.
  pause
  exit /b 1
)
tasklist /FI "IMAGENAME eq forge-tray.exe" 2>nul | find /I "forge-tray.exe" >nul || start "" "%TRAY%"
if "%FORGE_URL%"=="" set "FORGE_URL=http://127.0.0.1:8790"
rem give the tray a moment to start forge
powershell -NoProfile -Command "foreach($i in 1..40){try{Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 '%FORGE_URL%/health' | Out-Null; exit 0}catch{Start-Sleep -Milliseconds 500}}; exit 1"
if errorlevel 1 (
  echo forge did not answer at %FORGE_URL%. Run TROUBLESHOOT.bat.
  pause
  exit /b 1
)
start "" "%FORGE_URL%/"
