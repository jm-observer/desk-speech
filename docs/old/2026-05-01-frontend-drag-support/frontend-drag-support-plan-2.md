# Plan 2: CSS 样式微调

## 前置依赖
Plan 1

## 本次目标
1. 为可拖拽区域添加 `cursor: grab` 样式，提升用户感知。
2. 确保交互元素在拖拽区域上方时，鼠标样式正确切换。
3. 增加 `user-select: none` 到拖拽标题栏，防止拖拽时意外选中文字。

## 涉及文件
- `frontend-new/src/index.css`
- `frontend-new/src/App.tsx`

## 详细设计
- 在 `index.css` 中添加 `.drag-region` 类。
- 在 `App.tsx` 中应用这些类。

## 测试案例
1. 鼠标悬停在可拖拽区域时，显示为手型（grab）。
2. 鼠标悬停在按钮或输入框时，显示为指针（pointer）或文本选择。
3. 拖拽过程中不会选中 UI 上的文本。
