@echo off
title build-vm

:: Navigate to the folder of this script.
cd %cd%

:: Navigate back to the "vm-rust" folder.
cd .\..\vm-rust\

:: Run the build command.
powershell wasm-pack build --target web

pause
exit