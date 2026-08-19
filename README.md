# roxy-can

一个基于 Rust + Dear ImGui 的 CAN 总线分析工具，界面与交互参考 CANoe。内置虚拟仿真数据源，开箱即可演示；支持 DBC 解码、ASC 录制与回放。

## 功能

- **虚拟数据源**：按 DBC 定义的周期自动生成报文，无需硬件即可运行
- **Trace 视图**：逐帧滚动显示，含时间、通道、ID、报文名、数据、方向，支持暂停与过滤
- **Messages 聚合视图**：按报文 ID 聚合，显示计数、实测周期、最新数据，展开可查看 DBC 解码后的信号值
- **DBC 支持**：加载 `.dbc` 文件，Signals 树展示所有报文与信号，可订阅绘制
- **Graphics 窗口**：信号曲线实时绘制，带时间/数值坐标轴，可调整时间窗与 Y 轴范围，支持多窗口
- **Data 窗口**：信号数值列表实时刷新，支持多窗口
- **信号列表**：支持拖拽排序、全选、批量显示
- **ASC 录制 / 回放**：录制文件名自动带日期时间戳；ASC 路径留空可直接回放最近一次录制
- **Docking 布局**：窗口可停靠、可拖动，支持多显示器缩放与中文输入法（IME）

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

1. 点击工具栏 **Start** 启动虚拟测量（默认加载 `assets/sample.dbc`）
2. 通过 **Open DBC...** / **Open ASC...** 选择自己的文件
3. 勾选 **Record** 录制 ASC；View 菜单中可新建/切换 Data、Graphics 窗口
4. 在 Symbols 树中订阅信号，即可在 Graphics/Data 窗口中查看

## 主要依赖

- [imgui-rs](https://github.com/imgui-rs/imgui-rs) + imgui-wgpu：界面与渲染
- [winit](https://github.com/rust-windowing/winit)：窗口与输入
- [can-dbc](https://github.com/marcelbuesing/can-dbc)：DBC 解析
- [rfd](https://github.com/PolyMeilex/rfd)：原生文件对话框

## 许可

GPL-3.0，详见 [LICENSE](LICENSE)。
