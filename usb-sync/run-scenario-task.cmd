@echo off
rem Run scenario verification via Task Scheduler (ASCII only for cmd.exe safety)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-scenario.ps1" -Exe "E:\rust-target\usb-sync\release\usb-sync-cli.exe" > "E:\rust-target\usb-sync\scenario-result.txt" 2>&1
