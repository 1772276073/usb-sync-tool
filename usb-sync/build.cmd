@echo off
rem ============================================================
rem USB-Sync v2 一键构建
rem 自动接入：Rust GNU 工具链 + MSYS2 binutils（dlltool/as）
rem 用法：build.cmd            -> debug 构建
rem       build.cmd release    -> release 构建（交付产物）
rem       build.cmd test       -> 运行单元测试
rem ============================================================
set PATH=%USERPROFILE%\.cargo\bin;C:\msys64\mingw64\bin;%PATH%
cd /d "%~dp0"

if "%~1"=="release" (
    cargo build --release
    echo.
    echo 产物：E:\rust-target\usb-sync\release\usb-sync.exe 与 usb-sync-cli.exe
) else if "%~1"=="test" (
    cargo test
) else (
    cargo build
)
