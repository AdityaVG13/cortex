@echo off
REM MCP wrapper -- copies cortex.exe to a runtime copy so the build target
REM is never locked by a running MCP session. Enables cargo build --release
REM without killing the MCP process.
setlocal
set SRC=%~dp0daemon-rs\target\release\cortex.exe
set DST=%~dp0daemon-rs\target\release\cortex-mcp.exe

REM Copy if source is newer or runtime copy doesn't exist
if not exist "%DST%" copy /y "%SRC%" "%DST%" >nul 2>&1
for %%A in ("%SRC%") do set SRC_DATE=%%~tA
for %%A in ("%DST%") do set DST_DATE=%%~tA
if "%SRC_DATE%" neq "%DST_DATE%" (
    copy /y "%SRC%" "%DST%" >nul 2>&1
)

"%DST%" mcp %*
