# roxy-can

一个基于 Rust + Dear ImGui 的 CAN 总线分析工具，界面与交互参考 CANoe。总线数量不固定，可在 **Buses** 窗口中增删并自定义名称，每条总线各挂一个 DBC；Data/Graphics 窗口是纯信号观测器，可跨总线混合选择信号。虚拟模式下总线默认空闲，报文由内置信号生成器（Interactive Generator）产生，发送周期取自 DBC 声明，信号还可选斜坡/正弦/阶跃/随机激励随仿真时间连续变化，Network 窗口里勾选一个 ECU 节点即可按它声明的周期把自己放上总线；帧模型覆盖经典 CAN、**CAN FD**（变长载荷至 64 字节、BRS/ESI 标志）、以及错误帧 / 远程帧，支持 DBC 解码，以及与 Vector/CANoe 兼容的 ASC 录制与 ASC/BLF 回放（大文件走 mmap 流式加载）。

![roxy-can](screenshot.png)

## 功能

- **CAN FD / 错误帧 / 远程帧帧模型**：载荷变长至 64 字节，DLC 码与真实字节长度分离，FD 帧携带 BRS / ESI 标志；同时识别经典 CAN 的错误帧（`e` 类型）与远程帧（`r` 类型），并在 Trace 中给错误行铺红底、远程行铺淡紫底；Trace / Messages / Statistics 共用一个 `Flags` 列显示 `FD` / `FD·B` / `FD·E` / `FD·BE` / `ERR` / `RTR`；错误帧不进入 Messages / Statistics 聚合，远程帧的 payload 视为空；经典 CAN 数据帧行为完全向后兼容
- **动态总线**：Buses 窗口（View → Buses）中可添加/删除总线、自定义总线名、为每条总线单独加载 DBC；删除总线时所有观测器、过滤器、生成器自动重映射
- **Interactive Generator（信号生成器）**：所有总线的 DBC 报文即开即用，支持按周期发送、按信号拖拽编辑物理值（自动编码为数据字节）或按 hex 编辑；每条报文带 FD 勾选（DBC 报文 >8 字节时自动置为 FD），hex 编辑最多 64 字节；搜索框按名称/ID 过滤，每条总线可一键 All On / All Off
  - **发送周期来自 DBC**：新建条目取数据库声明的周期（`GenMsgCycleTime` 优先于 `CycleTime`，都没显式声明时取 `BA_DEF_DEF_` 默认值），数据库完全没这个属性才用 100 ms 兜底；声明为 0 视为事件触发，永不按定时器发送。默认工程的 CAN2 因此 `EngineData` 是 133 ms、`ABSdata` 是 50 ms，其余仍 100 ms（条目照旧全部默认关闭，不会有任何原本沉默的报文自己开始发）。已有条目的周期不会被改写——手调过的值说了算；正因如此，当某行与声明不一致时行上会出现 `DBC 133ms`（事件触发则是 `DBC event`）小按钮，点一下即还原并立刻按新表发送
  - **信号激励（value source）**：每个信号可选 **Ramp / Sine / Step / Random**，取值随仿真时间连续变化，起测量后在 Graphics 里直接看到曲线；要一个固定值就留 **Constant**（下拉第一项）并用滑块/hex 设 base 值。Constant 只是给"没有激励"起的名字——它不存任何源，值仍住在 base payload 里，所以工程文件里的形状编码一个字节都没挪动。信号行右侧下拉选形状，`...` 弹窗设参数（lo/hi、周期、相位；Random 为 redraw 间隔与 seed；Step 为逗号分隔序列），弹窗里改的一切都要 **Apply** 才生效。首次启用时按 DBC 量程快照 lo/hi，之后显式存储并随工程文件保存；选回 Constant 即把该信号交还 base 值。启用激励的报文标题会显示 `N driven`，被驱动的信号实时值前缀 `~`
  - **base 与激励分离**：hex 框与滑块编辑的是 base 报文，激励只在发送瞬间叠加、永不改写 base（`base` 标签提示该行有激励）。暂停时拖动某个信号的滑块＝"就地按停"，只清掉这一个信号的激励，其他信号仍在动；改 hex 不会销毁已配好的激励
  - **数值只在手势结束时生效**：信号滑块拖拽时句柄和读数跟着走，但报文要等**松手**（或回车）那一刻才被改写；hex 框也是**离开输入框时**才解码——否则重打 `11 22 33` 会先把长度 1 的残帧发出去。发送周期与激励参数两个弹窗都是"草稿 + **Apply**"，Cancel/Esc 整次放弃。原因很实在：imgui 的内联数字框每敲一个字符都回报一次变化，打了一半的数字（`100` 打到 `1`）本来会立刻上线，1 ms 周期就是这么来的
  - **仿真时基**：生成帧打在理论时隙上（不是 UI 心跳），所以 Statistics 的周期 Min/Avg/Max 恒等于设定周期，Trace 时间列与录制的 ASC 也落在仿真时基上；UI 卡顿会丢弃积压周期而不是补发一串；暂停会冻结仿真时钟，波形相位与发送计划停在原地、恢复后继续、不跳相位
