# roxy-can

基于 Rust + Dear ImGui 的 CAN 总线分析工具：多总线、每条总线各挂一个 DBC，覆盖经典 CAN、CAN FD、错误帧与远程帧，支持 DBC 解码，以及与 Vector 工具链兼容的 ASC / BLF 录制与回放。Windows x64 可执行文件见 [Releases](https://github.com/chemPolonium/roxy-can/releases)。

![roxy-can](screenshot.png)

## 功能

- **动态总线**：总线数量不固定，在 Buses 窗口增删、改名，为每条总线单独加载 DBC；删除总线时观测器、过滤器、生成器自动重映射
- **DBC 解码**：复用报文只解当前组（`M` / `mN` / `mNM` 嵌套、`SG_MUL_VAL_` 区间扩展复用）；`VAL_` 值表显示枚举文本；`SIG_VALTYPE_` 浮点按位模式解码；数值带类型标记（`[u16]` / `[f32]`）
- **帧模型**：经典 CAN、CAN FD（变长载荷至 64 字节、BRS / ESI）、错误帧、远程帧；Trace 中错误行铺红底、远程行铺淡紫底，Flags 列统一显示帧类型
- **信号观测器**：Trace / Messages / Statistics / Data / Graphics 五类窗口均可多开、各自独立过滤；Data / Graphics 可跨总线选择信号，Data 以范围条对照声明区间显示物理值与 Raw 值，Graphics 有 14 档时间窗、缩放平移、采样点圆点
- **总线负载统计**：Statistics 窗口顶部按总线给出线上一帧占时加权的负载与帧率（1 s 滚动窗）、60 s 负载曲线、错误帧计数；仲裁与 CAN FD 数据段比特率按总线设置，BRS 载荷按数据段速率计费
- **Interactive Generator（节点中心）**：DBC 报文按数据库声明的周期发送（`GenMsgCycleTime` 优先于 `CycleTime`，事件触发不上定时器），按信号拖拽编辑物理值或按 hex 编辑；每个信号可挂 Ramp / Sine / Step / Random / Triangle / Counter 激励随仿真时间连续变化；**每个节点的生成器在 Network 窗口的节点详情里**（条目 + Add + 响应规则 + 经硬件开关），总览窗口只留未分配条目与按 id 添加；**角色是总开关、条目开关是自定义**——切角色不改写逐条目开关，新建条目默认不发车，行内显示最近一次实际发射的载荷
- **Triggers 触发器**：信号越阈 / ID 出现 / 错误帧 / 周期超时四类条件，动作支持开始·停止录制（带预触发上下文与 post-roll）、单帧反应（触发帧信号自动镜像进目标载荷）、插入标记（Graphics 竖线）、清空 Trace；编辑器与持久化齐备
- **Network 视图**：每条总线一段拓扑，点击节点打开该节点的详情——角色三态声明（带说明文案）——**模拟**（允许按 DBC 声明的周期模拟整个 ECU）/ **监听**（只收不发）/ **离线**（不在仿真总线上，未声明即离线）；**该节点的生成器条目、响应规则与脚本也在这里**，一个节点的一切一处看完；顶部 Profile 行下拉应用 `profiles/*.toml` 角色覆盖与硬件映射（all-or-nothing）
- **节点角色模型**：DBC 是网络拓扑的唯一事实源，角色声明决定"这个节点现在谁扮演"——角色只是闸门，条目发不发还看生成器逐条开关；缺省即离线，restbus 无需配置；`profiles/<名>.toml`（角色覆盖 + `[[hw]]` 硬件映射）+ CLI `--profile` / Network 窗口 Profile 行让同一工程在 simulation / bench / CI 零修改切换，错配整份拒绝
- **Kvaser 硬件联动**：canlib32 运行时加载（驱动未装优雅降级）；Buses 窗口挂接通道（被占用自动降级只收并标注，CAN FD 数据段预设自动申请，不匹配降级经典并标注），适配器收到的帧（含 FD/BRS/ESI 徽标）照常进总线；逐节点"经硬件"开关把该节点的生成器条目、脚本 `send` 与触发反应帧同时发上真实总线——默认关，接真实硬件由你显式决定
- **回放块**：依附于 DBC 节点（Network 节点详情里创建与编辑）——把该节点录制日志按 id 过滤后注回仿真总线，按录制间距发车——restbus 的落地方案；仅仿真模式生效，绝不与回放模式二次投递
- **派生信号**：脚本 `emit_value("名", 表达式)` 一行发布——表达式即求值逻辑（`sig()` 读数、数学与波形内建自由组合），派生信号像数据库信号一样进 Graphics / Data / State Tracker
- **仿真节点**：类 CAPL 脚本语言（编译成字节码跑在自带 VM 上）驱动的自定义 ECU 节点——`on start` / `on message`（含 `*` 通配与错误帧事件）/ `on timer`（周期与一次性）事件驱动，读写 DBC 信号、收发帧、随机与波形内建；每回调 10 万指令预算，坏脚本卡不死总线；可**绑定**到 DBC 节点，绑定后发帧受该节点角色闸。语言参考见 `docs/script_language.md`，可运行示例见 `examples/`
- **Specification（规格监视）**：实测流量与数据库声明逐条对账，四类判定——Unknown（未知 ID）、Dlc（长度不符）、Cycle（周期漂移）、Missing（掉线）；容差与宽限可调并随工程保存
- **State Tracker（状态带观察器）**：订阅信号按状态分段绘制——VAL_ 值表标签、二进制方波、会话稳定配色，支持自定义阈值区间（名字 + 颜色）与颜色钉住；碎带按最短显示时长合并，区段表可导出 CSV
- **录制与回放**：读写 Vector ASC（经典 / FD / 错误 / 远程帧），读取 Vector BLF（raw 与 zlib 压缩容器）；大文件走 mmap 流式加载；播放器式走带控制——倍速增减、倍速直选、可拖动时间轴任意定位；录制支持 id 白名单过滤，Trace 容量可调（50k–2M 帧）且头部被裁时明确提示
- **工程文件（.rxproj）**：总线与 DBC、观测窗口及过滤、信号选择、生成器配置、窗口布局全部存一个 JSON；DBC 路径相对工程目录，工程文件夹可整体移动；30 秒自动保存，异常退出后恢复
- **多桌面**：多个桌面工作区，各自记住观测窗口与全局面板的开关和布局
- **中文字体**：内嵌 Inconsolata 并合并系统中文字体字形，支持中文输入法（IME）

## 快捷键

| 按键 | 功能 |
| --- | --- |
| F9 | 启动 / 停止测量 |
| Space | 播放 / 暂停 |
| - / + | 回放减速 / 加速一档 |
| Home | 图形窗口回到实时边缘 |
| Ctrl+R | 切换 ASC 录制 |
| Ctrl+E | 导出第一个 Trace 窗口为 ASC |
| Ctrl+O | 打开 DBC |
| Ctrl+N / Ctrl+Shift+O | 新建 / 打开工程 |
| Ctrl+S / Ctrl+Shift+S | 保存 / 另存工程 |

菜单栏 Help → Shortcuts 可查看全部快捷键。

## 构建与运行

需要 Rust 工具链（edition 2024）。

```sh
cargo run
```

运行测试：

```sh
cargo test
```

## 上手

1. 工具栏 **Simulation / Replay** 下拉选模式，点 **Play** 启动（仿真跑虚拟总线，回放已加载的 ASC / BLF）
2. **View → Buses**：为总线加载 DBC、加载日志；**View → Network**：把节点角色设为**模拟**（总开关），再在 **Interactive Generator** 里逐条打开要发的报文、调数值、挂激励——新建条目默认关，发不发由你逐条决定
3. **Measurement Setup**：新增各类观测窗口、选择信号范围、逐个导出；Data / Graphics 的信号在 Signal Selection 弹窗里跨总线勾选
4. 勾选 **Record** 录制 ASC
5. 总线挂了 DBC 之后，**View → Specification** 查看实测流量与数据库声明的对账结果

## 仿真节点

在 **Network** 视图的节点详情里新建脚本节点（"+ 脚本节点"，自动**绑定**到该 DBC 节点，总线跟随节点），创建即弹出该节点的**独立脚本编辑器**（源码、运行开关、节点日志；可保存/加载 `.rxcan`，可多开）；树形拓扑的节点下面挂的就是它的脚本叶，点击即开。一门 C 风格的小语言编译成字节码执行，事件驱动产生总线行为：

```c
// 收到请求 200ms 后应答（完整示例见 examples/）
on message 0x6A0 {
    set_timer("resp", 200);
}

on timer "resp" {
    send(0x6A1, frame_byte(0) + 0x40, 0x01);
}
```

- 内建覆盖：收发帧、DBC 信号读写（`sig` / `set_sig` / `get_sig`）、周期与一次性定时器、帧数据访问、随机数、五类波形（与 TX 发生器共用求值器）、数学与位运算
- 节点日志走 `print`，编辑器里直接看；编译/运行错误带行号列号，出错节点熔断待恢复
- 节点源码、绑定与启用状态随工程（`.rxproj`）保存
- 命令行可脱离界面运行与校验（CI 友好）：`--check-script` 编译节点脚本并打印静态响应映射与发送 / 接收集；`--project <p.rxproj> --duration <s> [--stats <csv>]` 无头仿真整个工程（生成器 + 脚本节点按保存的激活状态上线）；`--profile <名>` 叠加 `profiles/<名>.toml` 角色覆盖，同一工程在 simulation / bench / CI 间零修改切换，错配整份拒绝；`--convert <in> <out.asc>` 把 BLF/ASC 转存为 ASC，非零退出报告失败；`--kvaser-probe` 列出 canlib 可见的 Kvaser 通道

## 文档

- [使用说明](docs/usage.md)：命令行、仿真节点、触发器、State Tracker 的操作语义与口径说明
- [架构文档](docs/architecture.md)：核心线程模型、命令/快照边界、数据共享、帧键模型与开发约定
- [节点脚本语言参考](docs/script_language.md)：完整的语言参考手册
- [路线建议](docs/roadmap.md)：定位取舍、优先级排序与脚本语言决策（建议，未立项）

## 主要依赖

- [imgui-rs](https://github.com/imgui-rs/imgui-rs) + imgui-wgpu：界面与渲染
- [winit](https://github.com/rust-windowing/winit)：窗口与输入
- [can-dbc](https://github.com/marcelbuesing/can-dbc)：DBC 解析
- [rfd](https://github.com/PolyMeilex/rfd)：原生文件对话框
- [memmap2](https://github.com/Razaek/memmap2-rs)：ASC/BLF 大文件 mmap 流式读取
- [flate2](https://github.com/emoryns/rust-flate2)（rust_backend）：BLF zlib 压缩容器解压

## 许可

GPL-3.0，详见 [LICENSE](LICENSE)。
