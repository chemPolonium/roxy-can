# roxy-can

基于 Rust + Dear ImGui 的 CAN 总线分析工具：多总线、每条总线各挂一个 DBC，覆盖经典 CAN、CAN FD、错误帧与远程帧，支持 DBC 解码，以及与 Vector 工具链兼容的 ASC / BLF 录制与回放。Windows x64 可执行文件见 [Releases](https://github.com/chemPolonium/roxy-can/releases)。

![roxy-can](screenshot.png)

## 功能

- **动态总线**：总线数量不固定，在 Buses 窗口增删、改名，为每条总线单独加载 DBC；删除总线时观测器、过滤器、生成器自动重映射
- **DBC 解码**：复用报文只解当前组（`M` / `mN` / `mNM` 嵌套、`SG_MUL_VAL_` 区间扩展复用）；`VAL_` 值表显示枚举文本；`SIG_VALTYPE_` 浮点按位模式解码；数值带类型标记（`[u16]` / `[f32]`）
- **帧模型**：经典 CAN、CAN FD（变长载荷至 64 字节、BRS / ESI）、错误帧、远程帧；Trace 中错误行铺红底、远程行铺淡紫底，Flags 列统一显示帧类型
- **信号观测器**：Trace / Messages / Statistics / Data / Graphics 五类窗口均可多开、各自独立过滤；Data / Graphics 可跨总线选择信号，Data 含 Min / Avg / Max 统计与 Sparkline，Graphics 有 14 档时间窗、缩放平移、采样点圆点
- **Interactive Generator**：DBC 报文即开即用，按数据库声明的周期发送（`GenMsgCycleTime` 优先于 `CycleTime`，事件触发不上定时器），按信号拖拽编辑物理值或按 hex 编辑；每个信号可挂 Ramp / Sine / Step / Random 激励随仿真时间连续变化
- **Network 视图**：每条总线一段拓扑，点击节点查看收发详情；勾选 **Simulate this node** 即按 DBC 声明的周期模拟整个 ECU
- **Specification（规格监视）**：实测流量与数据库声明逐条对账，四类判定——Unknown（未知 ID）、Dlc（长度不符）、Cycle（周期漂移）、Missing（掉线）；容差与宽限可调并随工程保存
- **录制与回放**：读写 Vector ASC（经典 / FD / 错误 / 远程帧），读取 Vector BLF（raw 与 zlib 压缩容器）；大文件走 mmap 流式加载；播放器式走带控制——倍速增减、倍速直选、可拖动时间轴任意定位
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
2. **View → Buses**：为总线加载 DBC、加载日志；**View → Network**：勾选一个 ECU 节点开始模拟它；或在 **Interactive Generator** 里逐条开关报文、调数值、挂激励
3. **Measurement Setup**：新增各类观测窗口、选择信号范围、逐个导出；Data / Graphics 的信号在 Signal Selection 弹窗里跨总线勾选
4. 勾选 **Record** 录制 ASC
5. 总线挂了 DBC 之后，**View → Specification** 查看实测流量与数据库声明的对账结果

## 主要依赖

- [imgui-rs](https://github.com/imgui-rs/imgui-rs) + imgui-wgpu：界面与渲染
- [winit](https://github.com/rust-windowing/winit)：窗口与输入
- [can-dbc](https://github.com/marcelbuesing/can-dbc)：DBC 解析
- [rfd](https://github.com/PolyMeilex/rfd)：原生文件对话框
- [memmap2](https://github.com/Razaek/memmap2-rs)：ASC/BLF 大文件 mmap 流式读取
- [flate2](https://github.com/emoryns/rust-flate2)（rust_backend）：BLF zlib 压缩容器解压

## 许可

GPL-3.0，详见 [LICENSE](LICENSE)。
