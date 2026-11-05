# roxy-can Web 前端

Preact 10 + TypeScript + Vite，构建产物由 Rust 宿主嵌入。产品定位与操作语义不在这里重复，见 [`docs/product.md`](../docs/product.md) 和 [`docs/usage.md`](../docs/usage.md)。

## 运行与验证

```powershell
npm install
npm run dev        # 需要同时运行 dsh web / roxy-can --web
npm run build
npm run typecheck
npx vitest run
```

构建后的 `dist/` 由 `src/web.rs` 提供；改前端后必须重新 `cargo build` 才会嵌入 exe。

## 结构与边界

- `app.tsx`：外壳、快照轮询、命令发送、跨会话布局。
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
- 改 UI 必须在 1280×800 与 1024×768 下检查主要标签页和弹窗无横向溢出。

## 当前不在这里

Ethernet 实时抓包/发送、FlexRay 硬件发送、SOME/IP union/string 的 transformer 对齐，均由 Rust 核心和后续台架决定。
