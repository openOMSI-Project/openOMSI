@echo off
rem Start the openOMSI dedicated server: start.cmd C:\path\to\OMSI2 [server.cfg]
rem (the OMSI 2 folder: a complete original installation, as the players have it; mods
rem go into this folder's Vehicles, maps, Sceneryobjects ... as in the game's content folder)
rem Exit code 75 = restart from the local dispatch page; any other code ends the loop.
setlocal
set "HERE=%~dp0"
if "%~1"=="" (
  echo usage: start.cmd C:\path\to\OMSI2 [server.cfg]
  exit /b 1
)
set "CFG=%~2"
if "%CFG%"=="" set "CFG=%HERE%server.cfg"
:loop
rem (the output stays in this window, as start.sh shows it)
"%HERE%openomsi.exe" --root "%~1" --server "%CFG%"
if errorlevel 75 if not errorlevel 76 (
  echo server: restarting...
  goto loop
)
exit /b %ERRORLEVEL%
