@echo off
chcp 65001 >nul
cd /d "%~dp0"
if exist "WidgetCalendar.exe" goto :run
echo WidgetCalendar.exe 가 아직 없어서 먼저 빌드합니다.
call "%~dp0build.bat"
exit /b
:run
start "" "WidgetCalendar.exe"
