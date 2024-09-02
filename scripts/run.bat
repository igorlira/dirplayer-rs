@echo off
title run

:: Navigate to the folder of this script.
cd %~dp0

:: Navigate back.
cd .\..\

:: Run the start command.
powershell npm run start

pause
exit