- **Trace / Messages / Statistics 均可多开**：在 Measurement Setup 中用 +Trace / +Messages / +Statistics 新建，每个窗口有独立的过滤设置
- **Signals 选项**：每个观测器一个 Signals 下拉框，三档可选——所有总线、某一条总线、Manual（手动勾选，各窗口的勾选项互相独立）；选 "…" 打开 Message Selection 弹窗，按报文勾选，勾选即切换为 Manual，Clear 恢复为所有总线
- **Trace 视图**：逐帧滚动显示，含时间、总线、ID、报文名、数据、方向（Tx 高亮），支持 Signals 范围 + 文本/方向/DBC-only 过滤；点列头可按该列排序（第三次点击恢复默认新→旧）；右键行可快速过滤该 ID、清除过滤、加入生成器或复制整行/ID，可按当前过滤导出 ASC
- **Messages 聚合视图**：按（总线, ID）聚合，显示计数、实测周期、最新数据，展开可查看 DBC 解码后的信号值
- **Statistics 视图**：每报文计数、周期 Min/Avg/Max、长度（字节数）、总线占比
- **Data/Graphics 窗口即信号观测器**：不绑定具体总线，信号选择全部在 Measurement Setup 的 Filter 列完成；窗口本体只保留已选信号列表（可逐个开关显示/绘制、拖拽排序）；支持多窗口；Data 窗口值表含 Min/Avg/Max 统计列，可视化列在数值条与 Sparkline 之间点击切换；Graphics 窗口顶部一排时间窗口按钮（0.1s…1s/5s/10s/…/1m/2m/5m/10m/30m/1h 共 14 档）点选即换，Zoom 默认关闭，勾选后滚轮缩放/拖动平移，Live 回到实时边缘；**Dots** 勾选后在每个采样点上画圆点,完全手动控制、不受点密度影响（点很密时会连成串,很宽的窗口下也会增加绘制开销,不需要时关掉即可）；Graphics 显示的是**当前时间窗内**的采样，窗口自己向日志取数，所以拖动进度条、平移、缩放之后曲线立即是完整的，不必等回放走到那里（首次查看某段尚未解码的区域会有一次同步解码，可能卡顿一帧，之后该段已在缓存里）
- **Measurement Setup**：所有观测器（Trace/Messages/Statistics/Graphics/Data）一张表总览——顶部按钮新增任意观测器；每行一个方形 "->" 按钮，点击即打开并跳转到对应窗口（无关闭功能，关闭窗口用窗口自身的 X）；可重命名、逐个导出（Trace 按当前过滤器导出 ASC，其余导出 CSV），并可删除任意观测器；Trace/Messages/Statistics 行内选择 Signals 范围，Graphics/Data 行的 "…" 打开 Signal Selection 弹窗——报文 → 信号两级复选树（总线仅作分组标题，可跨总线任意勾选），报文级可整体勾选/取消，标签带（已选/总数）计数，支持搜索
- **Network 视图**：每条总线一段拓扑（DBC 节点框 + CAN 总线）；绿点表示实时活动，点击节点查看收发详情（详情在独立滚动面板中）；详情里勾选 **Simulate this node** 就把这个 ECU 放上总线——它负责的报文缺哪条补哪条、按 DBC 声明的周期开始发送（生成器里那些条目照常可逐条微调），节点框左缘出现琥珀竖条表示"这是我在仿真的 ECU"，与绿点的"我在总线上观察到它在发"各说各的、可同时成立；取消勾选只是停发，条目、payload 与激励全部保留，再勾回来就是原样
- **信号列表**：支持拖拽排序、全部显示；批量添加信号在 Signal Selection 弹窗中完成（报文级复选框整体勾选）
- **ASC / BLF 录制与回放（Vector/CANoe 兼容）**：读写标准 Vector ASC（`base hex` / `timestamps absolute`），经典数据帧走 `d` 型行、错误帧走 `e` 型行、远程帧走 `r` 型行，FD 帧按逐行 `CANFD` 记录，导出的 `.asc` 可被 CANoe / python-can 打开，CANoe 导出的经典、FD、错误/远程日志亦可导入；同时能直接读取 CANoe 导出的 `.blf`（文件头魔数是 `LOGG`，`BLF4` 只是格式名、并不出现在文件里；对象为 `LOBJ` 记录，raw 与 zlib 压缩容器均支持，CAN_MESSAGE / _2 / _FD / _FD_64 / _ERROR_EXT 全覆盖；各字段偏移以 Vector 官方库写出的样本文件为准，与 python-can 的期望值逐字段核对过），时间戳按首帧归零，与 ASC 的相对时间约定一致；≥100 MB 的 ASC 自动切至 mmap 流式读取，BLF 全程走 mmap，RSS 与日志大小解耦；录制文件名自动带日期时间戳；加载日志（Open Log...）与开始回放分开——加载只解析就绪，Play（Replay 模式）才开始播放；工具栏为播放器式走带控制：**<< / Play·Pause / >> / Stop**，`<<` `>>` 逐级放慢/加快，Stop 后的倍速下拉（0.5x / 1x / 2x / 4x）直接选择，回放中切换立即生效；走带行内嵌**可拖动时间轴**，拖到任意时刻即时定位，前向向后皆可，拖动过程中逐帧响应（稀疏索引随回放惰性生长，打开日志仍是 O(1)；代价是全新加载后第一次大跨度拖动要先扫过前缀，之后同一跳转即为瞬时）；日志播完后仍可拖回中途再按 Play 原地续播，`Stop` 则表示下次从头重放；回放时状态栏显示日志文件名 + 当前时间 / 总时长；日志路径留空可直接回放最近一次录制
- **拖放打开 / 最近文件**：把 `.dbc` / `.asc` / `.blf` 文件拖到窗口即可打开（DBC 装入第一条总线，日志切至 Replay 模式）；File 菜单提供 Recent DBC / Recent Logs 列表
- **状态栏**：当前工程名（有未保存修改时带 `*`）、测量状态、帧率 (f/s)、帧计数、录制指示；回放时显示日志文件名 + 当前时间 / 总时长（播完后不再消失，便于配合时间轴回拖）
- **多 Desktop**：类 CANoe 的多桌面——每个桌面各自记住五类观测窗口（Trace/Messages/Statistics/Graphics/Data）的开关与布局，以及全局面板（Generator/Network/Measurement Setup/Buses/ID Filter）的显示状态；底部桌面标签栏一键切换，`+` 新建空白桌面，右键标签可重命名、删除或调整顺序（Move to 选择目标位次，至少保留一个）；桌面列表与活动桌面随工程文件保存
- **工程文件（.rxproj）**：类 CANoe 的工程文件——总线数量/名称与各总线 DBC、每条总线勾选仿真的 DBC 节点、全部观测窗口及其过滤设置、已选信号、生成器配置、窗口布局/停靠全部打包在一个 JSON 工程文件里；勾选的节点只记意图，打开工程不会因此自己发起流量（实际发不发仍由每条报文的 On 决定），旧工程没有这一项则按全部未勾选加载；File 菜单提供 New / Open / Recent Projects / Save / Save As；DBC 路径相对工程目录存储，工程文件夹可整体移动；已保存工程退出时自动覆盖，未保存（Untitled）工程仅在有修改时于退出或切换时弹框询问（未改动则静默通过）；New Project 创建完全空的工程（无 DBC、无观测窗口、无生成器条目）；上次打开的工程自动恢复（`roxy-can.meta.json` 记录）；运行中每 30 秒写一份崩溃缓存（`roxy-can.autosave.rxproj`，不动工程文件本身），异常退出后下次启动自动恢复，正常退出时删除；旧版 `roxy-can.json` 在首次启动时自动迁移。窗口可停靠、可拖动；HiDPI 由平台层按 framebuffer scale 自动换算；字体沿用 roxy-dbc 的方案——内嵌 Inconsolata（13px、像素对齐）并合并系统中文字体字形，按基线自然对齐，支持中文输入法（IME）

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
| Ctrl+N | 新建工程 |
| Ctrl+Shift+O | 打开工程 |
| Ctrl+S | 保存工程 |
| Ctrl+Shift+S | 工程另存为 |

