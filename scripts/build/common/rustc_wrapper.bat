@echo off
REM scripts/build/common/rustc_wrapper.bat
REM
REM --- PURPOSE ---
REM This is a Windows batch shim for 'rustc_wrapper.ps1'.
REM Since Windows environment variables (RUSTC_WRAPPER) often prefer .bat/.exe
REM entry points, this file ensures that Cargo can correctly invoke our 
REM PowerShell-based interception logic.

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0rustc_wrapper.ps1" %*
