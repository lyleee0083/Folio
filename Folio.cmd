@echo off
cd /d "%~dp0"
if not exist "%~dp0Folio.exe" (
  echo Folio runtime is missing.
  pause
  exit /b 1
)
start "" /d "%~dp0" "%~dp0Folio.exe" %*
