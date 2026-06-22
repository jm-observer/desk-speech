#!/usr/bin/env pwsh
# 一键发服务端到 GB10。开发期常用,不做 git 检查、不打 tag。
#
# 用法:
#   .\scripts\release-server.ps1            # 同步 + 重建 + 重启 + 冒烟 asr
#   .\scripts\release-server.ps1 -NoBuild   # 同步后只 up -d(改 compose/env 用)
#   .\scripts\release-server.ps1 -SyncOnly  # 只推文件,不动容器
#
# 说明:
#   - **本脚本现在只管 asr**。orchestrator 已于 2026-06 迁出至 toolkit 仓
#     (crates/orchestrator),改为宿主 systemd 服务,用 toolkit 的
#     `deploy-g10.ps1 -Service orchestrator` 部署 —— 不再走本脚本。
#   - asr-server(OpenAI 兼容外部 ASR)亦已迁出至 toolkit(deploy/asr-tts);
#     详见 server/asr-server/MOVED.md。

param(
  [ValidateSet('asr')]
  [string]$Service = 'asr',
  [switch]$NoBuild,
  [switch]$SyncOnly
)

$ErrorActionPreference = 'Stop'
$RemoteHost = 'fengqi@192.168.0.68'
$RemoteDir  = '~/server'
$Repo       = Split-Path -Parent $PSScriptRoot

function Step($m) { Write-Host "→ $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "✓ $m" -ForegroundColor Green }

$items = @('compose.yaml', 'asr')

$tar    = Join-Path $env:TEMP "release-server-$([guid]::NewGuid().ToString('N')).tar"
$tarExe = Join-Path $env:WINDIR 'System32\tar.exe'   # bsdtar:认 Windows 路径(避开 Git Bash 的 GNU tar)
Step "打包 $($items -join ', ')(排除 target/__pycache__/.venv)"
Push-Location "$Repo/server"
try {
  & $tarExe -cf $tar --exclude='target' --exclude='__pycache__' --exclude='.venv' $items
  if ($LASTEXITCODE -ne 0) { throw "tar 失败" }
} finally { Pop-Location }

Step "scp → $RemoteHost"
& scp -q $tar "${RemoteHost}:/tmp/release-server.tar"
if ($LASTEXITCODE -ne 0) { throw "scp 失败" }

Step "解包到 $RemoteDir"
& ssh -o BatchMode=yes $RemoteHost "tar -xf /tmp/release-server.tar -C $RemoteDir && rm /tmp/release-server.tar"
if ($LASTEXITCODE -ne 0) { throw "解包失败" }

Remove-Item $tar -Force
Ok "同步完成"

if ($SyncOnly) { Ok "SyncOnly 模式,结束"; exit 0 }

$composeBase = 'docker compose'
$svcArg      = $Service   # 只剩 asr

if (-not $NoBuild) {
  Step "$composeBase build $svcArg(可能数分钟)"
  & ssh -o BatchMode=yes $RemoteHost "cd $RemoteDir && $composeBase build $svcArg"
  if ($LASTEXITCODE -ne 0) { throw "build 失败 — 详细日志看 GB10 控制台输出" }
}

Step "$composeBase up -d $svcArg"
& ssh -o BatchMode=yes $RemoteHost "cd $RemoteDir && $composeBase up -d $svcArg"
if ($LASTEXITCODE -ne 0) { throw "up 失败" }

# asr 服务 HTTP 在 :9101(/health /embed /transcribe),WS 在 :9100。
# (旧的 :8090/api/stats 是 orchestrator 的独立端口,2026-06 已迁出至
#  toolkit-server:8788 —— 那个冒烟恒为空、误报失败,故改打 asr 自己的 /health。)
# FunASR 启动要加载模型,/health 可达通常需十几秒 —— 轮询而非死等固定秒数。
Step "冒烟(轮询 :9101/health,最多 ~40s)"
$health = $null
foreach ($i in 1..20) {
  Start-Sleep -Seconds 2
  $health = & ssh -o BatchMode=yes $RemoteHost "curl -s -m 5 http://localhost:9101/health"
  if ($health) { break }
}
Write-Host "  :9101/health    $health"
if (-not $health) { throw "冒烟失败 — asr /health 无响应,检查 docker compose logs --tail=80 asr" }
Ok "发布完成"
