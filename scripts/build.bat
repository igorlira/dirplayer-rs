@echo off
title build

:: Navigate to the folder of this script.
cd %~dp0

:: Navigate back.
cd .\..\

:: Run the build command.
powershell npm run build

pause
exit