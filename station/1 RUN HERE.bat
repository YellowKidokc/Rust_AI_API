@echo off
rem Station launcher: runs the default job over this station's INBOX into OUTBOX.
rem Layout:  <station>\1 RUN HERE.bat   <station>\BACKSIDE\forge.exe   <station>\BACKSIDE\config\   <station>\INBOX\   <station>\OUTBOX\
rem Ctrl+C is ignored during a run; type  stop  and Enter to finish in-flight documents and exit.
setlocal
cd /d "%~dp0"
set "JOB=%~1"
if "%JOB%"=="" set "JOB=default"
"%~dp0BACKSIDE\forge.exe" --home "%~dp0BACKSIDE" run "%~dp0INBOX" --job %JOB% --out "%~dp0OUTBOX"
echo.
pause
