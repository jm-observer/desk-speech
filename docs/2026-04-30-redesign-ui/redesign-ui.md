# UI Redesign: StreamSpeech Premium Interface

- 时间：2026-04-30
- 项目现状：当前前端使用原生 HTML + JS，界面较为简陋（基于 Sherpa-ONNX 示例）。
- 整体目标：基于新设计的“StreamSpeech”高保真设计方案，重新开发前端界面。采用 React + Vanilla CSS (CSS Modules/Variables)，提升视觉体验、交互流畅度及桌面端原生感。

## Plan 拆分

| Plan | 描述 | 依赖 | 状态 |
|---|---|---|---|
| Plan 1 | 设计系统与基础组件 (Design System & Primitives) | 无 | 待开始 |
| Plan 2 | 核心功能组件 (Core Components - Control Rail & Segment Card) | Plan 1 | 待开始 |
| Plan 3 | 状态管理与 Tauri 命令绑定 (State Management & API Integration) | Plan 2 | 待开始 |
| Plan 4 | 高级功能与弹窗 (Settings & Rules Modals) | Plan 3 | 待开始 |
| Plan 5 | 浮动简洁模式 (Simple Mode / Mini Widget) | Plan 4 | 待开始 |
| Plan 6 | 抛光与性能优化 (Polishing & UX refinement) | Plan 5 | 待开始 |

## 风险与待定项
- **音频波形实时渲染**：需要确保 Canvas 渲染在录音期间的高效性。
- **窗口大小调整**：简洁模式需要 Tauri 后端支持窗口尺寸动态调整。
- **环境迁移**：将纯 HTML 环境迁移到 Vite + React 结构。
