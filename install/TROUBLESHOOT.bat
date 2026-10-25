@echo off
rem Checks every piece of API Home and writes a report to LOGS\troubleshoot-<time>.txt (it opens when done).
rem TROUBLESHOOT.bat -Fix   also offers fixes (start forge, free a blocked port).
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0troubleshoot.ps1" %*
echo.
pause
