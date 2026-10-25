@echo off
setlocal
pushd "%~dp0"
cargo build --release -p forge
if errorlevel 1 goto failed
if not exist "station\BACKSIDE" mkdir "station\BACKSIDE"
copy /y "target\release\forge.exe" "station\BACKSIDE\forge.exe"
if errorlevel 1 goto failed
"station\BACKSIDE\forge.exe" init "%~dp0station\BACKSIDE"
if errorlevel 1 goto failed
echo Installed. Open station\4 OPEN WORKSPACE.bat
popd
pause
exit /b 0
:failed
echo Installation failed. Read the error above.
popd
pause
exit /b 1
