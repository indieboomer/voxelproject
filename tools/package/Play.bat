@echo off
setlocal
cd /d "%~dp0"
voxelproject.exe %*
if errorlevel 1 (
    echo The game stopped with an error. See the message above.
    pause
)
