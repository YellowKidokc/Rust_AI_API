@echo off
rem One-time setup for the clipboard librarian. Edit the two folders below, then run this once.
rem   CLIPS   = where your clipboard app drops clips (one .txt or .md per clip; a NAS share is fine)
rem   LIBRARY = where the sorted library goes (Library\<category>\<title>.md, INDEX.md, _review\, _held\)
rem It registers two Windows scheduled tasks:
rem   forge clips sort    every hour, free route (paid rungs skipped)
rem   forge clips review  every 8 hours, DeepSeek; writes a report, moves nothing
rem Remove them later with:  schtasks /delete /tn "forge clips sort" /f   (and "forge clips review")
setlocal
set "CLIPS=\\NAS\clips\inbox"
set "LIBRARY=\\NAS\clips\Library"
set "FORGE=%~dp0BACKSIDE\forge.exe"
set "HOME_DIR=%~dp0BACKSIDE"

echo Clips:   %CLIPS%
echo Library: %LIBRARY%
echo Forge:   %FORGE%
echo.
echo Dry run on the first 3 clips (no model call):
"%FORGE%" --home "%HOME_DIR%" clips sort "%CLIPS%" --library "%LIBRARY%" --dry-run --limit 3
echo.
set /p "GO=Register the hourly sort and 8-hourly review? [y/N] "
if /i not "%GO%"=="y" goto :eof

schtasks /create /f /sc hourly /mo 1 /tn "forge clips sort" /tr "\"%FORGE%\" --home \"%HOME_DIR%\" clips sort \"%CLIPS%\" --library \"%LIBRARY%\" --max-tier free"
schtasks /create /f /sc hourly /mo 8 /tn "forge clips review" /tr "\"%FORGE%\" --home \"%HOME_DIR%\" clips review \"%LIBRARY%\" --route default"
echo.
echo Done. Open %LIBRARY%\INDEX.md any time; reports land in %LIBRARY%\_review\
pause
