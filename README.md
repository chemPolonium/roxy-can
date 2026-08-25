# roxy-can

一个基于 Rust + Dear ImGui 的 CAN 总线分析工具，界面与交互参考 CANoe。总线数量不固定，可在 **Buses** 窗口中增删并自定义名称，每条总线各挂一个 DBC；Data/Graphics 窗口是纯信号观测器，可跨总线混合选择信号。虚拟模式下总线默认空闲，报文由内置信号生成器（Interactive Generator）产生；支持 DBC 解码、ASC 录制与回放。

## 功能

- **动态总线**：Buses 窗口（View → Buses）中可添加/删除总线、自定义总线名、为每条总线单独加载 DBC；删除总线时所有观测器、过滤器、生成器自动重映射
- **Interactive Generator（信号生成器）**：所有总线的 DBC 报文即开即用，支持按周期发送、按信号拖拽编辑物理值（自动编码为数据字节）或按 hex 编辑；搜索框按名称/ID 过滤，每条总线可一键 All On / All Off
- **Trace / Messages / Statistics 均可多开**：在 Measurement Setup 中用 +Trace / +Messages / +Statistics 新建，每个窗口有独立的过滤设置
- **Signals 选项**：每个观测器一个 Signals 下拉框，三档可选——所有总线、某一条总线、Manual（手动勾选，各窗口的勾选项互相独立）；选 "…" 打开 Message Selection 弹窗，按报文勾选（鼠标悬停浮窗显示该报文的信号），勾选即切换为 Manual，Clear 恢复为所有总线
- **Trace 视图**：逐帧滚动显示，含时间、总线、ID、报文名、数据、方向（Tx 高亮），支持 Signals 范围 + 文本/方向/DBC-only 过滤；点列头可按该列排序（第三次点击恢复默认新→旧）；右键行可快速过滤该 ID、清除过滤、加入生成器或复制整行/ID，可按当前过滤导出 ASC
- **Messages 聚合视图**：按（总线, ID）聚合，显示计数、实测周期、最新数据，展开可查看 DBC 解码后的信号值
- **Statistics 视图**：每报文计数、周期 Min/Avg/Max、DLC、总线占比
- **Data/Graphics 窗口即信号观测器**：不绑定具体总线，信号选择全部在 Measurement Setup 的 Filter 列完成；窗口本体只保留已选信号列表（可逐个开关显示/绘制、拖拽排序）；支持多窗口；Data 窗口值表含 Min/Avg/Max 统计列，可视化列在数值条与 Sparkline 之间点击切换
- **Measurement Setup**：所有观测器（Trace/Messages/Statistics/Graphics/Data）一张表总览——顶部按钮新增任意观测器；每行一个方形 "->" 按钮，点击即打开并跳转到对应窗口（无关闭功能，关闭窗口用窗口自身的 X）；可重命名、逐个导出（Trace 按当前过滤器导出 ASC，其余导出 CSV），并可删除任意观测器；Trace/Messages/Statistics 行内选择 Signals 范围，Graphics/Data 行的 "…" 打开 Signal Selection 弹窗——报文 → 信号两级复选树（总线仅作分组标题，可跨总线任意勾选），报文级可整体勾选/取消，标签带（已选/总数）计数，支持搜索
- **Network 视图**：每条总线一段拓扑（DBC 节点框 + CAN 总线）；绿点表示实时活动，点击节点查看收发详情（详情在独立滚动面板中）
- **信号列表**：支持拖拽排序、全部显示；批量添加信号在 Signal Selection 弹窗中完成（报文级复选框整体勾选）
- **ASC 录制 / 回放**：录制文件名自动带日期时间戳；加载 ASC（Open ASC...）与开始回放分开——加载只解析就绪，Play（Replay 模式）才开始播放；工具栏为播放器式走带控制：**<< / Play·Pause / >> / Stop**，`<<` `>>` 逐级放慢/加快，Stop 后的倍速下拉（0.5x / 1x / 2x / 4x）直接选择，回放中切换立即生效；回放时状态栏显示当前时间 / 总时长；ASC 路径留空可直接回放最近一次录制
- **拖放打开 / 最近文件**：把 `.dbc` / `.asc` 文件拖到窗口即可打开（DBC 装入工具栏当前选中的总线）；File 菜单提供 Recent DBC / Recent ASC 列表
- **状态栏**：测量状态、帧率 (f/s)、帧计数、录制指示；回放时显示当前时间 / 总时长
- **Docking 布局与工作区持久化**：窗口可停靠、可拖动；窗口位置/停靠布局自动保存到 `roxy-can.ini`，总线与 DBC 路径、各观测器窗口及其过滤设置、已选信号、生成器配置退出时自动保存到 `roxy-can.json`，下次启动恢复；HiDPI 由平台层按 framebuffer scale 自动换算；字体沿用 roxy-dbc 的方案——内嵌 Inconsolata（13px、像素对齐）并合并系统中文字体字形，按基线自然对齐，支持中文输入法（IME）

