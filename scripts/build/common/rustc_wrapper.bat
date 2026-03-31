@echo off
REM scripts/build/common/rustc_wrapper.bat
REM Windows batch wrapper for rustc (PowerShell script)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0rustc_wrapper.ps1" %*
