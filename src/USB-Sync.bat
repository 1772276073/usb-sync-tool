@echo off
chcp 65001 >nul
cd /d "%~dp0"
powershell -ExecutionPolicy Bypass -WindowStyle Normal -File "sync-gui.ps1"
if %errorlevel% neq 0 (
    echo 启动失败，错误代码: %errorlevel%
    pause
)
