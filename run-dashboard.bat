@echo off
REM Simple Cortex Dashboard Launcher

echo ============================================================================
echo   CORTEX DASHBOARD LAUNCHER
echo ============================================================================
echo.

REM Change to cortex directory
cd /d "%~dp0"
echo Working directory: %CD%
echo.

REM Check Python
echo Checking Python...
python --version
if %errorlevel% neq 0 (
    echo [ERROR] Python not found
    pause
    exit /b 1
)
echo [OK] Python found
echo.

REM Check streamlit
echo Checking streamlit...
python -c "import streamlit; print('Streamlit', streamlit.__version__)"
if %errorlevel% neq 0 (
    echo [ERROR] streamlit not installed
    echo Installing with uv: uv pip install streamlit httpx
    uv pip install streamlit httpx
    if %errorlevel% neq 0 (
        echo [ERROR] Failed to install
        pause
        exit /b 1
    )
)
echo [OK] streamlit installed
echo.

REM Check Cortex daemon
echo Checking Cortex daemon...
curl -s http://127.0.0.1:7437/health >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Cortex daemon not running on http://127.0.0.1:7437
    echo Start it first with: node src\daemon.js serve
    pause
    exit /b 1
)
echo [OK] Cortex daemon is running
echo.

REM Check cortex_client
echo Checking cortex_client...
if not exist "workers\cortex_client.py" (
    echo [ERROR] workers\cortex_client.py not found
    pause
    exit /b 1
)
echo [OK] cortex_client found
echo.

REM Launch dashboard
echo ============================================================================
echo   LAUNCHING DASHBOARD
echo ============================================================================
echo Open browser at: http://localhost:3333
echo Press Ctrl+C to stop
echo ============================================================================

echo.
streamlit run workers\cortex_dash.py --server.port 3333

echo.
echo Dashboard stopped.
pause
