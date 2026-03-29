@echo off
REM cortex-app.bat — Launch the Cortex Control Center (Tauri desktop app)
REM Requires: Cortex daemon running on port 7437

setlocal

REM Check daemon is running
curl -s http://127.0.0.1:7437/health >nul 2>&1
if %errorlevel% neq 0 (
    echo [cortex-app] Daemon not running. Starting it first...
    call "%~dp0cortex-start.bat"
    timeout /t 2 >nul
)

set APP_EXE=%~dp0desktop\cortex-control-center\src-tauri\target\debug\cortex-control-center.exe

if not exist "%APP_EXE%" (
    echo [cortex-app] ERROR: Tauri app not built yet.
    echo [cortex-app] Build it first: cd desktop\cortex-control-center\src-tauri ^&^& cargo tauri dev
    pause
    exit /b 1
)

echo [cortex-app] Launching Cortex Control Center...
start "" "%APP_EXE%"
