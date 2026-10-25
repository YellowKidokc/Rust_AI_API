@echo off
setlocal
pushd "%~dp0"
cargo build --release
if errorlevel 1 (
  popd
  pause
  exit /b 1
)
popd
if not defined PIPELINE_WORKFLOWS_ROOT set "PIPELINE_WORKFLOWS_ROOT=%~dp0..\pipeline-workflows"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install\install.ps1" -Dest "%~dp0station\BACKSIDE" -Pipeline "%PIPELINE_WORKFLOWS_ROOT%\API" -Quiet -NoShortcuts -NoLaunch
if errorlevel 1 (
  echo Installation failed. Read the error above.
  pause
  exit /b 1
)
echo Installed. Open station\4 OPEN WORKSPACE.bat
pause
