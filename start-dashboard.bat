@echo off
REM Cortex Dashboard Launcher
REM Start the Streamlit dashboard at localhost:3333

echo Starting Cortex Dashboard...
echo Navigate to http://localhost:3333 in your browser
echo.

REM Check if streamlit is installed
python -c "import streamlit" 2>nul
if %errorlevel% neq 0 (
    echo ERROR: streamlit is not installed
    echo Installing with uv...
    uv pip install streamlit httpx
    if %errorlevel% neq 0 (
        echo ERROR: Failed to install dependencies
        pause
        exit /b 1
    )
)

REM Check if cortex_client.py exists
if not exist "workers\cortex_client.py" (
    echo ERROR: workers\cortex_client.py not found
    echo Make sure you're running from the cortex directory
    pause
    exit /b 1
)

echo [OK] Dependencies installed
echo [OK] Cortex client found
echo.

REM Start dashboard
cd /d "%~dp0"
echo Launching Streamlit...
streamlit run workers\cortex_dash.py --server.port 3333

pause
