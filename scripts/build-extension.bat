@echo off
title build-extension

:: Navigate to the folder of this script.
cd %~dp0

:: Navigate back.
cd .\..\

:: Run the NodeJS command.
powershell npm run build-extension

:: Our "question" entry point.
:question
echo Would you like to compress the files into a ZIP file? (y/n)
set /p answer=
if /i "%answer%"=="y" goto yes
if /i "%answer%"=="n" goto no
echo Invalid input. Please enter "y" or "n".
goto question

:: Our "yes" entry point.
:yes

:: Navigate to the "dist-extension".
cd .\dist-extension\
:: Deletes "dcr*" folders with all its contents.
powershell -command "Get-ChildItem | Where-Object Name -Like 'dcr*' | ForEach-Object { Remove-Item -Recurse -LiteralPath $_.Name }"
:: Delete all unnecessary files.
del loader.html /Q
del index.html /Q
del favicon.ico /Q
del robots.txt /Q

:: Checks whether "dist-extension.zip" exists.
if exist .\..\dist-extension.zip (
    :: Delete the old ZIP file.
    del .\..\dist-extension.zip /Q
)

:: Create the new ZIP file.
powershell Compress-Archive -DestinationPath .\..\dist-extension.zip *
echo ZIP file "dist-extension.zip" was successfully created!

:: Our "no" entry point and the end of "yes".
:no
pause
exit