菜单栏 Help → Shortcuts 可随时查看全部快捷键，About 显示版本信息。

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

1. 用工具栏的 **Simulation / Replay** 下拉选择模式（切换时自动停止当前运行），点 **Play** 启动（同一按钮切换暂停/继续），`<<` `>>` 逐级调整回放倍速，**Stop** 停止：仿真模式跑虚拟总线，回放模式回放已加载的 ASC/BLF（未加载时弹出文件选择）；菜单栏 **File** 可打开 DBC/日志、导出与退出，**Measurement** 可启停/暂停，**View** 开关各面板（默认两条总线：CAN1 挂 `assets/sample.dbc`、CAN2 挂 `assets/motbus.dbc`）
2. 要造一条总线的流量，最省事的是 **View → Network**：选中一个 ECU 节点，勾选 **Simulate this node**，它负责的报文就按 DBC 声明的周期开始发（节点框左缘出现琥珀竖条）；取消勾选只是停发，配好的条目与激励都还在。要逐条精调就到 **Interactive Generator**：报文按总线区分，勾选 **On** 单独开关、点行上的周期（如 `133 ms`，事件触发显示 `event`）弹出小窗口改数值——里面可以直接输入也可以拖动预览，**Apply** 才生效、**Cancel**/Esc 放弃（回车等于 Apply；填 0 就是事件触发，永不按定时器发送），所以打字打到一半的数字绝不会先跑上总线；该行与数据库声明不一致时还会出现 `DBC …ms` 小按钮，点一下还原。展开后按信号调整数值或按 hex 编辑。要让某个信号自己动起来：在该信号右侧下拉里选形状（如 Sine），点 `...` 设 lo/hi 与周期，例如 `EngineSpeed` 选 Sine、lo 0 / hi 8000 / period 2000 ms，起测量后把它加进 Graphics 就是一条连续正弦；弹窗里改的 lo/hi/周期都要按 **Apply** 才写进这条激励。拖该信号的滑块会在**松手那一刻**把它"按停"在当前位置（拖的过程中只是预览，不会把半个数字编进报文），下拉改回 Constant 则把该信号完全交还 base 值
3. **View → Buses** 管理总线：改名、**Open...** 为单条总线加载 DBC、**+ Add bus** 新增、**x** 删除；**Open Log...** 只加载日志（ASC 或 BLF），回放由 **Play** 启动；`<<` `>>` 逐级变倍速，Stop 后的倍速下拉直接选择（0.5x/1x/2x/4x）；倍速下拉右侧的**时间轴可拖动定位**到任意时刻（暂停时拖完再 Play 从落点继续；日志播完后拖回去再按 Play 会原地续播，按 **Stop** 再 Play 才从头重放）；**回放进行中不能更换日志**——`Open Log...` 与 Recent Logs 会置灰，需先 **Stop**（Stop 后换日志会强制从头打开新文件，不会续播上一个）
4. 勾选 **Record** 录制 ASC；**Measurement Setup** 表里可总览所有观测器，点 "->" 打开并跳转到对应窗口，并在此新增/删除各类窗口、逐个导出
5. 每个观测器行内选择 **Signals** 范围（所有总线 / 单条总线 / Manual），Manual 时点 "…" 在 Message Selection 弹窗中勾选报文
6. Data/Graphics 的信号选择在 Measurement Setup 行内点 "…" 打开 Signal Selection 弹窗勾选（按总线分组，可跨总线选择）；窗口本体只显示已选信号列表，可逐个开关

## 主要依赖

- [imgui-rs](https://github.com/imgui-rs/imgui-rs) + imgui-wgpu：界面与渲染
- [winit](https://github.com/rust-windowing/winit)：窗口与输入
- [can-dbc](https://github.com/marcelbuesing/can-dbc)：DBC 解析
- [rfd](https://github.com/PolyMeilex/rfd)：原生文件对话框
- [memmap2](https://github.com/Razaek/memmap2-rs)：ASC/BLF 大文件 mmap 流式读取
- [flate2](https://github.com/emoryns/rust-flate2)（rust_backend）：BLF zlib 压缩容器解压

## 许可

GPL-3.0，详见 [LICENSE](LICENSE)。
