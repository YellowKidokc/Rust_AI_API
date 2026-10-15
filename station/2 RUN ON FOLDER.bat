@echo off
rem Drag a folder (or a single .md) onto this file, or run it and paste a path.
setlocal
cd /d "%~dp0"
set "TARGET=%~1"
if "%TARGET%"=="" set /p "TARGET=Folder or file to run: "
set "TARGET=%TARGET:"=%"
set /p "JOB=Job template [default]: "
if "%JOB%"=="" set "JOB=default"
"%~dp0BACKSIDE\forge.exe" --home "%~dp0BACKSIDE" run "%TARGET%" --job %JOB% --dry-run
echo.
set /p "GO=That was a dry run (no API call). Send it for real? [y/N] "
if /i "%GO%"=="y" "%~dp0BACKSIDE\forge.exe" --home "%~dp0BACKSIDE" run "%TARGET%" --job %JOB%
echo.
pause
