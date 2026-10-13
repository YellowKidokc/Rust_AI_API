@echo off
rem Starts the local gateway the HTML pages and other tools call (http://127.0.0.1:8790 by default).
rem Point the Atom Builder's Worker URL at it, or any OpenAI-style client at http://127.0.0.1:8790/v1
cd /d "%~dp0"
"%~dp0BACKSIDE\forge.exe" --home "%~dp0BACKSIDE" serve %*
pause
