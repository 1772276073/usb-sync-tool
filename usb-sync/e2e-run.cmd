@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0e2e-ret.ps1" > "E:\rust-target\usb-sync\e2e-ret.txt" 2>&1