## 快捷键

| 按键 | 功能 |
| --- | --- |
| F9 | 启动 / 停止测量（按工具栏 Simulation/Replay 下拉选择的模式启动） |
| Space | 播放 / 暂停（未运行时启动，同 Play 按钮） |
| - / + | 回放减速 / 加速一档 |
| Home | 图形窗口回到实时边缘（复位平移） |
| Ctrl+R | 切换 ASC 录制 |
| Ctrl+E | 导出第一个 Trace 窗口为 ASC |
| Ctrl+O | 打开 DBC |

## 构建与运行

需要 Rust 工具链（edition 2024）。

```sh
cargo run
```

运行测试：

```sh
cargo test
```

## 使用

1. 用工具栏的 **Simulation / Replay** 下拉选择模式（切换时自动停止当前运行），点 **Play** 启动（同一按钮切换暂停/继续），`<<` `>>` 逐级调整回放倍速，**Stop** 停止：仿真模式跑虚拟总线，回放模式回放已加载的 ASC（未加载时弹出文件选择）；菜单栏 **File** 可打开 DBC/ASC、导出与退出，**Measurement** 可启停/暂停，**View** 开关各面板（默认两条总线：CAN1 挂 `assets/sample.dbc`、CAN2 挂 `assets/motbus.dbc`）
2. 在 **Interactive Generator** 中勾选报文 **On** 产生总线流量（可展开按信号调整数值），报文按总线区分
3. **View → Buses** 管理总线：改名、**Open...** 为单条总线加载 DBC、**+ Add bus** 新增、**x** 删除；工具栏的下拉框 + **Open DBC...** 也可加载；**Open ASC...** 只加载日志，回放由 **Play** 启动；`<<` `>>` 逐级变倍速，Stop 后的倍速下拉直接选择（0.5x/1x/2x/4x）
4. 勾选 **Record** 录制 ASC；**Measurement Setup** 表里可总览所有观测器，点 "->" 打开并跳转到对应窗口，并在此新增/删除各类窗口、逐个导出
5. 每个观测器行内选择 **Signals** 范围（所有总线 / 单条总线 / Manual），Manual 时点 "…" 在 Message Selection 弹窗中勾选报文（悬停可看信号）
6. Data/Graphics 的信号选择在 Measurement Setup 行内点 "…" 打开 Signal Selection 弹窗勾选（按总线分组，可跨总线选择）；窗口本体只显示已选信号列表，可逐个开关

## 主要依赖

- [imgui-rs](https://github.com/imgui-rs/imgui-rs) + imgui-wgpu：界面与渲染
- [winit](https://github.com/rust-windowing/winit)：窗口与输入
- [can-dbc](https://github.com/marcelbuesing/can-dbc)：DBC 解析
- [rfd](https://github.com/PolyMeilex/rfd)：原生文件对话框

## 许可

GPL-3.0，详见 [LICENSE](LICENSE)。
