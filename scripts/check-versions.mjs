#!/usr/bin/env node
// 发布前版本一致性校验脚本
//
// 检查以下三个版本源是否一致：
//   - src-tauri/Cargo.toml  [package].version
//   - src-tauri/tauri.conf.json  .version
//   - package.json  .version
//
// 不一致时退出码为 1，一致时退出码为 0。
// 无新依赖，仅使用 Node.js 内置 fs 模块。

import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');

function parseTomlVersion(filePath) {
  const content = readFileSync(resolve(root, filePath), 'utf-8');
  const match = content.match(/^\[package\]\s*\n[\s\S]*?version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`无法从 ${filePath} 中提取 [package].version`);
  }
  return match[1];
}

function parseJsonVersion(filePath) {
  const content = readFileSync(resolve(root, filePath), 'utf-8');
  const data = JSON.parse(content);
  if (typeof data.version !== 'string') {
    throw new Error(`${filePath} 中缺少 string 类型的 version 字段`);
  }
  return data.version;
}

const sources = [
  { name: 'Cargo.toml', version: parseTomlVersion('src-tauri/Cargo.toml') },
  { name: 'tauri.conf.json', version: parseJsonVersion('src-tauri/tauri.conf.json') },
  { name: 'package.json', version: parseJsonVersion('package.json') },
];

const versions = new Set(sources.map((s) => s.version));

if (versions.size > 1) {
  console.error('版本不一致，禁止发版：');
  for (const source of sources) {
    console.error(`  ${source.name}: ${source.version}`);
  }
  process.exit(1);
}

console.log(`版本一致: ${sources[0].version}`);
process.exit(0);
