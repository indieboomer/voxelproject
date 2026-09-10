@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\package_game.ps1" %*
set "RESULT=%ERRORLEVEL%"
if not "%RESULT%"=="0" echo Package creation FAILED. See the error above.
if "%~1"=="" pause
exit /b %RESULT%
