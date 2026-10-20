@echo off
setlocal
set "API_HOME=%~dp0"
rem %errorlevel% inside ( ... ) is read before the group runs, so jump out first and return Python's own status.
where py >nul 2>nul && goto run_py
where python >nul 2>nul && goto run_python
echo Python 3 was not found. Install it or add it to PATH.
exit /b 9009
:run_py
py -3 "%API_HOME%engine\menu.py" %*
exit /b %errorlevel%
:run_python
python "%API_HOME%engine\menu.py" %*
exit /b %errorlevel%
