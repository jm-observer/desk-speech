# Plan 2: 核心功能组件 (Core Components)

## 前置依赖
Plan 1

## 本次目标
- 实现左侧控制面板 (Control Rail) 布局。
- 实现分段结果卡片 (Segment Card) 视觉效果。
- 实现音频播放/进度条 (Audio Player)。
- 实现实时波形 Canvas 组件 (Waveform)。

## 涉及文件
- `src/components/ControlPanel.tsx`
- `src/components/RecordCard.tsx`
- `src/components/SegmentCard.tsx`
- `src/components/AudioPlayer.tsx`
- `src/components/Waveform.tsx`

## 详细设计
- **RecordCard**: 包含渐变背景、计时器和开始/停止录音按钮。
- **SegmentCard**: 展示时间、原文、翻译、操作按钮（复制、导出）。
- **AudioPlayer**: 底部粘性条，包含播放控制和进度条。
- **Waveform**: 使用 Canvas 绘制 32 个柱状条，支持 idle 和 active 两种动画模式。

## 测试案例
- [ ] 验证控制面板的宽度是否固定为 320px。
- [ ] 验证分段卡片的 Hover 阴影效果。
- [ ] 验证波形 Canvas 是否正常绘制。
