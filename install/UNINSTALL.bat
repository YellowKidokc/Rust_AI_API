@echo off
rem Stops API Home and removes its programs and shortcuts. Your config, ledger (_data) and LOGS stay.
rem UNINSTALL.bat -All   removes the whole install folder.
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
echo.
pause
