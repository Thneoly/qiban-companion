@echo off
rem %* forwards -Configure/-Check so double-click users keep parity with npm run coordinator.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start.ps1" %*
if errorlevel 1 pause
