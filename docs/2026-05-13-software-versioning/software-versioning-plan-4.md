# Plan 4: 测试、发布与验收

## 前置依赖
- Plan 1: 统一版本模型与单一真源
- Plan 2: 运行时版本暴露与前端接入
- Plan 3: 升级检测与兼容迁移

## 本次目标
- 为软件版本能力定义必要的测试覆盖、发布前检查和验收标准。
- 防止版本号漂移、运行时展示错误或升级标识失真进入发布流程。
- 将版本治理纳入现有修复流程和发布动作，而不是依赖人工记忆。

## 涉及文件
- `src-tauri/tests/`
- `src/src/`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `package.json`
- `.github/` 下发布相关工作流（如存在后续调整）
- `docs/2026-05-13-software-versioning/software-versioning.md`

## 详细设计

### 1. 后端测试建议
- 单元测试：
  - 校验 `AppVersionInfo.app_version` 非空。
  - 校验 `build_profile` 只可能为 `debug` 或 `release`。
  - 校验升级检测函数在“首次安装 / 同版本 / 升级”三条路径上输出正确。
- 数据访问测试：
  - 基于测试数据库验证 `app.last_run_version` 的读写逻辑。
- 命令层测试：
  - `get_app_version_info` 返回结构字段完整，序列化结果与 schema 对齐。

### 2. 前端测试建议
- API 类型测试：
  - `AppVersionInfo` 字段在调用处完整消费，不出现拼写漂移。
- 组件测试：
  - 设置页渲染版本号成功。
  - `first_run_after_upgrade = true` 时升级提示出现；为 `false` 时不出现。
- 容错测试：
  - `git_commit = null` 时 UI 不报错。

### 3. 发布前一致性校验
- 建议新增一个非阻塞、无新依赖的校验脚本或测试步骤，至少检查：
  - `src-tauri/Cargo.toml` version
  - `src-tauri/tauri.conf.json` version
  - `package.json` version
- 三者不一致时直接失败，阻止继续发版。
- 该校验可接入：
  - 本地发版前命令
  - CI 中 tag 构建前的预检查步骤

### 4. 与现有修复流程的关系
- 软件版本设计落地后，仍需通过项目既有循环：
  1. `cargo clippy --workspace -- -D warnings`
  2. `cargo fmt --check --all`
  3. `cargo test --workspace`
- 若新增前端测试或脚本检查，建议作为补充，而不是替代 Rust 侧修复流程。

### 5. 验收标准
- 用户可在 UI 中明确看到当前软件版本。
- 后端命令能返回统一版本结构，且字段含义稳定。
- 本地升级后，前端能在首次启动识别升级事件。
- 数据库/schema 版本与应用版本职责清晰，没有复用单个 `version` 字段承载多重含义。
- 发布前存在自动化检查，能发现 Cargo、Tauri、npm 三处版本漂移。

### 6. 人工验收清单
- 开发构建下打开设置页，确认展示 `vX.Y.Z`。
- 将本地 `app.last_run_version` 人工改为旧值后重启，确认出现一次升级提示。
- 再次重启，确认升级提示消失。
- 修改任一清单文件版本造成不一致，确认校验步骤失败。

## 测试案例
- 正常路径：
  - 三个版本源一致时，构建与 UI 展示均正常。
- 异常路径：
  - `tauri.conf.json` 与 Cargo 版本不一致时，校验脚本失败。
- 边界路径：
  - 从 `1.13.0` 升级到 `1.13.1` 这类补丁版本时，升级提示仍可识别，但不会触发额外 schema 迁移。
- 回归路径：
  - 现有 `InitStatus`、设置页、质量配置页接口不因加入版本能力而变更语义。
