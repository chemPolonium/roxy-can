# 使用说明

各面板能做什么、数字按什么口径算、出错时说什么。界面是固定的 Web 外壳：
Standard 工作区可选 Focus / Split / Main + side / Quad 四种布局，每个槽位用页眉菜单
选面板；顶栏 Standard / Panel 切换操作页。文中“窗口”指对应的 core 观测模型与页面。
总览与快速上手见 `README.md`，脚本语法见 `docs/script_language.md`。

## 布局与槽位

- **四种布局**：Focus（1 页）、Split（左右 2 页）、Main + side（主页 + 右侧上下 2 页）、
  Quad（2×2 4 页）。顶栏 **▦ Layout** 菜单切换；Alt+1…4 同效。工程文件记住每种布局
  各自的页面分配与分隔比例，切换布局不会覆盖另一套配置。
- **每个页一个面板**：页眉左侧是面板菜单，按 Stream / Live / Analysis 分组；选中面板
  即放到这一页。面板只有一个实例：如果它已经在别的槽位，两个槽位交换；菜单右侧标出
  它当前在哪个槽位。
- **Move to**：页面菜单里的 Move to 把当前页整体移到另一个槽位，同样与目标页交换，
  不会出现重复页或丢页。
- **最大化**：页眉最右的图标把当前页盖住应用外壳；Esc 或再点一次恢复。Signals 最大化
  时读数表在左侧，其他页按原样铺满。
- **保持固定**：布局只是四种预设，不提供自由拖拽、浮动窗、关闭页或任意嵌套分割；
  未显示的面板随时可以从任意页的页眉菜单选出来。
- **窄槽位**：1024 宽度下页眉会换行；Trace 表在很窄的槽位里可能出现横向滚动，需要整表
  宽度时用 Focus/Split 或拖宽分隔条。

## 运行模式与测量状态

- **Run mode**（顶栏，Start 左侧）：**Simulation** 运行生成器并分析总线/硬件流量；**Replay** 播放 Setup → Replay 选中的录制。它决定“这一趟数据从哪来”。
- **测量状态**（状态栏左侧）：**STOPPED / MEASURING / PAUSED**。Start/Stop/Pause 只改变测量状态，不改变 Run mode。
- 状态栏第二段写 `Simulation · <project/log>` 或 `Replay · <log>`，把运行模式和当前数据来源放在一起，避免把两者混成一个“模式”。

## 命令行（无头模式）

```
roxy-can                                  打开 Web UI（默认，127.0.0.1:2010）
roxy-can --replay <log>                   回放日志，不开窗口
roxy-can --project <p.rxproj>             无头仿真整个工程
roxy-can --check-script <f.rxcan>         校验节点脚本（flag 可重复）
roxy-can --convert <in> <out.asc>         BLF/ASC 转存为 ASC
roxy-can --kvaser-probe                   列出 canlib 可见的 Kvaser 通道
roxy-can --vector-probe                   列出 Vector 通道与总线类型；有 FlexRay 口时试开并抽取 2 秒
roxy-can --vector-probe --fibex <f>       先校验集群描述，再按它配置试开
roxy-can --vector-tx-probe <通道>         在该 CAN 口发几帧，打印驱动回的每个事件
roxy-can --project <p.rxproj> --duration <s>
           --test --test-json <report.json> 无头跑 test() 断言并输出 JSON
roxy-can --export-csv <log> --out <csv>   无窗口导出信号 CSV（--dbc / --fibex 可重复）
roxy-can -h                               完整帮助
```

- `**--replay**`：`--speed <n>` 倍速（默认 1.0）、`--duration <s>` 到时即停、`--stats <csv>` 结束后导出消息统计。日志自带的 id 会让同名发生器条目不发送，不会让两个来源混在一起。
- `**--project**`：按工程里保存的条目开关与角色启动，必须配 `--duration`（仿真没有天然终点）。
- `**--profile <名>**`：叠加 `profiles/<名>.toml` 的角色覆盖与硬件映射，需配合 `--project`。全量生效——档案里没列的总线会被断开；写了不存在的总线 / 节点 / 角色词则**整份拒绝**，中止在任何发送之前。硬件打不开只进状态行，不中止（角色覆盖独立成立）。
  ```toml
  # profiles/bench.toml —— 被测件是真硬件，其余全模拟
  description = "bench rig"

  [[node]]
  bus = "Powertrain"
  node = "EngineECU"
  role = "Simulated"

  [[hw]]              # 可选：硬件映射（全量声明）
  bus = "Powertrain"
  driver = "Vector"   # 或 Kvaser，缺省 Kvaser
  channel = 0
  ```
- `**--check-script**`：编译脚本，全部通过退出码 0，任一失败非 0 并列出 `line N:M`。通过时顺带打印静态收发集合与响应映射。配 `--dbc`（可重复）与 `--node` 时，GUI 启动节点那套装配检查照跑（信号引用、发送集对照、漏实现、`set_sig` 范围）。
- 回放模式不能录制（录回放只是复刻日志）。
- release 版是控制台程序：双击会保留一个终端窗口，服务地址、启动失败原因都打印在那里。

## 仿真节点（脚本编辑器）

