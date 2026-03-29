@echo off
REM cortex-dashboard.bat — Launch the Streamlit dashboard at localhost:3333
REM Requires: Cortex daemon running on port 7437, streamlit installed

setlocal

REM Check daemon is running
curl -s http://127.0.0.1:7437/health >nul 2>&1
if %errorlevel% neq 0 (
    echo [dashboard] Daemon not running. Starting it first...
    call "%~dp0cortex-start.bat"
    timeout /t 2 >nul
)

REM Check streamlit
python -c "import streamlit" 2>nul
if %errorlevel% neq 0 (
    echo [dashboard] streamlit not installed. Installing...
    uv pip install streamlit httpx --system
)

cd /d "%~dp0"
echo [dashboard] Open http://localhost:3333
streamlit run workers\cortex_dash.py --server.port 3333
