# Plan 1: 设计系统与基础组件 (Design System & Primitives)

## 前置依赖
无

## 本次目标
- 建立符合设计的 CSS 变量系统 (Tokens)。
- 建立基础 UI 组件（按钮、开关、下拉框、图标库）。
- 配置本地字体资源。

## 涉及文件
- `src/index.css` (新设计 tokens)
- `src/components/ui/Button.tsx`
- `src/components/ui/Switch.tsx`
- `src/components/ui/Dropdown.tsx`
- `src/components/ui/Icon.tsx`
- `src/assets/fonts/` (本地字体)

## 详细设计
- **Tokens**: 直接从设计稿 README.md 复制颜色、阴影、圆角和字体变量。
- **Button**: 支持 `primary`, `outline`, `soft` 三种风格，以及不同的尺寸。
- **Icons**: 将设计稿中的内联 SVG 转换为 React 组件。
- **Typography**: 引入 Noto Sans SC 和 Geist 系列字体。

## 测试案例
- [ ] 验证所有 CSS 变量是否正确加载。
- [ ] 验证按钮在各种状态（Hover, Active, Disabled）下的视觉表现。
- [ ] 验证开关组件的滑动动画。
