# Plan 1: 拖拽逻辑完善与优化

## 前置依赖
无

## 本次目标
1. 确保 `handleWindowDragStart` 在所有非交互区域生效。
2. 在详细模式下，增加整个侧边栏或顶部栏的拖拽支持。
3. 优化拖拽区域的检测逻辑，排除所有按钮和输入控件。

## 涉及文件
- `frontend-new/src/App.tsx`

## 详细设计
- 在 `App.tsx` 中，检查 `handleWindowDragStart` 是否绑定到了足够的容器上。
- 在简洁模式下，目前的容器已经绑定了 `onMouseDown={handleWindowDragStart}`，但需要确保内部的 `data-tauri-drag-region="false"` 块（如 WindowActions）正确阻断拖拽。
- 在详细模式下，目前只有顶部条绑定了 `handleWindowDragStart`，考虑在侧边栏也增加。
- 优化 `interactiveSelector`，增加常见的交互元素选择器。

## 测试案例
1. 简洁模式：点击中间区域、边缘区域可以拖动窗口。
2. 简洁模式：点击按钮、切换模式按钮无法拖动窗口。
3. 详细模式：点击顶部条可以拖动窗口。
4. 详细模式：点击侧边栏空白区域可以拖动窗口。
