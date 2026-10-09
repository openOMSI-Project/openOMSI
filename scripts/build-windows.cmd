@echo off
rem Build openOMSI for Windows into dist\windows (openomsi.exe is the game and, started with
rem no arguments, the launcher window): x64, or ARM64 with the target as the first argument
rem (build-windows.cmd aarch64-pc-windows-msvc). Needs Rust with the MSVC toolchain
rem (https://rustup.rs) and Visual Studio Build Tools with "Desktop development with C++"
rem (for ARM64 also its ARM64 build tools and LLVM's clang) and the Windows SDK.
rem To build the Windows version on a Mac, use scripts/build-windows-cross.sh instead.
setlocal
cd /d "%~dp0\.."
set "TARGET=%~1"
if "%TARGET%"=="" set "TARGET=x86_64-pc-windows-msvc"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if errorlevel 1 (
  echo Install Rust from https://rustup.rs using the MSVC toolchain, then run this script again.
  exit /b 1
)
cargo build --locked --release --target %TARGET% -p omsi-app -p omsi-launcher-core
if errorlevel 1 goto :failed
if not exist "dist\windows" mkdir "dist\windows"
copy /y "target\%TARGET%\release\openomsi.exe" "dist\windows\openomsi.exe" >nul || goto :failed
copy /y "target\%TARGET%\release\openomsi-launcher.exe" "dist\windows\openomsi-launcher.exe" >nul || goto :failed
rem (the release package also carries Google's ANGLE - libEGL.dll, libGLESv2.dll and
rem LICENSE-ANGLE.txt from openOMSI-org/angle-openomsi, see .github/workflows/release.yml -
rem for the "ANGLE (DirectX 11)" graphics API; startup::backend_order skips it without them)
rem (Steam's library, x64 only: an ARM64 build has no Steam, see crates\omsi-app\build.rs)
if /i "%TARGET%"=="x86_64-pc-windows-msvc" copy /y "assets\steam_redist\steam_api64.dll" "dist\windows\steam_api64.dll" >nul || goto :failed
echo.
echo Done. Run: "%CD%\dist\windows\openomsi.exe"
exit /b 0
:failed
echo.
echo Build failed. See the error above.
exit /b 1
