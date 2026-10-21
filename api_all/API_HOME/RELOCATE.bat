@echo off
setlocal
set "API_HOME=%~dp0"
rem %errorlevel% inside ( ... ) is read before the group runs, so jump out first and return Python's own status.
where py >nul 2>nul && goto run_py
python "%API_HOME%engine\relocate.py" %*
exit /b %errorlevel%
:run_py
py -3 "%API_HOME%engine\relocate.py" %*
exit /b %errorlevel%
