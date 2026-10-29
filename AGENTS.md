# 给 agent 的守则

## UI 只有 Web 一套

前端在 `web/`（Preact 10 + TypeScript + Vite），后端是 `src/web.rs` 的 axum
宿主；Rust 核心（`bus` / `app` / `observe`）不依赖任何 UI 库。不要在 Rust 里
再加桌面窗口或第二套界面。

- 固定外壳：Trace / Messages 共用主视图并可切换，顶栏 **Standard / Panel** 切换操作页；
  右侧 Rate/Signals/Monitor/Data/State、下栏 Statistics/Bus/Spec/Write；
  不要为单个面板引入窗口管理器。
- Panel 操作页是固定画布（默认 1280×720）、8px 网格，不做缩放或流式重排；
  控件坐标按格保存，拖动/缩放必须吸附网格。画幅尺寸也按格保存，缩小前
  必须保证所有控件仍在画布内。全屏用应用内固定层，不调用浏览器原生
  Fullscreen API，画布尺寸保持不变。控件类型先有 Button（**On click** 三选一：
  Write signals / Send message / 脚本动作；Write signals 只应用一组 Signal writes，
  逐项写入并汇总失败；Send message 按一次发一条 CAN/FlexRay/Ethernet 报文
  （CAN 可发 DBC 外 ID 并进硬件，FR/ETH 只进本会话；CAN 先选 Raw frame
  或目录报文，目录报文与 FR/ETH 已知帧可切 Raw / Physical，Physical 按目录
  信号物理值编码，未写信号取声明初始值）；
  仿真控制只在顶部工具栏，不再放进面板按钮；脚本动作可带一个数值参数）、Value（只读信号/派生/SysVar，可按升序上限分段染数字颜色）、
  Bar / Gauge（按 min/max 填充，复用同一套色段）、Lamp（源值达到阈值时点亮，
  颜色可选）、Toggle（写 SysVar 0/1）与 Setpoint（Target 选 SysVar 或信号：
  SysVar 按 min/max 夹取，CAN/FlexRay/Ethernet 信号按目录声明的范围夹取并回读总线实际值；
  CAN 写发送帧，FlexRay 与 Ethernet 只在本次测量内注入一帧（FR 端口仍只收、Ethernet 没有
  发送端口），派生信号拒绝写入；都可带值触发脚本动作，旧的 Numeric/Setpoint 页面在加载时迁移）。
- 新控件先进入 `panelPage.ts` 的控件注册表（创建/字段/最小尺寸/补丁），
  调色板（Edit 里的 **+ Add** 菜单）与属性栏由注册表生成，不要在 PagePanel
  里加类型分支；绑定解析与可用性判断放 `panelBinding.ts` 的纯选择器，并补
  单测。Edit 下属性栏常驻，未选中控件时显示 Panel/画幅属性，避免选区切换
  导致画布横移。Panel 是多页面 book：单页 workspace 要能迁移，页面名/画幅/
  控件按页保存。自绘菜单（Add / 建议下拉 / 品牌菜单）的行必须显式覆盖亮色
  主题的全局 `button` 底色与边框，否则每一行都会变成独立按钮；菜单用
  `role="menu"/"menuitem"`，支持 ↑/↓/Home/End/Esc（Esc 回到触发按钮）。
- Edit 下的右键菜单：控件 = Duplicate / Copy / Copy signal key / Delete；空白画布 =
  Paste / 添加七类控件 / Undo / Redo；页签 = Rename / Duplicate / Export / Delete。
  菜单同样用 `role="menu"/"menuitem"`、↑/↓/Home/End/Tab 导航，Esc 只关菜单、
  不退出 Edit；右键按下的主键判断要排除，不能触发拖动。
- 页面文件 `.rxpanel` 只在前端序列化（format + version + page，无控件 id），
  Export 用 Blob 下载，Import 用 `FilePick` 在浏览器读文本并作为新页面追加。
  导入必须严格校验：未知类型、越界、低于最小尺寸、重叠都报错且不改动现有
  页面，不要静默丢弃或夹取。
- Source/SysVar 这类带建议的输入用自绘 suggestion 下拉（fixed 锚定到输入框），
  不要用原生 `<datalist>`：浏览器弹层的字号/主题不受页面 CSS 控制。
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
