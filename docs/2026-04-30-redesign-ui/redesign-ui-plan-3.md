# Plan 3: 状态管理与 API 绑定 (State Management & API Integration)

## 前置依赖
Plan 2

## 本次目标
- 建立全局状态管理（使用 Context 或 Zustand）。
- 绑定 Tauri 后端命令（ASR 控制、设置管理、数据库查询）。
- 实现录音流程的业务逻辑。

## 涉及文件
- `src/store/useAppStore.ts`
- `src/api/tauri-client.ts`
- `src/App.tsx` (整合逻辑)

## 详细设计
- **State**: 管理 `status` (`idle`, `recording`, etc.), `segments`, `settings`, `devices` 等。
- **Polling**: 实现对 `get_recording_state` 的轮询逻辑，实时更新分段结果。
- **Auto-copy**: 在分段完成时触发剪贴板写入逻辑。

## 测试案例
- [ ] 验证点击“开始录音”是否成功调用后端并切换状态。
- [ ] 验证轮询逻辑是否能实时捕获并展示新分段。
- [ ] 验证停止录音后是否正确加载回放音频。
