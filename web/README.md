# roxy-can Web 前端

Preact 10 + TypeScript + Vite，构建产物由 Rust 宿主嵌入。产品定位见 [`docs/product.md`](../docs/product.md)，操作语义见 [`docs/usage.md`](../docs/usage.md)。

## 运行与验证

```powershell
npm install
npm run dev        # 另开一个终端运行 roxy-can --web
npm run build
npm run typecheck
npx vitest run
```

构建后的 `dist/` 由 `src/web.rs` 提供；改前端后必须重新 `cargo build` 才会嵌入 exe。

## 结构与边界

- `app.tsx`：外壳、快照轮询、命令发送、跨会话布局。
- `dockLayout.ts`：四种布局、槽位面板分配、移动/交换、归一化与旧 workspace 迁移；纯函数并单测。
- `components/TraceTable.tsx`：手写固定行高虚拟化，不引入表格库。
- `components/SignalChart.tsx`：canvas 抽稀绘制，不引入图表库。
- `panelPage.ts`：Panel 控件注册表、页面序列化与校验。
- `panelBinding.ts`：纯选择器；控件可用性/禁用原因在这里，不在渲染层。
- `types.ts`：快照与 `Command` 类型，必须与 `src/web.rs` 同步。
- `SomeIpPanel.tsx` / `someipView.ts` / `sdView.ts`：SOME/IP Events、Services、Availability 的纯视图逻辑。

## 约定

- 快照只读；用户动作只产出 `Command`，由 `app.tsx` 的 `run()` POST。
- 新命令同时改 `WebCommand`、TypeScript `Command`、UI 调用点，并补 Rust 无头测试。
- Panel 新控件先加注册表，不在 `PagePanel` 里加类型分支。
- 文件交换只走 `/api/upload`、`/api/download`；前端不读任意主机路径。
- 主界面文案英文；长标签省略，控件文字随尺寸自适应。
- 视觉皮肤在 `minimal.css`：窄边距、直角、无装饰阴影；状态灯保留圆形。新增组件沿用这套 chrome，不要引入圆角卡片。
- 改 UI 必须在 1280×800 与 1024×768 下检查主要页面、四种布局和弹窗无横向溢出。
