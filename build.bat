@echo off
chcp 65001 >nul
setlocal EnableExtensions
cd /d "%~dp0"
title Widget Calendar - 자동 빌드

echo ================================================================
echo   Widget Calendar 자동 빌드
echo   처음 한 번은 필요한 도구를 설치하느라 30분 이상 걸릴 수 있습니다.
echo   중간에 관리자 권한 확인 창이 뜨면 [예]를 눌러 주세요.
echo ================================================================
echo.

where winget >nul 2>&1
if errorlevel 1 goto :no_winget

set "PATH=%USERPROFILE%\.cargo\bin;%ProgramFiles%\nodejs;%PATH%"

rem ---------- 1. C++ 빌드 도구 ----------
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
call :check_vc
if defined HAS_VC goto :vc_ok
echo [1/4] Visual Studio Build Tools 설치 중... 용량이 커서 오래 걸립니다.
winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements --override "--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :check_vc
if not defined HAS_VC goto :fail_vc
:vc_ok
echo [1/4] C++ 빌드 도구 확인 완료.

rem ---------- 2. Rust ----------
where cargo >nul 2>&1
if not errorlevel 1 goto :rust_ok
echo [2/4] Rust 설치 중...
curl -L -o "%TEMP%\rustup-init.exe" https://win.rustup.rs/x86_64
if errorlevel 1 goto :fail
"%TEMP%\rustup-init.exe" -y --default-toolchain stable
if errorlevel 1 goto :fail
:rust_ok
where cargo >nul 2>&1
if errorlevel 1 goto :fail
echo [2/4] Rust 확인 완료.

rem ---------- 3. Node.js ----------
where node >nul 2>&1
if not errorlevel 1 goto :node_ok
echo [3/4] Node.js 설치 중...
winget install -e --id OpenJS.NodeJS.LTS --accept-package-agreements --accept-source-agreements
where node >nul 2>&1
if errorlevel 1 goto :fail
:node_ok
echo [3/4] Node.js 확인 완료.

rem ---------- 4. 컴파일 ----------
echo.
echo [4/4] 라이브러리 설치 및 컴파일 중... 처음에는 10~15분 걸립니다.
call npm install --no-audit --no-fund
if errorlevel 1 goto :fail
if /i "%~1"=="installer" goto :build_installer

call npm run build -- --no-bundle
if errorlevel 1 goto :fail

rem 이미 실행 중이면 exe를 덮어쓸 수 없으므로 먼저 종료한다.
taskkill /f /im WidgetCalendar.exe >nul 2>&1
timeout /t 1 /nobreak >nul
copy /y "src-tauri\target\release\widget-calendar.exe" "WidgetCalendar.exe" >nul
if errorlevel 1 goto :fail

echo.
echo ================================================================
echo   완료! WidgetCalendar.exe 가 만들어졌습니다. 지금 실행합니다.
echo   다음부터는 start.bat 또는 WidgetCalendar.exe 를 실행하면 됩니다.
echo ================================================================
start "" "WidgetCalendar.exe"
timeout /t 5 >nul
exit /b 0

:build_installer
call npm run build
if errorlevel 1 goto :fail
echo.
echo ================================================================
echo   설치 파일이 만들어졌습니다. 열리는 폴더의 Setup.exe 를 배포하세요.
echo ================================================================
start "" explorer "src-tauri\target\release\bundle\nsis"
timeout /t 5 >nul
exit /b 0

:check_vc
set "HAS_VC="
if not exist "%VSWHERE%" exit /b 0
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "HAS_VC=%%i"
exit /b 0

:no_winget
echo [오류] winget 을 찾을 수 없습니다.
echo Microsoft Store 에서 "앱 설치 관리자" 를 업데이트한 뒤 다시 실행해 주세요.
goto :end

:fail_vc
echo.
echo [오류] C++ 빌드 도구 설치가 끝나지 않았습니다.
echo 설치 후 재부팅이 필요할 수 있습니다. 재부팅한 뒤 build.bat 을 다시 실행해 주세요.
echo 그래도 안 되면 Visual Studio Installer 에서 "C++를 사용한 데스크톱 개발" 을 직접 선택해 설치해 주세요.
goto :end

:fail
echo.
echo [오류] 위 단계에서 실패했습니다. 위쪽의 오류 메시지를 확인해 주세요.
echo 방금 설치한 도구가 있다면 창을 닫고 build.bat 을 한 번 더 실행하면 이어서 진행됩니다.

:end
echo.
pause
exit /b 1
