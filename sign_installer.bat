@echo off
chcp 65001 >nul
setlocal EnableExtensions
cd /d "%~dp0"

rem ================================================================
rem  설치 파일·exe에 코드 서명을 붙입니다 (회사 인증서 사용).
rem  사용법:  sign_installer.bat  인증서.pfx  [서명할 파일 ...]
rem    - 파일을 생략하면 빌드 결과(WidgetCalendar.exe, 설치 파일)를 찾아 서명합니다.
rem    - 암호는 환경 변수 SIGN_PASS 가 있으면 그 값을, 없으면 입력을 받습니다.
rem  필요한 것: Windows SDK 의 signtool.exe (Visual Studio Build Tools 설치 시 함께 설치됨)
rem ================================================================

if "%~1"=="" goto :usage
set "PFX=%~1"
if not exist "%PFX%" goto :no_pfx
shift

rem ---------- signtool 찾기 ----------
set "SIGNTOOL="
for /f "delims=" %%i in ('dir /b /s "%ProgramFiles(x86)%\Windows Kits\10\bin\signtool.exe" 2^>nul ^| findstr /i "\\x64\\"') do set "SIGNTOOL=%%i"
if not defined SIGNTOOL where signtool >nul 2>&1 && set "SIGNTOOL=signtool"
if not defined SIGNTOOL goto :no_signtool

if not defined SIGN_PASS set /p "SIGN_PASS=인증서 암호: "
if not defined TIMESTAMP_URL set "TIMESTAMP_URL=http://timestamp.digicert.com"

rem ---------- 서명할 파일 목록 ----------
set "COUNT=0"
if not "%~1"=="" goto :given
for %%f in ("WidgetCalendar.exe" "src-tauri\target\release\bundle\nsis\*.exe") do if exist "%%~f" call :sign "%%~f"
goto :done

:given
if "%~1"=="" goto :done
call :sign "%~1"
shift
goto :given

:sign
echo.
echo [서명] %~1
"%SIGNTOOL%" sign /fd sha256 /f "%PFX%" /p "%SIGN_PASS%" /tr "%TIMESTAMP_URL%" /td sha256 /d "Widget Calendar" "%~1"
if errorlevel 1 goto :fail
"%SIGNTOOL%" verify /pa /q "%~1" >nul 2>&1
if errorlevel 1 echo   - 서명은 붙었지만 이 PC가 인증서를 신뢰하지 않습니다. 회사 인증서를 이 PC에 등록하면 해결됩니다.
if not errorlevel 1 echo   - 서명 확인 완료.
set /a COUNT+=1
exit /b 0

:done
echo.
if "%COUNT%"=="0" echo 서명할 파일을 찾지 못했습니다. 파일 경로를 직접 지정해 주세요.
if not "%COUNT%"=="0" echo 완료: %COUNT%개 파일에 서명했습니다.
goto :end

:usage
echo 사용법: sign_installer.bat 인증서.pfx [서명할 파일 ...]
goto :end
:no_pfx
echo [오류] 인증서 파일을 찾을 수 없습니다: %PFX%
goto :end
:no_signtool
echo [오류] signtool.exe 를 찾을 수 없습니다. Visual Studio Build Tools 또는 Windows SDK 를 설치해 주세요.
goto :end
:fail
echo [오류] 서명에 실패했습니다. 인증서 암호와 파일 경로를 확인해 주세요.
:end
echo.
pause
exit /b 0
