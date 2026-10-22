# 给 agent 的守则

## UI 只有 Web 一套

前端在 `web/`（Preact 10 + TypeScript + Vite），后端是 `src/web.rs` 的 axum
宿主；Rust 核心（`bus` / `app` / `observe`）不依赖任何 UI 库。不要在 Rust 里
再加桌面窗口或第二套界面。

- 固定外壳：Trace 主表、右侧 Rate/Signals/Monitor/Data/State、下栏
  Messages/Statistics/Bus/Spec/Write；不要为单个面板引入窗口管理器。
- 大表手写固定行高虚拟化（`TraceTable.tsx`）；曲线用 canvas 手写抽稀，
  不要引入表格/图表组件库。
- 浏览器文件交换走 `/api/upload`、`/api/download` 与 per-user 数据目录；
  不要给页面加任意主机路径读取接口。

## 界面语言

- 窗口标题、标签页、按钮、表头、状态/错误行、输入提示等**主界面文案一律英文**；
  代码注释继续用中文。
- 悬停/弹窗注释以英文为主；遇到不容易一眼看懂的领域规则，可以在末尾加一行
  `中文：…` 的简短解释。不要把中文留在按钮、表头这类需要快速扫读的位置。
- 格式化占位符、单位与已有快捷键不要顺手改。

## 前端状态与命令：快照只读，写入集中

新面板和新控件按 `SignalPicker` / `BusesModal` 的形状写：

- 弹窗/面板自己的草稿、选中、搜索框放在组件本地 state；跨会话的工程状态
  放 `App` 或 core 快照，不要塞进前端全局单例。
- 后端数据只读：组件通过 props 拿 `state`/`catalog`，不要直接改写；
- 用户动作只产出 `Command`（定义在 `web/src/types.ts`），由 `app.tsx` 的
  `run()` POST 到 `/api/command`；默认值、合法性、总线写入都留在 Rust；
- 新增命令必须同时在 `WebCommand`、TypeScript `Command` 和 UI 调用点落地，
  并至少补一条 Rust 无头测试走真实 core；
- 表格/曲线更新要保持增量协议（`epoch/seq`、`signalsEpoch/since`），不要
  每次轮询重发全量。

## 验证

改 UI 要跑 `npm run build`、`cargo fmt --check`、`cargo clippy --all-targets`、
`cargo test --all-targets`，并在 1280×800 与 1024×768 下检查主要标签页和
弹窗没有横向溢出。验证用的服务、浏览器必须停掉，不要留后台进程。
