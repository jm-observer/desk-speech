#!/usr/bin/env bash
# 影子期观测数据的 TTL 清理。**装在 GB10 宿主机上跑，不要放进 asr 容器**。
#
# 为什么必须在宿主：影子期一结束，asr 容器很可能就不再启动了，容器内的定时任务
# 随之停摆，而 bind mount 里的**用户语音副本**还留在宿主机上。正常路径是流程走完
# 就把整个目录删掉；这个脚本只是兜底，防止进程崩溃 / 容器重建 / 忘记关开关。
#
# 装法（宿主机）：
#   crontab -e
#   17 4 * * * /home/fengqi/server/asr-ctx-ttl.sh >> /home/fengqi/server/asr-ctx-ttl.log 2>&1
#
# TTL 默认 30 天而不是 7 天：影子采集要数天，之后还有人工听审、改闸门、对全部记录
# 重跑重分类，可能多轮迭代，而旧语料正是重跑的依据。7 天会让最早那批 wav 在流程走完
# 之前先消失。30 天是「确定的最长期限」这一性质的下限，不是期望值。
set -euo pipefail

DIR="${ASR_CTX_OBS_HOST_DIR:-/home/fengqi/server/asr-ctx-shadow}"
TTL_DAYS="${ASR_CTX_SHADOW_TTL_DAYS:-30}"

[ -d "$DIR" ] || { echo "$(date -Is) 目录不存在，跳过：$DIR"; exit 0; }

before=$(du -sh "$DIR" 2>/dev/null | cut -f1 || echo '?')
n=$(find "$DIR" -type f -mtime "+${TTL_DAYS}" -print -delete | wc -l)
find "$DIR" -type d -empty -delete 2>/dev/null || true
after=$(du -sh "$DIR" 2>/dev/null | cut -f1 || echo '?')

echo "$(date -Is) TTL=${TTL_DAYS}d 删除 ${n} 个文件  ${before} -> ${after}"
# 清理失败要看得见，不能静默：set -e 会让上面任一步失败即非零退出，cron 会发信/留日志。