- 在 **Setup → Scripts…** 里 **+ New script** 新建，再在 **Bind to** 里选 DBC 节点（总线跟随节点，也可保持 unbound）。编辑器在同一个窗口里打开。
- **角色是开关**：所属节点不是"模拟"时脚本一律不发送（`on start`、定时器、事件发帧全部被禁止）。未绑定脚本保持未绑定，不会被自动归到某个 DBC 节点。
- **Apply** 下发源码（测量中即时重编译重启该节点）；**运行**开关、保存/加载 `.rxcan`、节点日志都在编辑器里。
- 编辑器三栏：左栏**大纲**（每个处理器一行，点击跳转）与**收发**（静态事实表：发送集按 DBC 归属标 `*` 本节点 / `!` 他节点 / `-` 不在库，接收集、系统变量、响应映射）；中栏源码与日志；右栏**函数 / SysVar / 报文 / FlexRay** 四个插入页——FlexRay 单独一页是因为它长（一个真实集群动辄上百个槽），每行写成"`名字 (FRn)` slot N · 帧名"，点选插入 `fr_sig(n, N, "名")`，三个参数都不用另开窗口去查。输入 `Message.`、`FR0.`、`FR.集群名.`、`ETH0.`、`ETH0.帧名.` 或 `ETH.集群名.帧名.` 会自动列出名字；**Alt+/** 手动唤出补全（Ctrl+Space 在中文输入法下是切换中英文的快捷键，所以不用它）。
- 事件：`on start`、`on message <id>`（`*` 收所有数据帧）、`on extended message <id>`、`on fr slot <槽>` / `on fr <路> slot <槽>`（FlexRay 帧到达；不写路号就是所有路）、`on errorFrame`、`on timer <ms>`（周期）、`on timer "名"`（一次性，`set_timer` 武装、`cancel_timer` 取消）。
- **FlexRay 事件里的帧上下文**：`frame_id()` 是槽号、`frame_dlc()` 是字节数、`frame_byte(i)` 读载荷、`fr_cycle()` 是这次到达的通信周期号。一个静态槽按周期相位轮流排好几帧，处理器读到的是**实际到达那一帧**。`send(frame_id())` 在这种处理器里是编译错误——把槽号当 CAN id 发出去是两套编号混用最典型的错。
- 节点会收到**自己发出的帧**：通配或同 ID 处理器里无条件发帧会自激成环，用 `frame_id()` 过滤。
- 帧处理器里用 `this` 读触发帧：`this.bus` / `this.id`（CAN）/ `this.slot`、`this.cycle`（FlexRay）/ `this.len` / `this[i]` / `this.signal("名")`；`node.bus` 是脚本自己的 CAN 通道，定时器里也能用。网关脚本因此不用硬编码触发帧的 id 或总线号。
- 编译或运行出错即**自动停止运行**，日志留原因；重新 Apply 或重启测量恢复。周期定时器错过的点收敛为一次触发。
- `print(...)` 进节点日志（200 行封顶）并同步进 Write 窗口。
- `examples/` 有一组可运行示例（心跳、延迟应答、按值分发、信号监视、嗅探转发、错误监视、派生信号、系统变量联动、FlexRay 网关）。

## 总线与数据库（Buses 窗口）

- `EV_` 环境变量的 min/max 若写成浮点或 `3.4E+038`，导入时会归一化为整数后再解析，不影响报文/信号。
- 一条总线可加载**多个 DBC**：`Open...` 换主库（也接受只含一个 CAN cluster 的 ARXML），拖放或 `+` 附加扩展库；Buses 顶部可按名称/类型/来源过滤总线，也可把 DBC 或 ARXML/FIBEX 拖到弹窗里。解码按库序进行，重名报文先到先得。
- **一次导入全部 ARXML cluster**：Buses 顶部的 `Import ARXML...` 扫描文件里的 `CAN-CLUSTER` / `FLEXRAY-CLUSTER` / `ETHERNET-CLUSTER`，每个 cluster 建自己的总线。CAN cluster 读成 CAN 路（标识符/帧类、帧长、帧→PDU、I-Signal、compu/单位、PDU 周期、ECU 端口的发送方），FlexRay/Ethernet 落在各自编号空间的下一个空索引；同一文件与 cluster 再次导入会刷新那一路的数据库，不会重复建路。多个 CAN cluster 可以共享同一份帧/PDU/信号定义，地址以各 cluster 自己的 triggering 为准；triggering 放在 cluster 外时按 `/Cluster/<名>/...` 引用归属。CAN 行的 `Open...` 可换成 DBC 或只含一个 CAN cluster 的 ARXML（会离开当前 ARXML 源）。Container/Multiplexed/Secured PDU 不在此读取范围。
- **比特率是选的不是输的**：仲裁段下拉给 `10/50/100/125/250/500/800/1000` kbit/s，FD 数据段给 `0（无 FD 段）/1000/2000/4000/5000/8000`。选错比特率不会报错，只会把整条总线的帧全部解错。工程里已存的值一定在列表里。
- **DBC 自动重载**：每 2 秒比对校验和，外部修改自动生效（测量停止时同样比对）；库里新增的发送报文会出现为生成器条目，但**不自动发送**。
- 数据库列表随工程保存（相对工程目录）。
- **节点角色**：每个 DBC 节点是 Buses 里的 chip——勾选 = **Simulated**（本工具以它名义发送），取消 = **Offline**（缺省；真实节点的流量经 Real bus 或回放进来）。切换角色不改写生成器条目的开关。
- 通道编号映射在 **Setup → Replay… 的 Channel map**：把日志里的通道编号指到本工程的哪一路；回放过或存过映射的通道会出现在这里（详见「回放与录制」）。

## 回放块

- 把某节点录制日志里它的报文重新发到仿真总线上，按录制间距发送。在 **Setup → Generator…** 底部的 **Replay blocks** 新建与编辑：选日志、可选 DBC 节点、可选 hex id 过滤（如 `100, 3F4x`；后缀 `x` 表示扩展帧，留空 = 全部）。行上的 **Edit** 可改名称、总线、节点、日志和 id 过滤，Save 提交，非法 id 会就地提示。
- **只在仿真模式下生效**：回放模式下整份日志本来就是信号源，块保持静默。
- 载入失败（路径无效、过滤后无帧、节点不在库中）会**停用该块并就地写明原因**。新块默认停用。
- 声明随工程保存，打开工程时坏路径就会暴露。

## 交互式生成器

- **节点是编辑单元**：**Buses** 里把 DBC 节点设为 **Simulated**；**Setup → Generator…** 按总线列出各节点的条目（每路有 **All on / All off**）（On / 周期 / FD / BRS / 信号拖拽 / 激励）、Add 与回放块，行上标出所属节点。BRS 只在 FD 勾选后可用，让数据段跑总线的 FD 数据速率；Trace 的标记显示为 `FD·B`。脚本在 `on stop` 里用 `test("name", condition)` 记录断言，停止后 Scripts 的 Tests 面板与 Write 里给出通过/失败结论。
- **实际发送当且仅当**：测量运行中 ∧ 节点角色为模拟 ∧ 条目 On ∧ 周期非 0。角色切换**不改写**条目开关；新建条目一律默认关。不发送的原因在行上看得见（行上的状态词显示"角色"或"MUTE"）。
- **FlexRay 槽条目**在 **Setup → Generator…** 的 FlexRay 区按总线编辑，不另开窗口。下拉只列**描述把这路里哪些帧绑给它发送**的槽；描述没说的槽就没有条目可加（状态行会点明缺的是哪一条：没加载描述 / 没调度这个槽 / 没说这帧由谁发）。本工具不推断归属，所以也没有"手工输入任意槽号"的入口。
- **FlexRay 的周期按通信周期数计，不按毫秒**：静态槽在调度表排给它的那个周期发送，能改的只有"每几个周期"，弹窗实时显示它等于多少 ms。`0 = 事件触发`在 FlexRay 不存在（填 0 会被拒绝，要它不发用 On 勾选框）。这一路没有描述（或没声明周期时间）时退回按 ms 编辑，行上的悬停说明原因。改周期数不是改调度表——槽号、相位、载荷宽度仍归描述文件。
- **载荷宽度归描述**：hex 框接受该槽声明的字节数，填少了补 0、填多了**整笔拒绝**并说明差多少（不静默截断）。
- **FlexRay 发的是本会话的流量，不是总线上的帧**：端口目前仅接收，所以生成的帧从接收侧进入会话——Trace / Messages stats、负载与规格、触发器、脚本、录制全都看得见，但它不出端口、也不会被另一台设备收到。面板标题与行上 chip 就这么写。FlexRay 侧没有 DBC 节点，所以也没有角色开关，条目只有 On 与 MUTE 两态。
- 回放时该槽若被日志自己带着，条目整个回放期间不发送（chip 显示 MUTE），退出回放照常。
- 行内 hex 与信号值是**最近一次实际发出的那一帧**；描述被移除后条目照旧存在并发送（行上写明"没有描述，无人解码它"）。
- 信号行：物理值拖拽编辑（被激励接管的只读，改回 Constant 解除）、激励 Ramp / Sine / Step / Random / Triangle / Counter 带参数弹窗。

## 硬件联动（Kvaser / Vector）

- **连接**：Buses 里选中总线，在硬件通道下拉里选本机通道再点 **Attach**（前缀 `[K]` / `[V]` 标明驱动），**Detach** 随时可做。挂上后卡片显示用的是哪个端口。
- **总线模式（Simulated / Real bus）是模拟流量是否经硬件发出的唯一开关**。Simulated 下硬件连接保留但不收不发（行上标"未启用"）；Real bus 下 RX 进入总线，"模拟"节点的一切发送（生成器、脚本、触发反应帧）经硬件发出。想只听不发，把所有节点角色留在"离线"。
- 状态栏 `MEASURING (…)` 括号里说明**帧的走向**：`simulation`（未连接硬件）、`real bus · N`（N 条总线已连接硬件）、`simulation · hardware parked`（已连接但总线模式为 Simulated）。
- **CAN FD**：按总线的 FD 数据段波特率申请 FD 能力；Kvaser 与 Vector 都配置数据段，无匹配预设或硬件不支持时**降级为经典模式**并如实标注（Vector 会带上驱动返回的原因）。
- **USB 重新插拔后的恢复**：程序每秒检查一次 Windows PnP；设备拔出或重新插入时，会自动关闭 Vector 连接、重开 XL 驱动并刷新通道列表，不需要手动点 Refresh，重新插入后的 FD 能力也随之恢复。`Refresh channels` 仍可手动强制重扫。Kvaser 连接不受影响。
- **离线通道**：VN1610 通道按 Windows PnP 的存在性过滤（VID_1248/PID_1050），驱动留下的空名字离线记录也会被清掉；尚未建立型号映射的 Vector 设备仍以驱动配置列表为准。同型号多台设备在通道名重复时会附加 `S/N` 或 `dev N` 以便区分。
- **接收**：连接后适配器收到的一切帧照常进入总线。真实节点就这样进入工具——把它的角色留在"离线"（我们不代替它发送），它的流量照常显示。
- **仅接收降级**：优先申请收发；通道被别的程序占着时自动降级仅接收并标注。物理适配器的仅接收句柄写不进去（帧仍进内部总线，视图不丢）。
- **发送被线路拒绝不再静默**：第一次写入 Write 窗口并带上驱动给的原因，之后只累计次数，Buses 行标红 `发送被拒 ×N`（悬停读原因）。两种失败走同一句话：驱动当场拒绝写入；以及帧被接受、总线上没有节点应答（后者只出现在事件流里，行里带上驱动回报的 tag 号）。
- **与其他工具共存**：驱动的仲裁是通道级的——本工具连接期间别的程序可能起不了同一通道，反之亦然。换工具先在这边 **Detach**。
- 手动连接是会话状态；要固化就在 `profiles/<名>.toml` 里写 `[[hw]]`，再用 `--profile` 启动。
- **FlexRay 监听**：Buses 的 FlexRay 区与 CAN 那张表同形（一路一行，Name / 描述 / 速率 / 硬件 / 移除）。
  - `+ FlexRay` = 挑一份集群描述（FIBEX 2.x/3.x 与 AUTOSAR ARXML 都读，GBK 编码无碍），落在第一个没有描述的路索引上；取消则什么都不加。行集合 = 已描述 ∪ 已监听 ∪ 正在回放的日志里真有其流量；最后那种行会灰字标"日志里有这路流量"，`Open...` 就在它自己那一行。
  - **名字只是给人看的**：行末灰字 `#n` 才是这路的 cluster 索引，`FR0:13`、`fr_sig(0, ..)`、`on fr 0 slot 13`、触发规则存的数字都是它，所以改名不会影响要输入的那个编号。清空它、或重新输入 `FRn`，都等于没有名字。
  - **速率与周期只读**：FlexRay 的位时就是调度表本身，要改得改描述文件。
  - **Detach** 关端口，描述与曲线全留着；行末 `x` **删除这一路**——端口、描述以及引用它槽号的一切（曲线、手动勾选行、发送条目、触发规则、规格判定）一起走，**其他各路的路号一律不动**（索引是身份不是列表位置，把后面的前移会让所有打过的数字在未提示的情况下指向另一条总线）。空出来的号下次 `+ FlexRay` 拿回。
  - 端口号与监听本身不入工程；每路的索引、名字与描述路径入工程。
  - **每路的帧只按该路自己的描述解**：槽号在两个 cluster 之间会重复，所以帧名、解码信号、调度都分 `FR{n}`。描述放错路不报错——名字与物理值都会按另一路的调度解释，看着照常，所以行上印出文件声明的 cluster 名供核对。

## FlexRay 在视图里

- **Trace**：与 CAN 帧按时间交错混排——Bus 列写这一路的名字（未改名即 `FR{n}`，知道通道时带  `A`/ `B`），ID 列是 `slot.cycle`，Name 列是该路描述里的帧名。勾 **"FR 信号"** 展开解码后的信号子行（含枚举标签）。
- **Messages** 每 **(路, 槽, 帧)** 一行：计数、周期均值 ± 抖动、末帧载荷、帧名。一个静态槽按周期相位可排多帧，所以按槽统计会让帧名每来一帧变一次；解不出帧的到达自成一"无名"行。行首 `▸` 展开后按描述顺序列出信号子行：**信号名缩进在 Name 列、物理值放在 Cycle 列、单位与枚举标签放在 Data 列**，原始值在悬停提示里；只有展开的行才解码，收起不付代价。CAN 与 Ethernet 报文同样可展开。顶部的总线选择与搜索框按 bus / name / id 收窄当前 Messages；总线选择与 Trace 共用，切换视图不会丢过滤。**Clear** 清逐报文计数（二次确认），**Expand / Collapse** 一次展开或收起所有可解码行，**Copy** 把当前可见行复制为 TSV。
- **E2E**：受保护的 CAN/FlexRay/Ethernet 行显示 `E2E Pn`；当 ARXML/DBC 给出 profile、Data ID 和 CRC 位置时，同时校验 CRC（Profile 1/2/4/5/7/11/22/44，Profile 4/5/7/44 需显式 CRC 位置），失败标红并写 `✗`；没有 CRC 位置时只保留计数连续性与 Data ID。
- **Messages stats**：FR 槽位与 CAN 报文并列（计数、间隔 min/avg/max、占比）。
- **作用域**下拉有 `FlexRay: <路名>` 一档（与 `Bus: CAN{n}` 分列，因为两套编号不互解）：只看这一路，占比也算这一路的。**Manual 手动勾选的集合**同样能勾槽，与 CAN 报文共用同一份勾选集合。
- **信号可订阅**：Signals / Data / State / Monitor 都能选 FlexRay 信号，身份就是 `(路, 槽, 信号名)`。
- **录与放对称**：CANoe 录的 BLF 与 ASC 里的 FlexRay 收帧一并读入；本工具自己录的文件也写同类对象/行，录完再放回来是同一批帧。`--convert` 与 Trace 的 ASC 导出保留 FlexRay 流量。
- 负载与规格另有口径，见下面"总线统计与规格监视"。

## 车载以太网

- **数据来源**：PCAP/PCAPNG 与 BLF 的 Ethernet 对象（类型 71/120/121）都可回放；录制时 `.pcap` 或 `.blf` 会写 Ethernet，PCAP 仍是通用交换格式。
- **实时口**：Buses 选中 ETH 路，在 **Live port…** 里选 Npcap 接口并挂载；RX 帧进入 Trace/Messages stats/触发/录制，Panel/Trigger/脚本发出的 Ethernet 与 SOME/IP 帧走该端口；Real bus 关闭时端口仍挂着但不收发。Npcap 未安装或没有接口时下拉为空，错误写在行内。
- **矩阵**：Buses 加载 AUTOSAR Ethernet 矩阵（ARXML）或 FIBEX（Vector 导出的 FIBEX 扩展名通常是 `.xml`，不是 `.fibex`）的 CHANNEL/FRAME/PDU/SIGNAL；没有 frame 的 PDU-only ARXML 也会保留 PDU triggerings 与 socket/endpoint，显示 cluster/VLAN、PDU、endpoint 清单（含 endpoint→PDU 关联），并明确提示帧解码不可用。收到匹配 endpoint 的 UDP NM 帧时，PDU 信号解码后按变化写入 Write（重复值不刷屏），并按 RepeatMessageRequest / sleep-ready 信号给出 RepeatMessage / NormalOperation / ReadySleep / PrepareBusSleep / BusSleep 状态（3s/5s 超时）。官方 XSD 校验可运行 `pwsh scripts/validate-arxml.ps1 <arxml>`（默认用 `target/refs/AUTOSAR_00052.xsd` + `xml.xsd`，该 schema 是 R23-11，不是 4.2.2）。捕获 flow 按 VLAN、payload 长度和 IEEE1722 EtherType 打分匹配；多个候选或无候选都不猜，Name 列退回 MAC 对。`FRAME-LENGTH 含 L2 头` 是项目开关（随工程保存）：打开时按「声明长度 − 14」与捕获 payload 比较。
- **解码**：`I-SIGNAL-I-PDU` 里的 I-Signal 位偏移、大小、字节序、符号、factor/offset、单位和值表；同一 I-SIGNAL 被多个 PDU 复用时，按每个 PDU mapping 的 `START-POSITION` / 字节序分别解码，不丢第二个 PDU。Container / Multiplexed / Secured PDU 不在当前范围内。
- **Trace**：以太网帧与 CAN/FlexRay 按时间混排。Bus 列 `ETHn`，ID 列 EtherType，Name 列矩阵帧名（无唯一匹配时是 MAC 对）；右键可把该 L2 flow 加入本窗口的 Manual 选择。作用域多一档 `Ethernet: ETHn`。
- **实时 vs 会话**：没有挂实时口时，Panel 的 **Send message** 只注入本会话并进入当前录制；挂了实时口且 Real bus 打开时，帧走线上，Trace 里由 Npcap 回读。
- **Messages stats**：每个 L2 flow 一行，MAC、VLAN、EtherType、计数与周期；CSV 导出同样包含。
- **订阅**：以太网信号进 Signals / Data / State / Monitor，身份是 `(ETH 路, 帧名, 信号名)`，随工程保存；作用域为某个 Ethernet 路或 Manual flow 时 Messages stats 也按同一规则收窄。
- **Channel map**：抓包工具的以太网 channel 有单独一张表，先映射到工程的 `ETHn` 再匹配矩阵、解码和聚合。
- **导出**：Trace 窗口的 Export 按扩展名选格式——`.asc` 写 CAN/FlexRay；`.pcap` 写经典 PCAP（LINKTYPE_ETHERNET，保留 MAC、VLAN TPID（含 0x88A8）、EtherType 和载荷，不写 FCS；经典 PCAP 没有方向位，Rx/Tx 都按线上帧导出）。Messages stats 的 CSV 里以太网行把 EtherType 放在 `id` 列、VLAN 写进 `name` 列。

## 触发器（Triggers 窗口）

- **条件**：信号越阈（沿触发）、ID 出现、错误帧、周期超时、FlexRay 帧到达（按路 + 槽）、FlexRay 信号越阈、FlexRay 槽超时、系统变量越阈、CAN 载荷字节匹配（未描述帧：字节/掩码/期望值）、CAN 消息 E2E 计数器 gap/jump、以太网 flow 出现（路 + EtherType + 可选 VLAN）、SOME/IP service 消息（路 + service，可选 method 与 message type；留空为任意）。CAN 与 FlexRay 的条件形状一致。
- **动作**：开始 / 停止录制、发送整条消息（CAN / FlexRay / Ethernet / SOME/IP；CAN 反应帧可镜像触发帧的同名信号；Ethernet 用矩阵帧名 + hex 载荷，VLAN/EtherType/长度取自矩阵；SOME/IP 填 service/method/client/session/type，并可选 UDP/TCP 与 TCP host/port；session 填 0 表示自动递增；并可按 ARXML 字段名填 `name=value`（留空则用 hex payload））、插入标记、清空 Trace，以及写系统变量、跑脚本动作（`on panel_action`，可带值）。写单个信号不再作为动作：要临时改一个值，用系统变量做中间量，再由脚本/Setpoint 落到信号。
- **添加与工具**：顶部 **Add rule** 按 CAN / FlexRay / Ethernet / Global 分组列出条件类型；**More** 收纳 Check、Re-arm latched、Export、Import。录制默认值折叠在 **Recording limits** 里，规则行只保留开关、Edit、单条 Re-arm 和删除。
- **每条规则自己的节流与录制上下文**：**Cooldown ms** 限制两次触发的最小间隔，**Max fires** 到数后不再触发（0 = 不限）；**Pre frames / Post frames** 留空继承全局，填写后只影响这条规则启动/停止的录制；行上的 **Re-arm** 只复位这一条的锁存与计数，表头的 **Re-arm latched** 仍管全部。
- **触发可见、可跳**：每次真正执行动作都会在 Write 里留一行（规则、条件、动作、结果），并进入回放条的 **Prev trigger / Next trigger**；**Insert marker** 仍额外在曲线画琥珀色虚线。动作失败（缺发生器条目、未知变量、未测量等）标成 Warning 而不是静默。
- **就地建规则**：Trace 行右键 **Trigger on this frame**、曲线 lane 右键 **Trigger on this signal**，用当前地址/信号预填一条新规则，再在 Triggers 里改阈值与动作。以太网 flow 也可以在编辑器里按路 + EtherType + VLAN 直接建。
- **触发启动的录制**自动带预触发上下文（默认事件前 256 帧）；**触发停止的录制**再滚 32 帧才闭文件。手动录制立即开始/停止。顶部可调预触发 / post / 标记上限，随工程保存。
- **锁存**：ID 出现 / 错误帧 / FlexRay 帧到达这类"出现"条件每次运行只锁一次。**Re-arm latched** 手动重置；**清空 Trace** 的动作也会顺带重置所有出现类锁存。信号越阈与周期超时的电平不受影响。
- 触发器随工程保存；运行态电平与计数不写入文件。
- **导出 / 导入**：Triggers 窗口的 **Export** 把当前全部规则写成 `.rxtriggers`（JSON，带版本）；**Import** 追加到当前工程，按 cluster 名称重新映射总线，逐条校验并在状态行报告跳过原因。引用不存在或重名的总线不会被静默猜一个。
- **Check**：静态检查规则引用的总线、sysvar、派生通道、发生器条目、脚本动作和目标帧是否存在；导入后或开跑前用它找出“武装但永远不会触发/动作必然失败”的规则。它不重放流量，运行时仍以 Write 的 Warning 为准。

## Signals（曲线页）

- **游标 A/B**（曲线区的 **+ Cursor**）：按钮在可见数据中点放一条游标（依次标 A、B、C…，Δt 取 A/B 两条），按住游标线拖动微调，**Clear** 清掉全部。右上角读数给 `A`/`B` 两个时刻与 **Δt**（自动换单位，B 在 A 前就读负数），下面每个 **Plot** 或 **Read** 信号一行 `名字 A值 → B值 Δ差值`。
- **Read 信号**：Signals 选择器里每个信号都有 **Plot / Read** 两段式模式开关。Read 只订阅、不画 lane，但读数表始终存在：没有游标时显示 **Live values** 与最新值（列头 `Now`），有游标时切到游标值，A/B 时一起显示 A/B/Δ 与区间统计。点当前模式可取消，切换另一段直接换模式，Ctrl-click 信号名也能切换 Read。图上一个 lane 都没有时仍可拖游标读数，Read 集合随工程 workspace 保存。
- **Maximize 读数**：Signals 页最大化时读数表变成左侧竖栏，信号名与 A/B/Δ 值同行紧挨，图表占右侧；未最大化仍保留图下方的读数表。
- 游标记的是**时间**：平移、缩放、实时推进都让线跟着视图走，标的点不动；跑出视野的游标不画线，时刻仍留在读数里。没打上的那条印 `-`，没有样本的时刻也印 `-`——不把 0 当成一个读数显示。
- 游标位置与 Read 集合保存在 Web workspace 里，随工程保存恢复。

## Panel（操作页）

- 画布上方是页面标签栏：点击切换页面；Edit 下 `+` 新建空页，`…` 打开页面操作菜单（Rename / Duplicate / Import / Export / Delete；至少保留一页），双击页名就地重命名。每页有自己的画幅尺寸和控件，切换页面会清空选区、撤销历史和剪贴板，但保留 Edit 模式。
- 顶栏 **Standard / Panel** 进入操作页：固定画布默认 **1280×720**，始终显示 **8px 网格线**；画幅 W×H 在未选中控件时显示于右侧 Panel 属性，按 8px 取整，缩小到会把控件挤出画布时会拒绝；窗口不够大时画布出现滚动条，不缩放、不重排。控件尺寸变化时按钮/数值的字号与内边距按控件大小自适应，长标签省略显示，不需要手工调字号。
- **Edit** 进入编辑：右侧属性栏常驻，点控件显示控件属性，点空白显示 **Panel** 属性（画幅 W×H 与控件数），切换选区时画布宽度不变。**+ Add** 菜单添加 Button / Value / Bar / Gauge / Lamp / Toggle / Setpoint，拖动控件移动、拖右下角缩放，位置和尺寸实时吸附网格；最小尺寸按类型，重叠的落点会被拒绝。编辑时控件不触发动作。
- 选中控件后右侧属性栏按类型编辑：**Button** 的 **On click** 二选一、互斥——**Send message**（一次发一条 CAN / FlexRay / Ethernet / SOME/IP 报文；CAN 可填 raw 或目录报文，目录报文与 FR/ETH 已知帧可按物理值编码；FR/ETH/SOME/IP 只注入本会话；SOME/IP 可按 ARXML 字段名填值（字符串写 `name="text"`，union 用 `name.$selector`））或 **Script action**（动作名 + 可选数值参数）；**Value** 填 Source（信号 key、`derived:Name` 或 `sysvar:ns::name`）与小数位，只读显示实时值；**Bar / Gauge** 填 Source、最小值/最大值与小数位，按比例填充并同样可用 **Color bands**；**Lamp** 填 Source、点亮阈值（Lit at or above）与点亮颜色，源值达到阈值时亮起；**Toggle** 填系统变量名，显示 ON/OFF 并写 1/0；**Setpoint** 先选 Target（SysVar 或信号），再填目标、小数位与步进（Step），输入后按 Enter/失焦写入，按对应后端的量程夹取，并可带值触发同名脚本动作。**Color bands** 在所有只读控件上一致：按升序添加上限（`≤`）并为每段选 Neutral / OK / Warning / Critical，可再加一条 `above` 兜底最高段，只改数字/填充颜色、不改卡片底色。属性栏里的 **Duplicate** 与 **Delete** 复制/删除控件。
- 页面文件：Edit 下 `…` → **Export page** 把当前页下载成 `.rxpanel`（带格式与版本，不含控件 id），`…` → **Import page…** 在浏览器里读取文件并作为新页面追加（名称自动去重）。导入严格校验画幅、控件类型、最小尺寸与重叠，任何一条不合格都给出提示且不改动现有页面。
- 编辑快捷键：**Ctrl+C / Ctrl+V** 复制粘贴（自动找空位并选中副本），**Ctrl+Z** 撤销、**Ctrl+Shift+Z** 或 **Ctrl+Y** 重做（工具条右侧是 ↶ / ↷ 图标按钮），**Delete** 删除选中控件。不做手动微调与对齐工具——位置、尺寸一律吸附网格。
- 控件会表达可用性：未测量时写信号/发报文的按钮禁用，写信号的 Setpoint 也要求测量在跑；脚本动作只在有运行中的同名处理器时可用；未知总线/帧、未知 SysVar 或未绑定的控件显示占位并提示原因。
- **Fullscreen** 让 Panel 盖住顶栏和状态栏，但仍保持浏览器窗口；画布按当前尺寸居中，不缩放。`Exit` 或 Esc 返回；编辑时 Esc 先清选中/退出编辑。
- 画布尺寸、控件坐标（按格）与绑定以 `.rxpanel` 文档存进工程文件；打开工程时自动合并回 Panel，不需要手工 Import。View 模式下按钮直接执行所选 **On click**。

## State Tracker

- 对订阅信号的历史做状态分段：值不变的最长区段为一条带，带内标值。
- Signals 页的 lane 菜单或 Data 页里**右键任一信号**即可加入 State Tracker。
- 恒定贴实时边缘（回放中跟随播放头）；时间窗 5 秒到 30 分钟预设；**Min ms** 把更短的区段并入前一段（0 = 全部显示）。
- 默认模式取 VAL_ 值表标签（显示枚举名而非数字）；每个状态可固定颜色，未固定的按会话槽位自动配色。自定义模式按切分点分区间，每区间自定义名字与颜色。
- 可见状态只有 {0,1} 时自动画成二进制方波。
- **Export CSV** 导出当前窗口的区段表（总线、信号、起止秒、时长、标签），与屏幕所画一致。

## Monitor（轻量监控面板）

- **Monitor 页**（可从任意页眉菜单放到任意槽位）：一行一个信号 `[状态点] 标签 值 单位`，从与 Data/Signals 同一棵选择树里挑（数据库信号 / 派生信号 / 系统变量通吃）。
- 每行可开一条阈值规则（`>=` 或 `<=` 时整行换色），行内改阈值、切方向、点色号换色。
- 行随工程保存（键、标签、小数位、规则全部写入工程）；Clear 清空，单行 `x` 删除。

## 系统变量

- **Setup → Variables…**：CANoe 式命名空间变量。表里列 namespace、name、实时值（行内可改）、Init、Min、Max、单位/备注。定义随工程保存。
- 脚本用 `sys_get("ns::name")` / `sys_set("ns::name", v)` 读写系统变量；信号用 `报文名.信号名`（非标识符名字用 `报文名["信号 名"]`）读取。写入被夹在界限内；引用未定义的变量/报文/信号在节点启动时报出来。
- 每个变量发布为一条可观测流，信号树里按 namespace 分组。
- 每次开始测量所有变量回到声明的初值。值为数值型；改名 = 删旧 + 建新，脚本引用不会自动跟随。

## Write 窗口

- **Write 页**：整系统一条滚动输出——节点 `print`（带归属）、装配检查 `[check]`/`[info]`、告警、节点停止的错误（红）、命令消息（蓝）。
- 每行带墙钟时间戳；停在底部时自动跟随新行，向上滚动阅读时不被拽回。顶部按类别过滤。**导出**写当前过滤视图（与屏幕同形）；全被滤掉时拒绝导出而不写空文件。**清空**只清这个窗口，不动测量与录制。
- **状态栏只留最后一条，这里才是记录**：步进中出现的新消息（回放到达末尾、触发命中、post-roll 结束）与前端的失败（工程读不到、导出失败、描述装载失败）都同步写入 Write。
- 打开缺文件的项目时，状态栏只说"loaded · N 项未能载入（Write 窗口有明细）"，逐条原因（红色）在 Write 里——能恢复的部分照常恢复。

## 总线统计与规格监视（口径）

- **Busload** 按线上一帧占时算：经典帧 `47+8·len` 位、扩展帧 `67+8·len` 位、FD 的载荷与 CRC 走数据段速率（位数为无填充近似）。1 秒滚动窗，**窗口只由新帧推进**——总线静默时读数冻结，不归零。错误帧单独计数并计入占时。
- **FlexRay 的静态段占用**单位是槽不是长度：一帧占用的介质时间 = 该路描述的 `静态槽长 × 宏 tick`（载荷 24 字节还是 254 字节都一样）。两点边界：动态段不计入；没有加载描述或没声明槽时长时**只数帧、不报占用率**（显示 `-`，而不是无依据的 0 %）。
- **Clear**（Messages stats）清的是逐报文计数，两种总线一起清；测量照常，负载与规格记忆不受影响，过滤器不重置。
- **规格监视**用逐步区间判定周期（不是运行平均，也不受单帧毛刺影响）。四类判定：**Unknown**（库里没这个 ID / 这路描述排不出这一帧）、**Dlc**（长度不符）、**Cycle**（周期漂移超容差）、**Missing**（静默超过宽限）。同一报文持续违规只加深计数，不重复开行；Export 的 CSV 头部带判定前提。
- **FlexRay 走同一套判据，前提换成调度表**：Unknown = 到达的槽/相位在这路描述里排不出帧；Dlc = 实到载荷**短于**声明长度（长于不算，槽载荷定长，帧名之外的填充不是错）；Cycle = 实测间隔偏离"**周期重复数 × 这一路的通信周期**"（那个 ms 数就在 Buses 行上）超容差；Missing = 原本在到的帧静默超过宽限。
- 扩展帧在表与 CSV 的 id 列带 `ext` 后缀。

## 回放与录制

- 回放按时间戳顺序调度，**不提供进度条随机跳转**（CANoe 的录制模式同样如此）：进度条只跟随不回跳，要向后赶用 **Fast**（一次读到日志末尾）。倍速可增减可直选；暂停/恢复的边沿不产生重复帧。
- **SOME/IP**：回放 PCAP/PCAPNG 或 BLF 时，IPv4/IPv6 的 SOME/IP 消息会解码到 **SOME/IP** 页，分 **Events / Services / Availability / Database**：Database 是不需要流量的 ARXML 视图，可按 service/instance/endpoint 过滤，instance 显示 endpoint，并在下方列出远端订阅者；Events 默认只列时间、service、method、type 和字段摘要，点一行展开 L4/方向、client/session、return、endpoint、SD 与完整 payload，不靠横向滚动；Services 按 service 汇总消息数、method 列表、类型、SD offer 给出的 endpoint、平均周期、抖动和时间跨度；Availability 按 service/instance 给出 Offered / Stopped / Expired、TTL、endpoint 与订阅数（按抓包最后一帧判断 TTL 过期）。UDP 要求长度字段与 datagram 严格一致；TCP 按方向做顺序重组；IPv4/IPv6 分片按 offset/more 重组；IPv6 的 Hop-by-Hop / Routing / Destination / Fragment / AH 扩展头会逐跳跳过。Service Discovery 解析 entry 类型、service/instance、版本、TTL、IPv4/IPv6 endpoint、eventgroup 与 configuration option（按 SD 头里的 entry/option 长度读取，不猜布局）。SOME/IP-TP 按 offset / more-segments 重组为一条消息；ARXML 固定/动态数组（发送可写 `name.$count`）、record、FIELD notifier、service instance/eventgroup、application→implementation 映射、TEXTTABLE 枚举、string/union 已能展开，string/union/struct/array 的长度字段与 alignment 由 SOMEIP-TRANSFORMATION-PROPS 驱动。ARXML 解出的 SOME/IP 字段会以 `SOMEIP 0x1234/0x8001` 这样的合成 Ethernet 帧出现在 Signals 选择器里，record/array 展开为 `Details.Speed`、`Wheel[0]` 等叶子，枚举保留数值供曲线使用，单位与量程从 `UNIT-REF`/`DATA-CONSTR` 读取，可像普通信号一样 Plot/Read、进 Monitor/Data/State。Events 详情可复制 payload；缺 ARXML、方法/事件未定义或 payload 短于布局时会就地显示原因。本地 Offer 收到 `type=0x00` method 请求时，按 ARXML output 字段自动回 `0x80` 响应（字段填 0）。Panel/Trigger 以 `type=0x00` 发请求、`session=0` 自动编号时，核心会把同 service/method/client/session 的 `0x80` 响应记到 Write；5 秒内没有响应会写超时告警。
string/union 的 length-field 宽度与 alignment 会读取 SOMEIP-TRANSFORMATION-PROPS，struct/array 的 length field 也按 props 宽度读取。
- **以太网抓包回放**：`.pcap`、`.pcapng` 与 BLF Ethernet 对象可直接作为回放源（经典 PCAP / PCAPNG / BLF 类型 71/120/121）。抓包的绝对时间戳会重基到第一帧，帧进入 Trace/Messages stats 并参与解码；PCAPNG 多接口按接口编号作为文件里的 ETH 路号，可由 Channel map 指到本工程的路。非 Ethernet 链路类型的包会被跳过，不阻止整个文件打开。
- **前向导航**：Replay 行可设步长（10 ms–5 s）并按 **Step** 立即前进；**Play for** 从当前位置播放指定日志时长后自动暂停；**To** 输入绝对日志时间（秒）并 **Go**，播放到该时刻后暂停。三种都走正常回放步进，统计/观察器与连续播放一致；目标在播放头之前会被拒绝（向前有效）。
- **别人的日志靠编号对上**：日志里每条帧只带一个通道**编号**，那是录它的那个人工程里的编号，文件里没有总线名可对照。**Setup → Replay…** 里的 **Channel map** 列出当前录制里有哪几路、各带多少条（CAN、FlexRay、以太网各一组），每行下拉指到本工程的某一路，或选 **No replay**。**As-is** 表示"文件里的编号就当本工程的路号"，表里不留条目；编号在工程里落不到任何一路时，行上直接写 **not replayed**。改动立即生效（已经进 Trace 的帧不动），随工程保存。回放条上的 **Channels** 按钮打开同一张表，有帧被丢掉时直接标出来。
- 编号在本工程对不上任何一路的 CAN 通道**不再冒充第 n 路**：以前它以 `CAN3` 的名义出现在 Trace 里又解不了码，看着像总线对上了。现在它先被挡在门外，状态行说一句（每个通道只说一次，同时进 Write 窗口），Channel map 把它的条数照列，等你指过去。FlexRay 一侧照旧——没有描述的 cluster 号也照常进 Trace，Buses 的行上标"日志里有这路流量"。
- 回放中发生器照常可注入（落在日志时基上；日志静默段注入随之冻结）。回放期间不录制，注入帧不写入文件。
- **录制后端随扩展名**：`.asc` 写 ASC，`.blf` 写 Vector 二进制容器（经典 / FD / 远程 / 错误帧、FlexRay 收帧与 Ethernet 类型 71），同时生成同名 `.eth.pcap`；`.pcap` 只写 Ethernet PCAP。
- **边录边写**：ASC/BLF 与以太网 PCAP 都在录制过程中增长，不用等停止；进程被中止也不会把整段丢掉。以太网 sidecar 可在 Recording 面板单独下载或作为回放源加入 Replay。
- **录制位置**：Record to 框只填**文件名**（默认 `record`，自动加日期后缀），存到打开工程的 `Record/` 下；填绝对路径则按它保存；没开工程时落到程序工作目录。开录时状态行显示完整路径。
- **录制过滤**同时认两套编号：CAN 写 hex id 列表（`100, 3F4x`，`x` 为扩展帧），FlexRay 写 `FR<路>:<槽>`（如 `FR0:13`，路号是行末那个 `#n`，改了名也照旧用编号）。这个框列的是"要留什么"，不是"关掉哪条总线"，只写写了槽号不会让 CAN 一侧停止发送。过滤器只管文件，Trace / 统计 / 负载 / 规格始终看全总线。留空 = 全录。错误帧没有 id，过滤开启时不落文件。
- **BLF→ASC 转存**：`--convert <in> <out.asc>` 纯流式转码，时间戳 / 帧类 / 载荷原样保留。

## Trace 窗口

- **虚拟滚动**：几十万行也能流畅滚动与排序。容量下拉 50k–2M 帧（随工程保存），**上限按帧行计**——展开的信号子行不占额度，所以展开与否看得见同样多的历史帧。超出容量时最旧的帧写入临时归档（不丢），窗口顶部提示"已归档 N 帧"，**Export Trace 会先回放归档再写热环**，导出的 ASC 覆盖整段运行；文件名选 `.pcap` 时导出以太网热环（以太网没有归档）。
- **过滤**：总线范围 / 方向 / 仅 DBC / payload 字节搜索（hex 对，如 `11 22 3F`）/ 帧类型 / 文本（id 或报文名子串）/ 时间范围（两个秒数框，留空 = 不限）/ 信号值条件（`Name>10`，运算符 `> >= < <= == != =`）。只管这个窗口的显示。
- **Filters · N**：payload / 帧类型 / 仅 DBC / 时间范围 挤在可折叠行里，行收起时条件照旧在生效——所以按钮上写明此刻真在生效几条，悬停点名是哪几项（填了但解析不出来的不算）。行右键 **Clear filter** 一次清除全部条件，**唯独不动 Manual 手动勾选的集合**（那是你一条条勾的名单，不是一时输入的过滤式）。
- **精确地址**：普通文本是子串搜索（`13` 同时命中槽 113、槽 130 和名字里带 13 的帧）。只要这一个地址就写前缀：`id:1AB`、`id:1ABCDEFx`（29 位扩展帧）、`slot:13`（十进制）。两套编号不互解。行右键 **Filter this ID** 写的就是精确形式。**输入不完整时不猜测**：`id:` 后不是 hex、`slot:` 后不是合法数字时，整串按普通子串处理（表里通常就空了，改一个字符即恢复）。
- **信号值条件两种总线都算**：CAN 行问 DBC，FlexRay 行问这一帧在自己那路描述里解出的物理值——与曲线上画的、展开子行里印的是同一个数。这一帧没这个信号或这路没有加载描述，该行不出。多个条件是"且"。
- **点一行就是"这一行"**：左键点任一行，该行亮蓝底（与错误行、游标底色互不遮挡，同一行可以同时是"选中的"和"被标记的"）。表头提示 `已选中一行 · ↑/↓ 换行 · Ctrl+C 复制该行`。↑/↓ 按屏幕行序走（新行在上，所以 ↓ 往旧的方向），一个展开的 FlexRay 帧算一步（子行跟着父行一起亮、一起跳），到底就停住，不会把选中清掉；选中的行因过滤或裁剪不在了，下一次按键从当前最新一行重新落地。Ctrl+C 复制该行，八列与表格一致。两个键只在指针停在本窗口、没有输入框在打字时接管。
- **表里的两条游标 A/B**：行右键 → **Set cursor A / B** 把游标落在这一行的时刻（两种总线都放，一次右键只动一条），被标的行带底色（A 琥珀、B 青蓝，与 Signals 曲线同两色），表头读 `A … B … Δt +221.433 ms`（时刻给到微秒）。**Clear cursors** 只在有东西可清时出现。
- **Clear** 清空显示与归档，测量照常进行（录制与逐报文计数都不动），过滤器不重置。
- 时间范围两框、过滤行是否展开、FR 子行是否展开、选中与游标位置：前三样随工程保存，后两样是会话状态。
- **快捷键**：F9 启动/停止，F10 暂停/继续，F2 聚焦当前过滤框，Alt+1…4 切布局，Alt+Shift+1…2 在聚焦槽位显示 Trace / Messages，Alt+Shift+P 切 Standard / Panel（避开浏览器占用的 Ctrl+1…3）。
- **拖放**：DBC Explorer 的信号可拖到 Signals / Monitor / Data 页；目标页当前是哪个面板就加到哪个观察器。工程、ASC/BLF/PCAP/PCAPNG 与 `.rxpanel` 可直接拖进窗口；DBC/ARXML/FIBEX 拖到 Buses 弹窗。

## 工程文件

- 工程（`.rxproj`）单文件 JSON，DBC 路径相对工程目录。
- 只保留当前格式：旧版本工程里已删除的字段不会再被迁移（例如 ImGui 时代的 desktop dock 状态、`sim_nodes`、单 FIBEX 路径）；未知字段按 serde 规则忽略。
- 新会话的 Standard 默认是 **Main + side**（Messages / Rate / Messages stats）；四个布局各自保存页面分配与分隔比例。打开工程时布局、Signals 曲线配置与 Panel 页按文件恢复。
- 坏库容错：复用报文的 mux 条件引用了不存在的开关时按"无条件"处理（显示数据而不是空解码）。
- 信号类型标记（`[u16]` / `[f32]`）按 `SIG_VALTYPE_` 声明解位模式。

