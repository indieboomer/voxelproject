@echo off
setlocal
cd /d "%~dp0"

if /i "%~1"=="steam" (
    if /i "%~2"=="release" (
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\build_steam.ps1" -Release
    ) else (
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\build_steam.ps1"
    )
    if errorlevel 1 (
        echo Steam build FAILED.
        pause
        exit /b 1
    )
    pause
    exit /b 0
)

echo Direct-only build. For Steam support use: build.bat steam [release]

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
