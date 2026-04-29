#!/usr/bin/env bash
set -e
URL="https://raw.githubusercontent.com/jm-observer/tool-template-rust/blob/main/AGENTS.md"
TARGET="$(dirname "$0")/../../AGENTS.md"
# Download to target location (parent directory)
curl -fsSL "$URL" -o "$TARGET"
if [ $? -ne 0 ]; then
  echo "Download failed"
  exit 1
fi
echo "AGENTS.md updated"