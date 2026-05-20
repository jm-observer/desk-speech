#!/usr/bin/env pwsh
# 一键发服务端到 GB10。开发期常用,不做 git 检查、不打 tag。
#
# 用法:
#   .\scripts\release-server.ps1                       # 默认 both:同步 + 重建 + 重启 + 冒烟生产栈
#   .\scripts\release-server.ps1 -Service asr          # 只动 asr
#   .\scripts\release-server.ps1 -Service orchestrator # 只动 orchestrator
#   .\scripts\release-server.ps1 -Service asr-server   # 只动外部 OpenAI 兼容 ASR(profile)
#   .\scripts\release-server.ps1 -NoBuild              # 同步后只 up -d(改 compose/env 用)
#   .\scripts\release-server.ps1 -SyncOnly             # 只推文件,不动容器
#
# 说明:
#   - 'both' 指生产栈 asr + orchestrator(给桌面客户端用的核心链路)。
#   - 'asr-server' 是 OpenAI 兼容外部 ASR,compose 里挂了 profile,默认不参与;
#     脚本检测到 -Service asr-server 时会自动追加 `--profile asr-server`。

param(
  [ValidateSet('asr','orchestrator','both','asr-server')]
  [string]$Service = 'both',
  [switch]$NoBuild,
  [switch]$SyncOnly
)

$ErrorActionPreference = 'Stop'
$RemoteHost = 'fengqi@192.168.0.68'
$RemoteDir  = '~/server'
$Repo       = Split-Path -Parent $PSScriptRoot

function Step($m) { Write-Host "→ $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "✓ $m" -ForegroundColor Green }

$items = @('compose.yaml')
if ($Service -in 'asr','both')          { $items += 'asr' }
if ($Service -in 'orchestrator','both') { $items += 'orchestrator' }
if ($Service -eq 'asr-server')          { $items += 'asr-server' }

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

# 计算 compose 子命令:asr-server 需要 --profile;其余裸跑(profile 服务自动跳过)。
$composeBase = if ($Service -eq 'asr-server') { 'docker compose --profile asr-server' } else { 'docker compose' }
$svcArg      = if ($Service -eq 'both') { '' } else { $Service }

if (-not $NoBuild) {
  Step "$composeBase build $svcArg(可能数分钟)"
  & ssh -o BatchMode=yes $RemoteHost "cd $RemoteDir && $composeBase build $svcArg"
  if ($LASTEXITCODE -ne 0) { throw "build 失败 — 详细日志看 GB10 控制台输出" }
}

Step "$composeBase up -d $svcArg"
& ssh -o BatchMode=yes $RemoteHost "cd $RemoteDir && $composeBase up -d $svcArg"
if ($LASTEXITCODE -ne 0) { throw "up 失败" }

Step "冒烟(等 3s 让容器起来)"
Start-Sleep -Seconds 3

if ($Service -eq 'asr-server') {
  $hz     = & ssh -o BatchMode=yes $RemoteHost "curl -s -m 5 http://localhost:8091/healthz"
  $models = & ssh -o BatchMode=yes $RemoteHost "curl -s -m 5 http://localhost:8091/v1/models"
  Write-Host "  /healthz    $hz"
  Write-Host "  /v1/models  $models"
  if ($hz -ne 'ok') { throw "asr-server 冒烟失败 — docker compose --profile asr-server logs --tail=80 asr-server" }
} else {
  $stats = & ssh -o BatchMode=yes $RemoteHost "curl -s -m 5 http://localhost:8090/api/stats"
  $cfg   = & ssh -o BatchMode=yes $RemoteHost "curl -s -m 5 http://localhost:8090/api/asr-config"
  Write-Host "  /api/stats      $stats"
  Write-Host "  /api/asr-config $cfg"
  if (-not $stats -or -not $cfg) { throw "冒烟失败 — 检查 docker compose logs --tail=80" }
}
Ok "发布完成"
