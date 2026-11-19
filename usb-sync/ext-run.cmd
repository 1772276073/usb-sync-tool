@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0ext-test.ps1" > "E:\rust-target\usb-sync\ext-result.txt" 2>&1
