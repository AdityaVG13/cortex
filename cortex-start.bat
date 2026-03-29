@echo off
REM cortex-start.bat — Single source of truth for daemon lifecycle
REM Run this BEFORE any AI session. Both Claude and Droid connect as clients.
REM Usage: cortex-start          (start daemon)
REM        cortex-start stop     (stop daemon)
REM        cortex-start status   (check if running)

setlocal enabledelayedexpansion

set CORTEX_DIR=%~dp0
set DAEMON_JS=%CORTEX_DIR%src\daemon.js
set PID_FILE=%USERPROFILE%\.cortex\cortex.pid
set LOG_OUT=%CORTEX_DIR%daemon.out.log
set LOG_ERR=%CORTEX_DIR%daemon.err.log

if "%1"=="stop" goto :stop
if "%1"=="status" goto :status

:start
REM Check if already running
curl -s http://127.0.0.1:7437/health >nul 2>&1
if %errorlevel%==0 (
    echo [cortex] Already running on port 7437
    curl -s http://127.0.0.1:7437/health
    echo.
    goto :eof
)

REM Kill stale process if PID file exists
if exist "%PID_FILE%" (
    set /p OLD_PID=<"%PID_FILE%"
    echo [cortex] Stale PID file found ^(PID !OLD_PID!^), cleaning up...
    taskkill /PID !OLD_PID! /F >nul 2>&1
    del "%PID_FILE%" >nul 2>&1
    ping -n 2 127.0.0.1 >nul
)

REM Start daemon detached via wmic (survives terminal close)
echo [cortex] Starting daemon...
start "Cortex Daemon" /MIN cmd /c "node "%DAEMON_JS%" serve >"%LOG_OUT%" 2>"%LOG_ERR%""

REM Wait for it to come up (max 5s)
for /L %%i in (1,1,10) do (
    ping -n 2 127.0.0.1 >nul
    curl -s http://127.0.0.1:7437/health >nul 2>&1
    if !errorlevel!==0 (
        echo [cortex] Daemon is LIVE on port 7437
        curl -s http://127.0.0.1:7437/health
        echo.
        goto :eof
    )
)

echo [cortex] ERROR: Daemon failed to start after 10s
echo [cortex] Check logs: %LOG_ERR%
goto :eof

:stop
if exist "%PID_FILE%" (
    set /p PID=<"%PID_FILE%"
    echo [cortex] Stopping daemon ^(PID !PID!^)...
    taskkill /PID !PID! /F >nul 2>&1
    del "%PID_FILE%" >nul 2>&1
    echo [cortex] Stopped.
) else (
    echo [cortex] No PID file found. Checking port...
    curl -s http://127.0.0.1:7437/health >nul 2>&1
    if !errorlevel!==0 (
        echo [cortex] Daemon is running but no PID file. Kill node.exe manually or use: netstat -ano ^| findstr 7437
    ) else (
        echo [cortex] Daemon is not running.
    )
)
goto :eof

:status
curl -s http://127.0.0.1:7437/health >nul 2>&1
if %errorlevel%==0 (
    echo [cortex] ONLINE
    curl -s http://127.0.0.1:7437/health
    echo.
) else (
    echo [cortex] OFFLINE — run cortex-start to launch
)
if exist "%PID_FILE%" (
    set /p PID=<"%PID_FILE%"
    echo [cortex] PID file: !PID!
)
goto :eof
