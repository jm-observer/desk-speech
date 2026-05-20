@echo off
REM 双击入口:发默认生产栈(asr + orchestrator)到 GB10。
REM 自带 -ExecutionPolicy Bypass,无需事先 Set-ExecutionPolicy。
REM 需要其他参数(-Service asr-server / -NoBuild 等)时,直接在 PowerShell 里跑 release-server.ps1。

pushd "%~dp0\.."
pwsh -NoProfile -ExecutionPolicy Bypass -File ".\scripts\release-server.ps1"
set RC=%ERRORLEVEL%
popd

echo.
if %RC% NEQ 0 (
  echo [release-server] 失败,退出码 %RC%
) else (
  echo [release-server] 成功
)
pause
exit /b %RC%
