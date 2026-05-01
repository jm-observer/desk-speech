# Plan 2: 录音内存窗口与后端状态改造

## 前置依赖
- Plan 1

## 本次目标
- 将内存音频保留窗口固定为最近 10 分钟。
- 保证分段时间轴与导出行为可预测，不因裁剪导致错位。

## 涉及文件
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/lib.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/audio_buffer.rs`

## 详细设计
- 新增常量：
  - `MAX_AUDIO_WINDOW_SECS: usize = 120`
  - `SAMPLE_RATE: usize = 16000`
  - `MAX_AUDIO_SAMPLES = MAX_AUDIO_WINDOW_SECS * SAMPLE_RATE`
- 新增结构 `RollingAudioBuffer`：
  - 内部 `VecDeque<f32>` 存样本。
  - 维护 `global_start_sample: u64`（当前窗口起始的全局样本号）。
  - `push_samples(&[f32])` 超限时从头弹出并推进 `global_start_sample`。
  - `snapshot_range(global_start, global_end)`：把全局区间映射到窗口内局部区间。
- 识别分段时间语义：
  - `segments` 中保存全局时间（自 session 起始），不受窗口裁剪影响。
  - UI 播放器默认仅回放 10 分钟窗口；跨窗口段落仅支持文本查看与数据库回溯。
- 导出策略：
  - `save_segment_as_wav`：优先从窗口取数据；若窗口缺失该段，走数据库/磁盘归档文件重建（Plan 3 提供）。
  - `save_all_audio`：默认导出最近 10 分钟，新增接口 `save_session_audio` 用于整会话导出（从归档文件）。
- 状态返回扩展：
  - `RecordingState` 增加 `audio_window_start_sec`、`audio_window_end_sec`。

## 测试案例
1. 正常路径：持续录制 11 分钟，内存样本总量稳定在 10 分钟上限附近。
2. 边界条件：恰好 600 秒时不裁剪，600+1 秒开始裁剪。
3. 异常路径：请求导出已被裁剪段，返回“窗口外数据”错误码。
4. 一致性：分段时间戳在裁剪前后保持全局单调递增。
