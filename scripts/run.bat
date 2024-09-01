@echo off
title run

:: Navigate to the folder of this script.
cd %cd%

:: Navigate back.
cd .\..\

:: Run the start command.
powershell npm run start

pause
exit