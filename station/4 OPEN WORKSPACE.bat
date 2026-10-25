@echo off
setlocal
if not defined PIPELINE_WORKFLOWS_ROOT set "PIPELINE_WORKFLOWS_ROOT=%~dp0..\..\pipeline-workflows"
if not exist "%~dp0BACKSIDE\forge.exe" (
  echo Run INSTALL_FORGE.bat in the repo root first.
  pause
  exit /b 1
)
set "PIPELINE_SYSTEM=%PIPELINE_WORKFLOWS_ROOT%\API\_system"
echo Pipeline source: %PIPELINE_WORKFLOWS_ROOT%
echo Workspace: http://127.0.0.1:8790/
echo Keep this window open while using the workspace.
"%~dp0BACKSIDE\forge.exe" --home "%~dp0BACKSIDE" serve
pause
