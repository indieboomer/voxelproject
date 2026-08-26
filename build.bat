@echo off
setlocal
cd /d "%~dp0"

if /i "%~1"=="release" (
    echo Building Voxel Project - release...
    cargo build --release
    set "EXE=target\release\voxelproject.exe"
) else (
    echo Building Voxel Project - debug...
    cargo build
    set "EXE=target\debug\voxelproject.exe"
)

if errorlevel 1 (
    echo.
    echo Build FAILED.
    pause
    exit /b 1
)

echo.
echo Build succeeded: %EXE%
pause
