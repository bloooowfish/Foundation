@echo off
setlocal
cd /d "%~dp0"
where pwsh >nul 2>nul
if %ERRORLEVEL% equ 0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-server.ps1" -Stop %*
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-server.ps1" -Stop %*
)
exit /b %ERRORLEVEL%
