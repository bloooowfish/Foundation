@echo off
setlocal
cd /d "%~dp0"
where pwsh >nul 2>nul
if %ERRORLEVEL% equ 0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-server.ps1" %*
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-server.ps1" %*
)
set "EXIT_CODE=%ERRORLEVEL%"
if %EXIT_CODE% neq 0 (
    echo.
    pause
)
exit /b %EXIT_CODE%
