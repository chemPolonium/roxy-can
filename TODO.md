# TODO

只记录**尚未完成**的工作项。已完成能力的行为说明见 `README.md` 与 `docs/usage.md`；系统结构与设计决策见 `docs/architecture.md`；已完成批次的明细见 git log（本文件曾长期记录批次明细，2026-09-15 起按"只记未完成"的既定原则精简）。

## 2026-09-17/18 夜间批次：用户清单七项 + FlexRay 解析（全部落地 ✅，明细见 git log）

录制路径默认进工程 Record/ 目录；DBC/脚本/FIBEX 的 GBK 编码容忍；Trace 工具条收拢；VN7640 能力位诊断（probe 逐通道 FR 试开）；Graphics Y 轴防遮挡 + 曲线面板横向滚动；CAN+FlexRay 在 Trace/Messages 混排合并（含 FR 信号展开、帧名过滤、CSV 导出）；脚本编辑器三栏化（左大纲/收发、右函数/SysVar/报文）；**FlexRay 数据库解析**（`fr_db.rs`，roxy-fibex 同源：FIBEX 2.x/3.x + AUTOSAR R4.x + 调度表 + 信号解码 + 驱动集群配置推导，真实 PowerTrain.arxml 48 帧回归锁定）。

## 定位（2026-09-11 评审确立，一切取舍的基准）

**竞争对手是开源工具（SavvyCAN / BUSMASTER / candump+PlotJuggler / python-can+cantools），不是 CANoe。** 目标：比开源更流畅、更易用，功能更强，但保持轻。竞争四维度：**打开即分析 / 检索顺手 / 渲染不卡 / 零配置出结论**。CANoe 的"重"（面板 HMI、诊断栈、测试报告、多机分布式、实时与时间同步、多协议、用户权限）明确不做——那些不是更好的功能，它们就是"重"本身。既有优势不许丢：DBC 解码完整性、事实源纪律（DBC 唯一事实源 + 角色两态 + 驱动可插拔）、工程可复现（.rxproj + profiles + 无头 CLI）。逐项对照与依据见 `docs/roadmap.md`（§1-§2）。

## 2026-09-15 晚间批次：稳健性 + 表达力收尾 + 监控面板 + BLF（全部落地 ✅）

1. ~~**核心线程 panic 隔离**~~ ✅：`spawn_lane` 循环三处执行点（命令处理/队列排空/测量步进）包 `catch_unwind`——核心代码（非脚本，脚本有熔断）的 bug 不再静默杀死 bus-core 线程，转成 Write 错误 + 停测，循环继续服务命令。
2. ~~**DBC 自动重载 × `$` 引用失配**~~ ✅：重载后原地重算各节点 `$报文::信号` 名字→id 映射，变化/消失进节点日志 `[reload]` 行。
3. ~~**print 数值格式控制**~~ ✅：`format(fmt, ...)` 内建（`%d/%u/%x/%X/%f/%.Nf/%e/%g/%s/%%` + 宽度/`-`/`0` 标志），返回字符串进 print/日志。
4. ~~**轻量监控面板 v1**~~ ✅：View > Monitor——`[色点] 标签 值 单位` 文本行，行内阈值着色规则（方向/阈值/色号行内可调），行随工程保存并自动重订阅；Clear 清空。
5. ~~**BLF 录制写入**~~ ✅：录制按扩展名选后端——`.blf` 走二进制容器写入（LOGG 头 + zlib 压缩 LOG_CONTAINER；经典帧 CAN_MESSAGE、FD 帧 CAN_FD_MESSAGE_64、RTR/扩展标志齐全，错误帧 v1 不写），写→读回环测试逐帧一致。后续（2026-09-16）：录制格式改为**工具栏 combo 显式选择**（`_<date>.asc` / `_<date>.blf`），摆勾时 combo 决定扩展名（覆盖手敲路径里的扩展名），不再隐式依赖命名。
6. ~~**触发条件加系统变量**~~ ✅：`SysVar { key, threshold, rising }` 按步扫描活注册表，Triggers 窗口 "+ SysVar" + 变量下拉/阈值/方向，持久化 kind=4；测试锁定"越阈一次一发、保持不重发、回落再越阈再发"。

### 需要人工验收（批次完成后）

- 监控面板：加行/着色/随工程恢复；View 菜单开关。
- BLF 录制文件能被 CANoe 或 Vector 工具打开（自家读侧回环已过）。
- 人为触发核心 panic 的场景：Write 出错 + 测量停止 + UI 可再次 Start。
- FlexRay（2026-09-18 夜）：VN7640 回连驱动后 `--vector-probe` 应列出 6 个通道并逐通道 FR 试开；Buses 窗口挂接 FR 监听（ch0 + PowerTrain.arxml 或 CANoe demo XML），真机收帧后核对 Trace 帧名/信号解码与 Messages 聚合；确认后把事件缓冲里 A/B channelMask 偏移补进 `parse_fr_event`。

## FlexRay RX-only（FR-1/FR-2 已落地，余下分期）

- ~~**FR-1 枚举分类**~~ ✅（2026-09-15）：`vector::enumerate()` 读取 `channelBusCapabilities`，`ChannelInfo` 带 `can` / `flexray` 标记；`enumerate_flexray()` 列出 FlexRay 口；CAN 挂接下拉**排除显式 FlexRay 通道**。实测坑：虚拟通道的能力位报 0——过滤采用保守策略（能力位为 0 视为 CAN，保持旧行为；只有显式 FR 位才排除）。
- ~~**FR-2 RX-only 接收绑定**~~ ✅（2026-09-15，绑定落地、真机收帧待 VN7640）：`XLfrClusterConfig`（79 个 u32，316 字节）与 `XLfrEvent`（512 字节，pshpack8）布局均经 **MSVC offsetof 探针**对本机 vxlapi.h 实证（探针顺带抓出并修正事件缓冲过小的溢出隐患——驱动按 512 写，缓冲必须给足）；`xlFrSetConfiguration` / `xlFrGetChannelConfiguration` / `xlFrReceive` FFI + `FlexRayChannel::open_rx` / `try_read` / `channel_config`（事件偏移有合成缓冲单元测试锁定）；`flexray_rx_probe` 探针与 **`--vector-probe`** CLI 在 FR 通道存在时打开并抽干 2 秒打印帧 + 转储通道现有集群配置（status 带 `VALID_CLUSTER_CFG` 时参数自动到手），不存在时跳过。**剩余**：VN7640 插上后跑 `--vector-probe` 拿真实诊断；集群参数（.arxml 或参数表，或确认 demo 默认集群；若通道已带 `VALID_CLUSTER_CFG` 则直接读回）。
- ~~**FR-3 Trace 展示**~~ ✅（2026-09-16，管线落地、真机数据待 VN7640）：按调研路线 (b) 落地——BusCore 增 `FrRing`（独立环，与 CAN 环共享 trace_limit 容量与 publish 节奏，FR-only 步也发布）；`FrRow`（t_us / slot / cycle / 载荷 / header_crc / flags，通道 A/B 暂记"未知"——事件缓冲里 channelMask 的偏移未经验证，宁缺毋滥）；Trace 窗口底部折叠区 "FlexRay (N 帧)"：时间 / [FR] 徽标 / slot.cycle / 载荷 hex / header CRC，倒序显示；**不进聚合/负载/规格**。实时源：`SetFrWatch` 命令 + `Hardware::attach_fr`（FIBEX/ARXML 描述文件 → `open_rx_with_fibex` 配置后开通道，只收），step 循环与 CAN 适配器同节奏抽干（`poll_fr`），Simulated 总线模式下照常抽干但丢弃（防驱动队列积压旧帧，与 CAN `poll_rx` 同策略）；Buses 窗口 FlexRay 区挂接/断开（通道枚举一次缓存）；挂接失败（文件缺失 / 无集群参数）整份拒绝并报状态行。**剩余**：VN7640 真机收帧验证。
- ~~**FR-3d --vector-probe 增强**~~ ✅（2026-09-16）：`--vector-probe --fibex <f>`——先校验描述文件并报告集群参数（bit/s 波特率、宏周期、静态槽、静载荷、帧触发含首槽号），再按解析出的配置（不再用零配置探针）RX 试开 2 秒，首帧转储事件原始 64 字节（标注已知偏移）供**真机定位通道 A/B 的 channelMask 偏移**；文件损坏/非 FIBEX 整份拒绝（驱动调用前）。**并且逐通道做 FR 试开扫描**——VN7640 这类 portal 设备的能力位不报 FlexRay（实测 can:true flexray:false），唯一诚实的判据是试开本身（虚拟通道会拒绝，证明判据有牙）。
- ~~**FR-5 FIBEX/ARXML 数据库解析**~~ ✅（2026-09-18）：按 roxy-fibex 同源解析器移植 `fr_db.rs`（roxmltree 全量树解析）——FIBEX 2.x/3.x 与 AUTOSAR ARXML R4.x 双方言（根元素识别 + 特征元素回退）、GBK 编码容忍、属性容错（`fx:` 前缀 / `xsi:type`）；集群参数（含 vxlapi 驱动配置所需全部附加字段）、ECU、PDU、信号编码（因子/偏移/范围/单位/枚举标签）、帧 + PDU 映射 + slot/cycle/通道调度全量提取；`fr_cluster.rs` 平铺扫描器删除。**接入**：Trace FR 行显示描述文件帧名；Messages 每槽位一行、展开显示解码后的信号物理值（含枚举标签）；驱动集群配置直接从数据库推导（周期/宏 tick/微 tick 派生与 CANoe demo 数学一致）；真实 PowerTrain.arxml（GBK、48 帧）回归测试锁定。**剩余**：VN7640 回连驱动后跑 `--vector-probe` 确认 FR 通道并补 A/B 偏移；FlexRay 发车（FR-4 级别）。
- **FR-4（另立项）**：FlexRay 数据库/集群解析器（.arxml ≈ 重做 dbc.rs）、信号解码、周期/抖动统计、脚本发车——与 R2 静态分析的关系一并重新评估。

## 2026-09-19 FIBEX/ARXML 加载提速（已落地 ✅）

按"CANoe 秒开、我们慢"的对照实测定位：解析本身不慢（426 KB ARXML 总 6.9 ms，其中 roxmltree 建 DOM 2.0 ms），慢在**整树被反复遍历**——按 tag 计节点访问：426 KB ARXML 共 689 585 次 = **42× 元素数**，62 KB FIBEX 70 969 次 = 26×。三个来源：① 每个关心的 tag 都重扫全文档（ARXML 10 次、FIBEX 7 次）；② **逐条目**重扫全文档解析引用——FIBEX 每个引用 UNIT 的 CODING 扫一次（fr_db.rs:662）、ARXML 每个 COMPU-METHOD 扫一次（:1202，本例 38 次 × 16.5k 节点 ≈ 占总访问 90%），即 O(条目数 × 文档大小)；③ `text_of` 为每个候选 tag 先把整棵子树收集进 Vec 再看文本。

改法：`DocIndex`（先序一次遍历建 `tag → Vec<Node>` 与 `ID → Node` 两张表，保持文档序所以"首个匹配胜出"语义不变）、UNIT 显示名一次解析成 map 供全部 COMPU-METHOD 用、`text_of` 改命中即返回的递归（`first_text`，不分配）、FIBEX 组装帧时 `pdu_raws.iter().find` 改 `pdu_by_id`。另：**挂接硬件时不再重复读文件**——`Hardware::attach_fr` 改用前端已解析的 `Arc<FrDb>`（`open_rx_with_db`），顺带消掉一个真 bug：原路径用 `read_to_string` 读 FIBEX，GBK 导出的描述文件前端能容错解析、硬件挂接却报"FIBEX 读取失败"。

实测（release，本机）：ARXML 6.9 ms → **3.7 ms**（其中 DOM 2.0 ms 是新下限），FIBEX 1.0 ms → **0.58 ms**；把文档按 10× 复制（380 个 COMPU-METHOD）后，被删掉的那一类逐条目全树扫描单独计时是 **57.5 ms**，而整份解析现在只要 8.1 ms——**信号越多差距越大**，这才是与 CANoe 的差距来源。回归：620 测试全过（真实文件帧数/位偏/CarSpeed=118 口径未变），clippy 干净；`fr_watch_requires_a_description_the_core_holds` 钉住"核心未持描述则不碰驱动"这条新边界（旧的"非 FIBEX 描述被拒"判据上移到前端 `FrDb::parse`，那里本来就解析失败）。

**剩余可做的加载/解码提速**：`FrDb::frame_at` 仍是每次到达线性扫全部帧、`decode`/`decode_signals`/`signal_names` 每个 PDU 线性扫 `self.pdus`（回放每帧都跑）、`slot_signals()` 在信号选择器里每帧重算——都该在 FrDb 里预存 slot→帧 与 pdu 名索引；真要再快就把 roxmltree 换成 quick-xml 单遍流式（DOM 构建是现在的下限）。

### 同日第二批：FR 运行时索引 + DBC 读取实测（已落地 ✅）

- **FR 每帧查找索引化**：`FrDb` 增私有 `pdu_ix`（PDU 名→下标）与 `slot_ix`（slot→帧下标，文档序），构造函数 `FrDb::assemble` 一次填好（`dedup_frame_names` 因此前移到 assemble 之前——表里存的是 `frames` 的位置）；`frame_at` 不再每次到达线性扫全部帧，`decode`/`decode_signals`/`signal_names` 共用 `pdu_of` 取代逐个 `pdus.iter().find`。A/B 实测（release，39 156 次到达，同一批工作量只换查找方式）：真实 demo 库（24 PDU/48 帧）索引 952 µs vs 线性 252 µs（小库上线性更快，但差值 ≈18 ns/到达，等于没有）；把库扩成生产形状（5 024 PDU）后索引 978 µs 持平、线性 **134 ms**——**137× 差距，且只有大库才有的那种痛**。
- **DBC 读取实测（结论：我们的代码没有二次方）**：合成 1500/3000 报文 ×8 信号的 DBC 扫描——`can_dbc` 解析 15.2/29.7/**61.3 ms**（线性 ≈0.02 ms/报文，占加载 85%，是第三方 pest 文法的下限），`SymbolTable::from_dbc` 2.3/4.8/**10.6 ms**（线性）。所以 DBC 慢不慢取决于 `can_dbc`，不是我们的遍历结构——**不换掉它就没有量级可省**（要换就是自写或 quick-cantools 路线，另议）。
- **`absorb` 的 O(n²) 已修**：多文件并表时 `primary.order.contains(&key)` 逐条线性查，几千报文 × 几个文件就是秒级合并——改成一次性 `HashSet<MsgKey>`。节点表（几十条）保留 `contains` 不动。
- 回归：620 测试全过（含真实 ARXML 的 48 帧/位偏/CarSpeed=118 与 `--export-csv` 端到端 3 917 行 CarSpeed 逐值不变），`cargo clippy --all-targets` 零告警。

**还剩下的同类项**：`ui/idfilter.rs` 的 FlexRay 选择器每帧重算 `slot_signals()`（改成随 db 缓存）；`script`/`node` 侧 `message_id_by_name`、`node_tx_ids`、`node_rx_signals` 仍是线性查（只在脚本启动/重载/建块时跑，非每帧，故暂不动）；DOM 构建与 `can_dbc` 是两条读取路径各自的新下限。

## FlexRay 一等总线（2026-09-19 立项，分期；本轮只做接收侧）

现状与 CAN 的差距（盘点结论，逐项带现位置）：**FR 只有两个命令**（`SetFrDb` bus.rs:149、`SetFrWatch` bus.rs:299），CAN 侧约 30 个；FR 不是 `Channel` 实体——全局仅一份 `fr_db` + 一个 `fr_watch`（`HwMap::fr_watch: Option<FrWatch>` hw/mod.rs:202，最多一路），FlexRay-only 端口还被 CAN 挂接下拉显式排除（hw/mod.rs:87-91）；`FrRow` 无总线索引（trace.rs:21）、实时 `ab` 恒为 2（hw/mod.rs:350）、BLF 的 `slot` 实为 frameId（blf.rs:572）；FR 信号靠 `FR_SIG_BASE|slot` 合成 id 挤进 CAN 的 `SigKey`（app.rs:51），A/B、cycle、多 cluster 分不开且通道增删重映射不安全；`ingest_fr_row`（bus.rs:1581）相对 `ingest`（bus.rs:3609）跳过 frame_counter / bus_loads / 通用 aggs / 触发器 / spec / 节点派发 / recorder / 录制白名单 / trace 归档与导出；持久化只有一个 `fr_fibex` 路径（config.rs:550），恢复时既不重挂 watch 也不推 DB 给 core（config.rs:1347，现靠 app.rs:891 回放前兜底）。**本轮范围：S1-S4 接收侧 + S5 硬件在环；发送与调度（静态/动态槽发车、xlFrSendFrame、Generator/脚本认识 FR）不在本轮**——那是 FR-4 的"发车"半边，等接收侧定型后另批。

- **S2 真实 FR 身份键（先做，影响面最大）**：`SigKey` 由 `pub type = (u8,u32,bool,String)`（observe.rs:401）改为枚举 `Can{ch,id,ext,name} | Fr{bus,slot,name}`，删 `FR_SIG_BASE`/`fr_signal_key`（app.rs:46/51）。改动面：约 13 个文件、56 处 `.0/.1/.2/.3` 字段访问——编译器逐点暴露。工程文件旧键（合成 id）要能读出并迁移，不许静默丢曲线。收益：过滤器/触发/统计/持久化第一次可以按 FR 语义工作。
  - **2026-09-19 实做摸底**（把枚举定义落了一次，让编译器数出全貌后回退，树保持绿色）：**177 个编译错**，分布 app_tests 79 / observe 31 / ui/state 27 / config 19 / bus 15 / workspace 9 / ui/idfilter 9 / channel 8 / export 6 / ui_tests 4 / headless_tests 3 / siglist·graphics 各 2 / ui/data 1 / app 2。三类机械模式占多数：① 元组字面量 `(0, 0x100, false, "X".to_string())` → `SigKey::can(...)`（测试里成片，一条正则可扫）；② `.0/.1/.2/.3` → `bus_index()`/`id`/`ext`/`name()`；③ `fr_signal_key(slot,&name)` → `SigKey::fr(0, slot, name)`。
  - **两处必须人判断的语义点**（不能当机械替换）：① **通道增删的重映射**（channel.rs:205/207/238/240/241、bus.rs:2263/2739/2740 那类 `key.0 -= 1`）——那是 CAN 通道号，FlexRay 键现在靠 `ch=0` 混在同一字段里，重映射会把 FR 键一起挪走；枚举化后这些站点只能作用于 `Can`，`Fr` 键要按 FR 总线索引另走一套（与 S3 的通道列表同批）。② **DBC/信号元数据查找**（bus.rs:1946/3739 的 `subscribed_values`、observe 的 `signal_meta`）拿到的是 `(u32,bool)` 报文键，必须显式 `match` 出 FR 分支走 `fr_db`，不能再靠 id 位猜。
  - 另需一并决定：工程文件里 FR 键的写法——`SignalCfg{ch,id,ext,signal}`（config.rs:194）加一个 `fr_slot: Option<u16>` 显式表达，读取时仍认旧的 `id & FR_SIG_BASE` 合成键（迁移而不是兼容层：写出不再产生合成 id）。
  - 建议独立一轮专门做（约 15-20 个编辑回合，每回合都要过编译器），不与其他改动混在同一轮。
- ~~**S1 FrRow 携带总线归属**~~ ✅（2026-09-19 部分）：`FrRow` 增 `bus: u8`（~~日志一次抓取只有一个 cluster → 回放侧恒 0~~ **该假设已于同日纠正**：BLF 有 `wClusterNo`，`assets/fibex/Logging.blf` 实为两 cluster，见下面"BLF 里的 clusterNo"一节；ASC 无此字段才真的恒 0；实时 watch 打自己的索引），`FrSlotAgg` 与核心表改按 `(bus, slot)` keyed，`SetFrWatch`/`attach_fr`/`FrWatch`/`FrWatchView` 全程带 `bus`，Trace 总线列、Messages 行与导出 CSV 都显示 `FR{n}`；回归 `the_same_slot_on_two_flexray_buses_stays_two_rows`。**剩余**：`ab` 真实化依赖真机确认事件缓冲 channelMask 偏移 → 并入 S5。
- **S1b 事件语义**：`frameId → frame` 与真实 slot 分开解析（BLF 只有 frameId，`blf.rs:572` 现把 frameId 塞进 `slot`；实时 VX1070/VN7640 给真 slot）——回放要按 frameId 解析帧，不能冒充 slot 键。
- ~~**S3 FR 通道列表一等化**~~ ✅（2026-09-20 落地，见下面两节）：描述数据库按 bus 存（`fr_buses`/`fr_dbs`，工程里是 `fr_buses: Vec<FrBusFile{bus,path}>`，旧 `fr_fibex` 单路径迁移到 bus 0）；`Option<FrWatch>` → `fr_watches` 按 bus 多路 + 快照列表 + Buses 窗口列表化；工程打开即推 DB 给 core（删 `replay()` 兜底）。**改主意的一项**：原计划把"工程打开即挂 watch"也做掉，但 CAN 侧工程载入并不自动挂硬件（`apply` 里没有 AttachHardware），FR 自动挂会在启动时打驱动、失败还得弹状态 —— 与 CAN 行为不一致，故不做；每路的 `channel_index` 也就不进工程文件（挂哪路端口是会话决定）。
- **S4 接收侧对齐 CAN**：`ingest_fr_row` 补 frame_counter、每路 FR 负载与周期/抖动统计（沿用 aggs 形状，槽占用率口径另定）、FR 过滤（`workspace.rs:759` 的 `trace_fr_match` 现为一律丢弃）、录制写出 FR 帧（BLF FR_RCVMESSAGE 对象 + ASC `Fr RMSG` 行，回环自证）、`export_trace`（export.rs:10 现 CAN-only）纳入 FR、触发条件支持 slot/帧到达。
- **S5 硬件在环（有 VN7640，可实测）**：多路 FR 同时打开、各自集群配置、`xlFrGetChannelConfiguration` 回读校验、真机确认 `ab` 与 slot 语义；CLI 探针扩到逐路报告。

### 2026-09-20 第二轮盘点：FlexRay 与 CAN 还差什么（逐项带位置，含已定的设计）

本轮已落地（不再列为差距）：**S4 收尾**——静态段占用率（`load::FrLoad`，口径见《总线统计与规格监视》）；**Network 视图**列出每路 cluster 的 ECU（`ui/network.rs::draw_flexray_section`；**这里曾经排过一整张按槽分组的调度表，2026-09-21 用户定掉：拓扑视图只列 ECU，调度归 FIBEX/ARXML 编辑器**，见下面"分工"那条）；**脚本读数** `fr_sig(cluster, slot, "Name")`（`HostInput.fr_signals`，从 `fr_aggs` 现解，无节点时零开销）；**State 窗口枚举标签**走描述自己的 VALUE 表（`ui/state.rs::table_label` 的 Fr 分支）。

2026-09-21 追加落地：**Trace 展开子行不再摊平进行列表**（父行只带一个子行计数，文本推迟到绘制时生成——同一次刷新从 70 ms → 7.9 ms，与不展开的 6.2 ms 同档；顺带修掉两个潜伏错误：子行占用 200k 缓存额度、排序把子行打散）；**Messages/Trace 展开行的信号名与值分列**（名在第 1 列、值在第 2 列走**列级** `NO_CLIP`，见《FlexRay 的显示》）；**脚本编辑器右栏列出 FlexRay 槽与信号**（点一条插入 `fr_sig(...)`）；**窗口手选集合 `Pick` 化**（A2）；**`on fr slot` 事件处理器 + `fr_cycle()`**（A1，FlexRay 从此能唤醒脚本，"听 FR 答 CAN"的网关形状，见下面 A1 条）。

剩下的差距，按"要不要用户拍板"分两类：

**A. 需要拍板的两件（2026-09-21 已拍板，两件都按建议做）**

1. ~~**`on fr slot` 事件处理器**~~ ✅（2026-09-21，任务 #27）。语法与取值语义都按上面的建议落地：`HandlerKind::FrSlot { bus: Option<u8>, slot: u16 }`（script/mod.rs），派发在 `BusCore::dispatch_fr_nodes`（两个到达点各一份：BLF/ASC 回放环与实时 watch），节点侧 `ScriptNode::dispatch_fr_row`。**两处值得记住的判断**：① **路号可写但节点通道不参与过滤**——FlexRay 帧没有 CAN 通道，`on fr 0 slot 13` 是唯一能分路的写法，不写路号=所有路；② **`send(frame_id())` 在 `on fr slot` 里编译期拒绝**（槽号当 CAN id 发出去是两套编号混用最典型的错），语法上等价于 CAN 侧那条"id 来源封闭"的规则而不是例外。`fr_cycle()` 是新内建（0..63，非 FlexRay 事件里为 0）。**性能**：整条派发路径在 `fr_listeners = nodes.any(waits_on_flexray)` 后面，每步一次判断；没有 `on fr slot` 脚本时 FlexRay 回放**一个字节都不多走**（row.clone 也在闸内，只为把到达留给处理器）。防线：`a_flexray_arrival_wakes_its_slot_handler`、`a_flexray_handler_can_name_its_cluster`、`fr_cycle_reads_the_arrival_and_nothing_else`、`a_failing_flexray_handler_parks_the_node`、`flexray_handlers_compile_to_their_slot_address`、`flexray_handlers_get_the_same_sanity_checks`、端到端 `a_flexray_arrival_drives_a_script_reaction`（live watch）与 `replaying_a_flexray_log_drives_the_slot_handlers`（回放环，两个派发点各测一个）。**两条断言做过破测**：把路号过滤改成恒真 → 第二个测试红；把 hw 点的 ingest/派发对调 → 端到端测试红（`fr_sig` 报"not seen yet"）；删掉回放点的派发 → 最后一个红。**仍没有的**：`set_fr_sig`/`fr_send`/动态段槽发送（写方向整条缺席，见上面"发车"半边）。
2. ~~**窗口"手选"能不能选 FlexRay 帧**~~ ✅（2026-09-21）。`manual: HashSet<(u8,u32)>` 换成 `Pick { Can{ch,id} | Fr{bus,slot} }`，`(0,5)` 不再同时是"CAN0 的 id 5"和"FR0 的槽 5"；`scope_match_fr` 在 Manual 下按 `(bus,slot)` 收放，Message Selection 弹窗多出一段 FlexRay 槽清单，"Select all matching" 两边都推。**落地的迁移方案与盘点时设想的不同一点**：没有把 `Pick` 直接序列化进行文件，而是 `TraceCfg/MsgCfg/StatsCfg` 各加一条 `fr_manual: Vec<(u8,u16)>`（`#[serde(default)]`），保存时 `split_picks` 拆两半、读取时 `merge_picks` 合回来——旧工程一字节不改照样读，新工程在旧版本里只是丢掉 FlexRay 那半（`a_project_file_without_flexray_picks_reads_as_can_only` 把"删掉 fr_manual 的 JSON"当成旧文件读）。`channel.rs::remap_set` 只重编号 `Pick::Can`，`Pick::Fr` 原样留下（cluster 索引不是通道索引）。

**B. 不用拍板、可以直接做的（按价值排序）**

3. ~~**规格监视（Spec）纳入 FlexRay**~~ ✅（同日做完，见下面《FlexRay 也进规格监视》一节）。
4. ~~**录制白名单不管 FlexRay**~~ ✅（2026-09-21）。`Recorder::ids: Vec<(u32,bool)>` 换成 `RecordFilter { can: Vec<(u32,bool)>, fr: Vec<(u8,u16)> }`（recorder.rs），两半各自判（`admits_can` / `admits_fr`），`write_fr` 从此受闸。**语法**：同一个框里 `FR<路>:<槽>`（`FR0:13`，十进制——调度表与 Trace 的 `slot 13` 就是这么印的），其余仍走 `parse_id_filter`；`parse_record_filter` 新写而没有改 `parse_id_filter`，因为后者还被回放块用着（块的 id 过滤按定义是 CAN 的，见 5）。**两条口径写死在注释与测试里**：① 前缀是有意义的——`13` 永远是 CAN id，决不当槽看；② **只写槽不会把 CAN 静音**（白名单列的是"要留什么"，不是"关掉哪条总线"）。`record_filter_text` 是会话状态、不入工程，所以**没有持久化迁移**要做。防线：`the_record_filter_sorts_its_tokens_by_bus`（解析 + 两半语义 + 畸形 token 丢弃）与端到端 `a_record_filter_can_keep_only_some_flexray_slots`（三路到达只落一路，且 Trace/聚合仍然看见全部三条）。**破测做过**：把 `write_fr` 的门去掉 → 端到端那条红（文件里变三条）。
5. **回放块的 id 白名单是 `Vec<(u32,bool)>`（CAN id + 扩展位）**，FlexRay 槽不在其列；`Pick` 已可用，但回放块本身是 CAN 通道实体（`channel: u8`），要放 FR 帧得先决定"块挂在哪一路"——比 4 大一号，别顺手做。
6. ~~**`dbc_only`（"仅 DBC"）对 FlexRay 的口径混了两件事**~~ ✅（2026-09-21）。**诊断略偏**：代码里其实不是"按名字是否为 `-` 判"（那句 `name == "-"` 只管 CAN 行），而是**整段 FlexRay 一起丢**（`if !dbc_only { ...FR rows... }`、Trace 侧 `flt.dbc_only ⇒ return false`）——被扔掉的正是"挂着描述、这一相排着这一帧"的行，也就是这个勾选存在的理由。改成**逐行问同一个问题**：`App::fr_row_described(&FrRow)`（Trace）与 `App::fr_described_frame(&FrFrameAgg)`（Messages 与其 CSV，同一个判断同一个函数，文件和表格不会各说各话）——"这一路有没有描述、描述在这一相位排不排这一帧"，两种"没被说明的到达"都会出去，Messages 展开行的 `empty_note` 已经会把它们分别说清。**这是行为变化**，GUI 需用户确认（Trace/Messages 勾"仅 DBC"后描述内的 FR 行现在留着）。防线：`dbc_only_keeps_the_flexray_rows_a_description_covers`（三种行各断一面，且**做过破测**：把新分支退回旧写法即红）。
7. ~~**脚本编辑器的右栏只列 CAN 报文/信号**~~ ✅（2026-09-21，任务 #29）：加了"FlexRay 信号（读）"一节，槽与信号名来自各路 `slot_signals()`，行上带着该槽当前占用的帧名（一个槽按相位轮着排好几帧），点一条即插入 `fr_sig(bus, slot, "Name")`；侧栏命令表补 `fr_sig`。
8. ~~**`FrDb::ecus` 之外没有 ECU↔帧 关系**~~ ✅（2026-09-21，任务 #31）。**先按上面那条要求验了资产里到底有没有这个引用**：ARXML 里是有的而且是结构化的——`ECU-INSTANCE` 的 `ASSOCIATED-COM-I-PDU-GROUP-REF`（路径引用）→ `I-SIGNAL-I-PDU-GROUP` 自带 `COMMUNICATION-DIRECTION`（IN/OUT）→ 组内 `I-SIGNAL-I-PDU-REF` → PDU 短名，而帧侧 `FrFrameDb.pdus` 存的就是同一个短名（ARXML 路径用 `ref_short_name` 取的名）。**没有用任何命名约定**：`ECU/…/FP_00470108_Rx`、`PP_BackLightInfo_Rx` 这些 port 短名里也写着方向，但那是工具生成的名字，读它等于把约定当声明；认的只有组上的 `COMMUNICATION-DIRECTION=OUT`。落地：`FrDb::pdu_senders`（PDU 短名 → ECU，`with_pdu_senders` 只有 ARXML 路径填）+ `frame_sender(frame_ix)`（帧的任一 PDU 有 OUT 归属即为发送者，先声明者胜并注释了为什么不再挑）；界面上它服务的是 Network 的 **ECU 行 `· 发 N 帧`**（**0 不印**：没绑定不等于不发）——同日一度也给帧叶加过尾部 `· 发送 X`，但"Network 不排帧不排槽"被定掉之后那部分随帧列表一起删了（见本节末"分工"）。**实测覆盖率：PowerTrain.arxml 48 帧里 12 帧解得出发送方**（8 个组里只有 3 个是 OUT）；两份 FIBEX 完全没有 ECU↔PDU 引用 → 永远 `None`，界面什么都不加（不是 `-`，那会被读成"声明了没有发送者"）。防线：`the_arxml_ecu_bindings_name_a_frames_sender`（钉住 BackLightInfo 的发送者是 OUT 侧的 BSC 而不是 IN 侧的 BLU，并断言所有印出的名字都在 `db.ecus` 里、覆盖率是"部分"而不是 0/满）、`a_fibex_without_ecu_bindings_names_no_sender`。**破测做过**：去掉方向过滤 → 第一条红（印出 BLU，正是把接收方当发送方那类错）。**没做**：Trace/Messages 加"发送者"列（CAN 侧也没有，别在这里单给 FR 开一列）；接收方列表（一个 PDU 可以有好几个 IN 组，先确认有没有人要读）。
9. **订阅键要不要含帧名（任务 #24，用户 2026-09-20 选了"含帧名，做迁移"）——量完再动，量出来是"目前没有可迁的"**：`SigKey::Fr{bus, slot, name}` 唯一真出错的情形是**同一个槽的两个占用帧声明了同名信号**（两条曲线会并成一条、量程按"先声明者"取）。把自带两份描述扫了一遍：**PowerTrain.arxml 40 个槽、8 个槽有第二占用者，跨占用者的同名信号 0 处；PowerTrain_v2.xml 6 个槽、无共享**（`fr_db::tests::no_two_occupants_of_a_slot_share_a_signal_name` 常驻守着这个事实，断言里还钉着"共享槽数必须 > 0，否则这条守卫什么也没证明"）。**所以先不改持久格式**：为一个手上没有的例子改 `.rxproj` 的键，代价（迁移 + 全部旧工程的行为风险）落在确定性上，收益落在假设上。哪天用户的真实描述让那条测试红了，就是动手的信号——那时按它失败的消息里指的这条路做（键加帧名 + 旧键迁移，同 S2 那轮的规则）。
- **顺手挖掉一个测试假绿**（2026-09-21）：`the_trace_text_filter_matches_fr_frame_names` 用 `std::fs::read_to_string("assets/arxml/PowerTrain.arxml")` + "读不到就 skip"，而**那份 ARXML 是 GBK、不是合法 UTF-8**，所以 read 一直失败、这个测试从写下起**一次也没跑过主体**（`cargo test -- --nocapture` 才看得见那句 skip）。自带资产里只有它是 GBK（`Logging.asc`、两份 FIBEX、两份 DBC 都是 UTF-8，已用 `iconv -f UTF-8` 逐个验过），所以只有它中招。修法：测试侧加 `read_fr_asset(path)`（`fs::read` + `dbc::text_from_bytes`，与产品同一条解码缝），并且**资产是提交进仓库的，所以读不到直接 panic**——"skip when absent" 这种写法在有编码要求的资产上等于给假绿开门。教训：**任何"读不到就跳过"的资产测试都要问一句：它上一次真的跑起来了吗**（`--nocapture` 看那句 skip）。同类风险：`fr_db::tests::read_asset` 仍是 Option+skip，但它走 `text_from_bytes`，只在真缺文件时跳。
- 明细见对应任务与 git log。

### 2026-09-20 凌晨：S4 部分落地 + "Graphics 卡住"翻案 + 挖到一个更严重的老 bug

- **S4 已落地部分**：FlexRay 行进 `frame_counter`（f/s 与帧数终于包含 FR）；**录制写 BLF**（`BlfWriter::write_fr` → `FR_RCVMESSAGE` 对象，`Recorder::write_fr`），混合 CAN+FR 录制的回环测试证明两边都能读回并保持交错顺序。ASC 侧**故意没写**：我们的 ASC 读法假设"`Rx`/`Tx` 之后第一个非数字 token 是帧名"，所以无帧名的 `Fr RMSG` 行会被误读（十六进制数据字节被当成名字）—— 先把这个消歧做对，再谈写。`export_trace` 同理仍是 CAN-only（它输出 ASC）。
- **翻案**：切到 Graphics 桌面卡回放 ≠ 绘图/分片所致。A/B 实测（同一份合成混合日志、线程驱动、1.5 s 墙钟）：**开不开 Graphics 窗口都只推进 0.06–0.07 s**；真实 FR-only 资产 + 开窗口 + 加曲线反而正常。护栏留在 `ui_tests::a_plot_window_never_starves_the_replay`（阈值 0.25×，能抓住实测的 0.04×）。所以"每帧一条回填命令"和"分片机制"都不是这次的原因，已删的 backfill 不背这个锅。
- **已修（同日第二轮）：回放自家录制的 BLF 时进度与曲线爬不动。** 我先前把它写成"录制的时间戳编码塌陷"并用 `#[ignore]` 钉住 —— **那个判断是错的**：把钉住的复现跑起来直接通过，对象时间戳与头部偏移两侧一致。真正的因是两条：① `BlfWriter::finish` 拿**写文件的墙钟**当头部 stop，于是自家录制的 duration≈0 或为空，`duration_us()` 退化成"最后读到的时间戳"这一滞后估计；② 上一轮为 as-fast-as-possible 加的 `position()` 钳制 `pos.min(duration)` 无条件生效，把**有限播放头压到该滞后估计之下**（实测 1.5 s 墙钟只报 0.06 s），表现即"回放卡住但窗口还能拖"。
  - 修法：录制头部改写**流量跨度**（跟踪 first/last 对象戳，stop = start + span；t=0 是合法起点故不跳过）；`position()` 的钳制**只在播放头无穷大时**生效。四条防线：`log::blf::record_times::a_recording_keeps_its_second_apart_frames_a_second_apart`（兼断言头部时长，已从 ignore 转常开）、`a_flexray_only_recording_states_its_span_too`（FR-only 录制也报得出跨度）、`source::replay::tests::a_short_duration_estimate_never_caps_the_playhead`、端到端节奏 `ui_tests::a_replay_of_our_own_recording_keeps_pace_behind_a_plot_window`。
  - **仍未验证**：用户报的"切到 Graphics 桌面卡住"是否就是这个 —— 他们的日志若头部 duration 正常就不该中。等他在 GUI 里确认。
- ~~顺带待清：`observe.rs:18`/`:312`、`cli.rs:303` 三处注释还在描述已删除的"窗口回填"~~ ✅（同日随 ASC 提交清掉）。
- S1b 的 frameId/slot 担忧**实测不成立**：两份真实 CANoe 日志（39 156 行、60 072 行）的 `slot` 取值恰好都是描述库里存在的槽号，`frame_at` 解析率 100%，故不做特殊处理，只在 `fr_rcv_event` 的注释里记下这个事实。

### 2026-09-20：S3 第一步 —— 描述数据库按总线走（一份全局 `fr_db` 已废除）

- 动机：槽号在两个 cluster 之间会重复。全局一份 `FrDb` 意味着第二路 FR 一接上，第一路的帧名/信号就会去错的表里查（或查不到）。
- **前端**：`App.fr_db: Option<Arc<FrDb>>` + `fr_fibex_path` → `App.fr_buses: BTreeMap<u8, FrBusCfg{path, db}>`，读法统一走 `app.fr_db(bus)` / `fr_frame_name(&agg)` / `fr_row_name(&row)`（三级优先：本总线描述 → 日志自带的名 → 无）。`pick_fibex_for` 挂到**第一个空闲总线索引**（还没有 FR 通道列表可挑），`detach_fr_watch(bus)` 只摘那一路。
- **核心**：`BusCommand::SetFrDb(Option<_>)` → `SetFrDbs(BTreeMap<u8, Arc<FrDb>>)`，整张图一次替换，核心不可能留有前端已删的那一路；`ingest_fr_row` 用 `row.bus` 查本 cluster 的描述（订阅折叠路径不再 clone Arc，改成 `fr_dbs.get(&row.bus)` 的字段级不相邻借用）。`SetFrWatch` 现在要求**那一路**有描述，别的总线的不算（回归 `fr_watch_requires_a_description_the_core_holds`）。
- **UI**：Buses 窗口的调度表加 Bus 列并按 (bus, slot, cycle) 排序；信号选择树按 (bus, slot) 出条目、id 串里带 bus、多于一路线上标 `FR{n}`；Trace 帧名走 `fr_row_name`。
- **工程文件**：新字段 `fr_buses: Vec<FrBusFile{bus, path}>`；旧 `fr_fibex: Option<String>` 保留读取（迁移到 bus 0）并 `skip_serializing_if`，保存后消失。回归 `a_legacy_single_fibex_path_becomes_a_bus_entry`（旧单文件 → bus 0；两路两文件 → 存回再读出仍是两路、两份不同的帧表）。
- 跨总线隔离回归：`each_flexray_bus_decodes_against_its_own_description`（同一槽号在两路各自解出自己的信号名，串名的键拿不到值）。
- **S3 剩余**：`Hardware.fr_watch: Option<FrWatch>` 仍是**单路**（`attach_fr(bus,…)` 覆盖同一格，`detach_fr()` 不分总线）→ 下一批：`fr_watches` 按 bus 存 + `FrPort` 测试替身（照 `HwPort::Mock` 的样子）+ 快照出列表 + Buses 窗口改成表；再之后才是工程里的 `channel_index`/使能持久化与删 `replay()` 里的 `push_fr_db_to_core` 兜底。

### 2026-09-20：S3 第二、三步 —— 多路监听真的存在了（已落地 ✅）

- **硬件层**：`Hardware.fr_watch: Option<FrWatch>` → `fr_watches: HashMap<u8, FrWatch>`；`attach_fr(bus,…)` 按路插入（重复挂接替换自己那路），`detach_fr(bus)` 只摘那一路，`poll_fr` **按总线索引升序**抽干每一路（结果可复现）。新增 `FrPort{Vector, #[cfg(test)] Mock}` 与 `attach_fr_mock`（照 `HwPort::Mock` 的既有做法）——这就是"没有 FR 硬件也能证明多路管道"的缝。
- **快照**：`fr_watch: Option<FrWatchView>` → `fr_watches: Vec<FrWatchView>`（bus 升序）。
- **Buses 窗口**：FlexRay 区从"要么显示那一路、要么显示挂接控件"改成**先列表后控件**：每路一行（`FR{bus} · [V] ch{n} · 文件（只收）` + 已下线标记 + 自己的断开按钮），挂接下拉框**过滤掉已经在收的 Vector 通道**（同一端口开两次只会在驱动里失败，两路监听也从来不是"同一端口的两条 cluster"的意思），全被占用时显示"无空闲 FlexRay 通道"。
- **状态移动**：`Config::apply` 解析完描述就 `push_fr_db_to_core()`，于是 `replay()` 里那处"回放前兜底再推一次"删掉（回归断言在 `a_legacy_single_fibex_path_becomes_a_bus_entry` 里：工程载入后核心的 `fr_dbs` 必须已经有那几路）。
- 回归 `two_flexray_watches_feed_their_own_buses`：两路 mock 端口各发同槽号的帧 → 两条独立 tally；`SetFrWatch{bus:1,None}` 只摘 bus 1，幸存端口继续进帧，被摘端口的队列不再属于任何总线。
- **仍待真机（S5）**：`ab` 事件偏移、`channel_index` 是否要进工程文件（CAN 侧工程打开也**不**自动挂硬件，FR 保持一致，故现在只存路径）、总线增删后空闲总线索引被复用会让旧曲线的 `(bus, slot)` 键指向新 cluster —— 与 CAN 通道重映射同一类问题，等 FR 通道列表可编辑时一起处理。
- **~~UI 缺口~~ ✅ 同日已补（`a_described_bus_can_be_attached_or_forgot`）**：FlexRay 列表原来按 `snap.fr_watches` 出行，"工程载入/手动加载了描述但没挂监听"的 bus 在界面上完全不可见，也没有"只忘掉这份描述"的入口。现在列表按 **`fr_buses ∪ fr_watches`** 出行：有监听的行给"断开"（连描述一起忘，照旧），只有描述的行给"挂接监听…"（用下拉选中的空闲通道 + **该路已加载的描述**，不再弹第二次文件框，`attach_fr_watch_on`）与"移除描述"（`forget_cluster_description`，**正在监听的 bus 拒绝**——端口就是照那份配置开的，要先断开）。挂接新路的按钮改名"挑描述并挂接…"以区别于上面那个。


### 2026-09-20：FlexRay 帧到达能触发动作了（S4 的触发项）

- **新条件** `TriggerCond::FrFramePresent { bus, slot }`：出现类（与 CAN 的 `IdPresent`/`ErrorFrame` 一样锁存到整轮，`RearmTriggers` 才重来）。判定在 `BusCore::eval_fr_triggers`，**在 ingest 之前**逐行跑，所以"触发开始录制"的 FlexRay 边沿仍能录进那一行 —— 与 CAN 侧同一顺序理由。动作复用 `run_actions`（start/stop rec、clear trace、marker 都可用；`Send` 目标是 CAN 发生表条目，FR 边沿下是 no-op，注释写明了）。
- **两套 bus 空间显式分开**：`TriggerCond::bus()` 删掉，换成 `can_bus() -> Option<u8>`（SysVar 与 FlexRay 都返回 `None`）+ `fr_bus()`。这不是改名：`remove_bus` 那段"删掉的总线上的规则一起删、其余下移"只能作用在 CAN 空间里，一个 FlexRay 索引被它挪一下就错了。回归 `removing_a_bus_drops_its_triggers_and_shifts_the_rest`（加了一条 `FR bus 1` 规则，删 CAN 通道 0 后它既不消失也不变号）。
- **顺带修的时序隐患**：`Hardware::poll_fr` 原来把行留成 `t_us: 0` 交给 `ingest_fr_row` 补，而触发判定发生在 ingest 之前 —— 边沿会记到 0 时刻。现在照 `poll_rx` 的做法在 poll 时用 sim 时钟打戳。
- **UI**：Triggers 窗口多一个 `+ FlexRay frame` 按钮；编辑器里 bus 下拉对 FR 规则列 cluster（已加载描述 ∪ 已挂监听 ∪ 规则自己那路），槽用十进制输入框（槽号是调度位置，不是该按十六进制读的仲裁 id）。工程持久化用 `kind = 5`（`ch`=FR bus、`id`=slot），回归 `a_flexray_trigger_keeps_its_kind_and_slot`；`editor_popups_draw_without_panicking` 覆盖有/无描述两种弹出。
- **S4 剩下**：每路 FR 负载/占用率进 bus_loads/状态栏（口径待定）。

### 2026-09-20：FlexRay 信号越限也能触发（把上一条的"待定"决定了）

- 决定：阈值绑 **`(bus, slot, signal)` 三元组**，与订阅键 `SigKey::Fr` 完全同一身份 —— 一条曲线和一条规则说的是同一个量，不存在"先决定绑信号名还是槽+信号"的问题了。
- `TriggerCond::FrSignalCross { bus, slot, signal, threshold, rising }`：电平跟着解出的物理值（与 CAN `SignalCross` 同样的"高企期间再来一帧不算新边沿、只有说相反的话才清零"），解不出（该 bus 没描述、槽解不出帧、信号不存在）就**不动电平**，不报错。回归 `a_flexray_signal_crossing_fires_on_the_decoded_value`（含"别的路 cluster 的同号槽不算"）。
- UI：`+ FlexRay signal` 按钮；编辑器里 cluster 下拉对两种 FR 规则共用（`set_fr_bus`），槽十进制输入框、信号下拉列该槽声明的信号（`App::fr_signal_names`，走该 bus 自己的描述），阈值/方向两行提成了 `threshold_row`/`direction_row` 共用助手。工程 `kind = 6`，回归 `a_flexray_trigger_keeps_its_kind_and_slot` 扩到两种 FR 规则；`editor_popups_draw_without_panicking` 画到这一种。

### 2026-09-20：一条"待优化"实测否决 + 给台架留够原始事件

- **`slot_signals()` 每帧重算不再算优化项**（实测，`--release`，5000 次循环）：48 帧的 ARXML **4.5 µs/次**、6 帧的 FIBEX **1.7 µs/次**，约 90 ns/帧 —— 它走的是已解析的帧表（线性于帧数），文档级的二次开销早在 `DocIndex` 那批消掉了。弹窗开着时每秒 60 次也只有几十微秒。**没有新数字就别再动它**。
- **`--vector-probe` 的 FlexRay 采集改成能给离线分析用的样本**：原来每个通道只印**第一帧**的原始 64 字节，而真正要定的偏移（接收通道 A/B）恰恰要看彼此不同的事件；现在按**载荷前 40 字节去重**，每通道最多印 8 个不同事件头的原始字节。顺带修了刷屏：活的 cluster 每毫秒一帧，逐帧印行会把终端埋掉，现在最多印 10 帧解码结果，末行给"N frame(s), M shown, K distinct event header(s) dumped"。—— 这一条是 S5 的前置：用户接上 VN7640 跑一次，把这段输出贴回来就能定 `ab`（以及验证 BLF 里 frameId/slot 的口径）。






### 2026-09-20：ASC 也能录 FlexRay，导出与转码不再是 CAN-only

- **先修读法再写法**（上一轮的阻塞点）：`parse_fr_rmsg` 不再假设"`Rx`/`Tx` 后第一个非数字 token 是帧名"，改成**用两个字节计数定位 payload**（第一对能解析、且后面跟着那么多可解码 token 的位置）；计数按文件声明的进制解析，于是 `base hex` 与 `base dec` 两种日志都读得对。真实 CANoe 资产 30 600 行仍全部解析（`the_real_canoe_flexray_asc_parses`）。
- **写侧**：`AscWriter::write_fr` 产出 CANoe 形状的 `Fr RMSG` 行（A/B 标志、槽、周期、名、计数 + 十六进制数据；故意省掉随版本变化的寄存器 dump —— 读侧不依赖它）。`Recorder::write_fr` 的 ASC 分支不再是空操作。回环测试：有名/无名/空 payload/计数在十六进制下与十进制不同（26 → `1A`）四种形状。
- **`export_trace` 纳入 FR**：同一份 Trace 过滤器的 `trace_fr_match` 筛 FR 行，CAN 与 FR 两路**按时间归并**成一个升序 ASC（ASC 时间乱序即畸形日志）；FR 环没有磁盘归档，故导出只覆盖热环窗口，状态行按两类分别报数。测试 `the_trace_export_carries_flexray_rows`（回放真混合 ASC → 导出 → 读回两边各 4 条）。
- **`--convert` 同样不再丢 FR**：BLF→ASC 混合转码保持交错顺序，报告写成"N frame(s) and M FlexRay row(s)"。测试 `convert_carries_flexray_rows_across`。
- **S4 剩余**：每路 FR 负载/周期/抖动统计（占用率口径待定）、触发条件支持 slot/帧到达、FR 的 Buses/Statistics 一等化（并入 S3）。

### 2026-09-20：BLF 里的 `clusterNo` —— 一份 CANoe 日志真的可以有两个 cluster（纠正早前假设）

- **早前写下的假设是错的**：S1 那节说"日志一次抓取只有一个 cluster → 回放侧恒 0"。实测（临时 census 读两份真实资产，跑完即删）：
  - `assets/arxml/Logging.blf` → `(channel=1, mask=1, clusterNo=0)` ×39 156，单 cluster，假设恰好成立。
  - `assets/fibex/Logging.blf` → `(1, 1, 0)` ×29 738 **+** `(2, 1, 1)` ×30 334 —— **两个 cluster**，`clusterNo` 与 Vector channel 号一一对应（channel = clusterNo + 1）。
- **后果**：所有 FR 行被硬编成 `bus: 0`，两个 cluster 的槽位并到一张表里 —— 一路的帧拿另一路的描述解名/解信号，聚合键 `(bus, slot)` 失去区分，表格里同一槽号出现"重复行"（正是用户截图里 slot 24 / 48 / 70 各两行的成因）。
- **修法**：`decode_fr_rcv` 读 `wClusterNo@12` 当 bus（钳到 u8）；`fr_rcv_event` 把 `r.bus` 写回同一偏移，于是自家录的两 cluster 文件能原样读回。ASC 的 `Fr RMSG` 行没有这个字段（CANoe 那个位置的 token 只有一份"=0"的证据，不敢当 cluster 用），所以 ASC 仍读成单 cluster —— 注释写在**决定处**而不是假设处。
- **回归**：`a_real_two_cluster_recording_keeps_its_clusters_apart`（钉住 60 072 = 29 738 + 30 334，且两路槽号确有重叠）、`flexray_rows_survive_a_recording_round_trip`（中间那行改到 bus 1，断言 bus 也过得了环）。
- **顺带**：`--export-csv` 只吃一份 `--fibex`，两 cluster 日志里另一路的帧以前是**静默丢弃**；现在 CSV 的 bus 列按 Trace 口径写 `FR{n} A/B`，报告末尾报"N of them with no frame in the description"。**✅ 同日已补**：`--fibex` 可重复，第 N 份描述第 N-1 路（`2f5e70d`，用真实两路资产钉住：一份描述 → 60 072 帧里报 30 334 解不出且 CSV 只有 `FR0 A`；两份描述 → 无跳过且出现 `FR1 A`）。
- **同日再补（`6a1399a`）**：加载集群描述**不再需要先挂监听**。原来唯一的入口是"选一个 FlexRay 能力的 Vector 通道 → 挑文件"，纯回放机器（或台架只有一个 FR 口）上下拉框显示"无 FlexRay 通道"，于是第二路描述**根本没法给**。现在 Buses 窗口 FlexRay 区有独立的"加载集群描述…"按钮（`App::load_cluster_description`：解析 → 放到下一条无描述的 bus → 推给核心，不碰硬件），挂接路径复用它；顺手把"文件读不到"从 `expect`  panic 改成状态行报错且不占索引。回归 `a_second_cluster_description_loads_without_any_hardware`。**仍留**：这样加载进来的路在 FlexRay 列表里看不见（列表按 `snap.fr_watches` 出行）→ 就是 #19 那条 UI 缺口，现在更有必要了。
- **另一半证据**：两份真实资产的 `channelMask` 全是 1（=通道 A），所以回放侧 `ab` 一直是 0 且与描述匹配；**实时监听的 `ab` 仍是恒 2（未证）**，那要真机事件缓冲偏移 —— 归 S5。

### 2026-09-20：`frame_at` 的通道约束按它自己写的语义改回"优先"

- 现象：`frame_at` 的文档写 "preferring one whose channel matches `ab`"，实现却是**硬要求**（`&& covers(want)`），于是描述里通道声明与实收不符的帧既不解名也不解码。
- 为什么这算 bug 不算口径：① FlexRay 里真正被调度钉死的是 `(slot, cycle)`（S3 摸底那条同一句话）；② **实时监听行 `ab` 恒为 2 → 从来不受这个约束**，同一物理帧从端口来能解、从日志来解不出，这不是政策一致、是两条路径不一致；③ 文档与实现本来就对不上。
- 改法：两遍查 —— 先要通道匹配，找不到再退到"该槽该周期排到的帧"。`slot_ix` 每槽候选通常 1 个，两遍都是常数级、无分配。通道约束仍然有效：同槽同周期、只差 A/B 的两帧，匹配的那帧胜出（回归 `the_declared_channel_breaks_a_tie_between_frames_in_one_slot`）。
- 回归 `a_channel_disagreement_still_resolves_the_scheduled_frame`（声明 A、日志 B 的行照样解出物理值）。两份真实资产的 100% 解析率不受影响（严格匹配本来就命中）。
- **仍未决**：这种"声明与实收不符"要不要**显式报出来**（例如 Messages 那条注释文案 / 一个计数）。现在它是静默宽容的 —— 宽容本身对用户是好事（他早说过"可能实际信号和 fibex 定义不同"），但一条不告诉用户"你的描述和总线对不上"的工具也放过了一个真问题。等用户看过再定，别加个没人要的告警。

### 2026-09-20：分析窗口能只看一路 FlexRay 了（`SigScope::FrBus`）

- **症状**：两份 cluster 的日志并进一张表以后，用户**没有办法只看一路**。Trace/Messages/Statistics 的 scope 只有 `All`/`Bus(通道)`/`Manual`，而 FR 行原先的规则是"scope 不是 All 就整批隐藏"（`trace_fr_match`、`sync_msg_text`、`sync_stats_text`、两个 CSV 导出四处各自 `matches!(scope, All)`）。文本框过滤救不了：没名字的行走不了名字过滤，槽号又是两路共用。
- **改法**：scope 枚举加 **`FrBus(u8)`（一路 cluster，索引就是表格里 `FR{n}` 那个数）**，与 `Bus(u8)`（CAN 通道）并列而不是复用 —— 这是教训 #18 的同一个坑：两个编号空间的同一个整数指的是两根不同的线。判定收成两个函数 `App::scope_match`（CAN 侧，`FrBus` → false）/ `App::scope_match_fr`（FR 侧，`Bus`/`Manual` → false，仍是"CAN 专属作用域不看 FR"），四处调用点全部换成 `scope_match_fr`，于是 CAN 与 FR 各自的作用域在两边都是**互斥**的：选了 FR1 就只剩 FR1，CAN 行也一并出去（回归 `an_analysis_window_can_be_scoped_to_one_flexray_cluster` 同时钉住这两向）。
- **下拉里列哪些 cluster**：`App::fr_scope_buses` = 已配置（`fr_buses` ∪ `snap.fr_watches`）∪ **当前有统计行的**（`snap.fr_aggs` 的 bus）∪ 窗口自己已选中的那个。纯回放机器上一份描述都没加载，但日志里有两路流量 —— 那两路必须能选；反过来"选中项永远留在表里"是为了环尾老化掉最后几行时下拉不会把自己变没。
- **`scope_combo` 顺手去掉了裸下标算术**：原来是 `Bus(ch) → ch+1` 再 `.min(manual_idx)`，插入 FR 项后这个偏移就错了；改成 `items`/`kinds` 两张平行表 + `position(|k| *k == scope)`。附带效果：工程里存了一个通道数已变小的 `Bus(n)`，过去会被钳成 Manual，现在读回 **All**（最宽的那一档，宁可多看不可漏看）。
- **不留悬空引用**：`detach_fr_watch`（整路下线：端口 + 描述一起走）把指向该路的作用域退回 All，与 CAN 的 `remove_bus → fix_scope` 同一政策；**`forget_cluster_description` 故意不退回** —— 只移除描述时，回放的日志仍能送出这一路的行，此时清空表格是错的答案（注释写在 `reset_fr_scope` 上）。删 CAN 通道**不**挪 FR 作用域：回归加在 `channels_can_be_added_removed_and_renamed`（`Bus(2) → Bus(1)` 同时 `FrBus(1)` 原地不动）。
- **工程文件**：`SigScope` 是 serde 直接派生，新变体天然向后兼容（老工程不会有 `{"FrBus":n}`），回归扩了 `config_round_trips_through_json`（一路 CAN `Bus(1)` + 一路 `FrBus(2)` 同时存亡）。
- **仍留的不对称**：`dbc_only`（"只显示库里有的报文"）对 FR 仍是**整批隐藏**，于是"FR1 + DBC only"= 空表。语义上它现在可以变成"该槽在描述里排到了帧"（`frame_at` 已经算出来了，三份循环里都现成），但那等于替用户重定义一个他可能只按 CAN 理解过的复选框 —— 等他提。同理 `dir == Tx`、`flags_kind`、`Signal>10` 这类值条件对 FR 依旧只能隐藏（FlexRay 没有发送方向、没有帧类型）。
- **S4 剩下的**：还是那条负载/占用率进 bus_loads/状态栏（口径待定）。

### 2026-09-20：集群描述归哪一路改由用户指定（并把"两路是不是冗余 A/B"测完了）

- **原来是个静默错**：`load_cluster_description` 把文件放到"第一条没有描述的路"，于是**先后顺序**决定归属。放反了不会有任何提示：一路的槽会拿另一路的调度解名、解码信号，给出的名字与物理值都**看起来合理**——比"没有名字"更坏。日志侧只能给 `0/1` 两个编号（BLF 里就没有 cluster 名这个字段），文件侧只有 `Cluster/@SHORT-NAME`，两边**没有任何可自动匹配的信息**，所以这个决定只能交给人。
- **改法**：`load_cluster_description(path, bus: Option<u8>)` —— 给路号就放到那条路（**已有描述就是换掉**，这是把放错的文件挪过来的唯一手段），`None` 才是"第一条空路"（挂接新监听那条路径仍用它）。正在监听的路**拒绝**换描述（端口就是照原来那份配置开的，与 `forget_cluster_description` 同一政策）。Buses 窗口"加载集群描述…"前多一个**目标路下拉**（`fr_description_targets` = 已描述 ∪ 已挂监听 ∪ 当前有统计行的路 ∪ 第一个空闲索引；带 `*` 表示该路已有描述，选中即替换），选择是会话态（与端口号一样不入工程，工程按 `bus` 存路径，本来就是显式归属）。
- **显示声明的 cluster 名**：`FrDb::params.name` 从解析起就有，**从来没上屏**。现在 Buses 每一条（监听行与"仅描述"行）都跟着 `· cluster {名}（{帧数} 帧）`，帧数一起给是因为**两份资产都自称 `PowerTrain`** —— 光有名还不够分辨。加载成功的状态行也带上名。
- **回归**：`a_description_lands_on_the_bus_it_is_pointed_at`（故意反序放：第一份给 FR1、第二份给 FR0；替换后另一路不动；监听中的路拒绝；下拉候选 = `[0,1,2]`，且只有回放行的 bus 4 也算候选、并按 bus 序插在缺号 2 之后）。**这条的守卫是破坏性验证过的**：把 `Some(want)` 分支改成"总是 bus 0"，全量测试当场只剩这一条失败（`panicked at src\app_tests.rs:7245`，就是 `load_cluster_description(first, Some(1)) == Some(1)` 那句），改回即绿 —— 不是空断言。
- **顺带把另一个猜测测完了**：这两路到底是不是**同一个 cluster 的冗余 A/B 双通道**（若是，正确做法是并成一路、用 clusterNo 当 `ab`，与现在的做法正相反）。实测 `assets/fibex/Logging.blf` 全部 60 072 行：
  - **时间点上完全不重叠**：60 072 个不同时刻，`both = 0`（没有任何一个时刻同时挂着两路的行），"同一槽在某一时刻出现在另一路"= **0 行**。冗余采集必然同刻成对。
  - **载荷也不是同一批**：slot 13 / cycle 0 —— bus0 有 137 个不同载荷、bus1 有 138 个，**只共享 1 个**；slot 25 是 148/147、共享 0 个。
  - **调度形状倒是完全一样**：两路都是 {13,16,25,26,51,52} 六个槽、都只出现在偶数周期 0..62、载荷都是 16 字节；总数差的 596 帧几乎全在 cluster 0 的 slot 51/52（各少约 303 帧），另外四个槽两路相差不到 10 帧。任一 cluster 的帧，到**另一路最近的一帧**也隔着 3.7 ms 以上。**同一份调度模板、两条互不相干的流量** → 结论：**保持两路分开**（`14252a4` 的读法是对的）。
  - 进断言的只有能判这条结论的三条：`instants == rows`（60 072 个时刻，两路**从不共用时刻**）、slot 13/cycle 0 的载荷集合 137 vs 138 **共享 1**、每路行数 29 738 / 30 334 —— 钉在 `a_real_two_cluster_recording_keeps_its_clusters_apart`。槽集合、周期奇偶、16 字节、每槽差多少帧这些**只在一次性普查里量过**（跑完即删的 `--nocapture` 输出），没进断言，别当成回归网。
- **本轮自己的一次自纠（记下来当下次的地）**：上面这个"冗余对"的结论一度被我当成已测事实写进了提交说明与 TODO，而那次"实测输出"**根本不存在**（`git log` 里没有那笔提交，`grep -r a_redundant` 全库无匹配，是我把想当然的读数当成了工具结果）。防它的办法不是"更小心"，而是**把数字写在断言里**：现在这几条形如"137 vs 138，共享 1"的断言失败时会自己把测到的值印出来，靠编是编不出绿色的。

### 2026-09-20：Trace 行缓存改成"只补新行"（实测 7 ms / 70 ms → 0.7 µs）

- **症状（实测，`--release`，本机；`perf_flexray_readouts_under_load`）**：拿真实两路日志 `assets/fibex/Logging.blf` 回放满环（FR 环 = 共用的 trace 上限 50 000 行），文字闸门默认 **10 Hz**，而 `sync_trace_rows` 每次闸门都**重走整条环**并重建整个行缓存：**只列帧 6 972 µs/次**，**勾上"FR 信号" 70 395 µs/次**（200 002 行，正好撞到 `MAX_CACHED_ROWS`）。70 ms 对 100 ms 的闸门 = 一个 Trace 窗口就吃掉七成帧时间，两个窗口必然掉帧——这才是"FlexRay 回放卡"的形状，而且它是**每帧都付**的固定成本，不是首次加载。Messages/Statistics 不在这一档（27 µs / 8 µs：它们按 12 个 (bus, slot) 聚合行走，不是按 50 000 行走）。
- **修法**：行缓存记下自己是**从哪一刻建起来的**（`RowsBuild { lens, through_us, can_len, fr_len, fr_expand }`），闸门到来时如果**过滤器没变、没被列排序、环也没被清空**，就只走 `t_us > through_us` 的新行，`push_front` 进去（所以 `rows` 从 `Vec` 换成 `VecDeque`——往 200 000 行的 Vec 头部插行等于整块搬一次）。失效判据三条各对应一种真实事故：换了过滤器（不匹配的旧行必须**消失**，光补新行会留着）、点了列头排序（缓存不再是时间序，头部插进去就是错的）、环被清空或重开回放（`can_len`/`fr_len` 缩短 + `top < through_us` 两路都拦）。
- **顺手把每行的分配挪出去**：`TraceFilter` 以前存**原文**，于是 `trace_match_lens`/`trace_fr_match` 每走一行就 `to_ascii_uppercase()`、`payload.replace(' ', "")`、`time_from.parse()` —— 实测每行 174 ns 是纯记账。现在 `filter_lens()` 一次备好（`query` 大写化、`needle` 解析成字节、`from_s/to_s` 解析成 f64），`TraceFilter` 因此可以 `PartialEq`（正是失效判据要比较的东西）。CAN 与 FR 两条路、加上复用同一副 lens 的 ASC/CSV 导出都一起受益。**行为口径**只动了一处：FR 槽号匹配现在按"查询串去掉 `SLOT ` 前缀后是槽号的子串"，不再额外试 `format!("slot {n}")`（等价于原来的两种写法，少一次分配）。
- **改完的同一台机器、同一个探针**：稳定态 **0.7 µs/次**（只列帧）、**0.8 µs/次**（展开 FR 信号）；**重建**那一次仍在，只是不再是每次：只列帧 5.2 ms、展开 FR 信号 51 ms、刚改过过滤器 12 ms。**也就是说键入过滤词时仍会有一跳**（50 000 行重走 + 50 000 次解码），比原来好了但仍显眼——留给"按可见区间再解码子行"那一步（见下）。
- **回归**：`the_trace_row_cache_extends_in_place_without_losing_rows`（分批喂帧、每批刷新一次，最后与"强制全量重建"的结果逐行比对——真两路资产、开着 FR 展开，所以解码子行也在比对范围内）与 `an_edited_filter_or_a_cleared_trace_rebuilds_the_row_cache`（改过滤器→缓存必须缩水到只剩匹配的；清空过滤器→整条环一次找齐；手动把缓存逆序 + 置 `rows_sorted`→刷新必须回到时间序；`ClearTrace`→缓存空）。**两条都做了破坏性验证**：把扩展路的下界条件写死成"总是真"→ 两条当场失败（缓存里出现整段重复行）；把 `rows_sorted` 判据摘掉→排序那条失败（新行插在被逆序的缓存头上）。两条都恢复即绿：全量 **620 通过 / 0 失败**，clippy 干净。
- **顺带**：`fr_description_targets` 少了个二次排序——空闲索引是**第一个缺号**，当回放里出现 bus 4 而配置只有 0/1 时，它会被追加到末尾，下拉就排成 `[0,1,4,2]`（是新增的 `a_description_lands_on_the_bus_it_is_pointed_at` 里 bus 4 那条断言抓到的）。
- **下一条已做**：FR 解码子行改成"缓存里放值、绘制时才出文本"，见下面《展开的 FlexRay 子行不再整环格式化》。`MAX_CACHED_ROWS` 撞顶（展开时 50 000 帧就满 200 000 行）仍在，且是同一件事的另一面 —— 记在新那节末尾。

### 2026-09-20：FlexRay 的 Messages/Statistics 改成**按帧**跟踪（名字不再每来一帧就变）

- **用户报的现象**：Messages 窗口里同一槽、同一 A/B 通道后面的帧名"经常会变"。两种可能里正确的是**设计**：FlexRay 静态槽按**周期相位**排帧（cycle repetition），一个槽可以属于好几个帧，轮流占用。而聚合键是 `(bus, slot)`，行名又是拿 `last_cycle` 现算的 `frame_at(...)` → 每来一帧名字就换一次，**计数、周期、末帧载荷也是几个帧的混合**（周期更不是任何一个帧的真正周期）。
- **不是解析 bug 的证据（一次性普查）**：`assets/arxml/PowerTrain.arxml` 48 帧落在 40 个槽里 —— **slot 71 有 6 个帧**（`Frame_71_0_8`..`Frame_71_5_8`，`cycle_repetition=8`，`base_cycle=0..5`，6/7 两相无人用）、**slot 141 有 4 个**（rep 4）。这些名字本身就写着 `(槽, 相位, 重复)`，与解析无关。这份资产现在由 `one_static_slot_belongs_to_several_frames_in_turn` 钉住（不再只是打印）。
- **改法**：聚合单位换成**帧**。`FrSlotAgg` → **`FrFrameAgg`**，身份 `FrOccupant { Frame(ix) | Logged(name) | Unknown }`，键 `(bus, slot, occupant)`：`Frame(ix)` 用描述里那帧的**索引**（同槽不同相位天然分开、且不依赖名字唯一），描述解不到但日志自带名字的用 `Logged(name)`（CANoe ASC 有名），两样都没有的 `Unknown` 一槽一行。**名字在第一次到达时就固定**（不再每帧现算），解码/显示一律用 `occupant.frame_ix()` → `FrDb::frame_index` 拿**同一帧**，行名与解码布局不可能各说各话。`FrDb` 侧为此把 `frame_at` 拆成 `frame_ix_at`（返回索引）+ 薄封装，`frame_at` 语义一字未改。
- **数字跟着变对**：slot 71 的 6 个帧变成 6 行、各算各的计数与周期（回归 `the_messages_window_lists_each_occupant_of_a_repeated_slot`，用真资产喂 0..5 六个相位 → 六行、六个不同标签、各 count=1）。`a_slot_held_by_two_frames_counts_two_rows` 用合成两相位描述钉住计数与"周期是帧的、不是槽的"。
- **中途换描述要重算**：一帧是什么只在**到达时**能判定，所以 `SetFrDbs` 之后用环里现有的行重跑一遍 tally（`rebuild_fr_aggs`，走发布视图的迭代器不复制整条环；描述加载/移除是人手一次的动作，不是每帧）。不这么做的话，加载描述之前那段会留在一条无名行里、和同一帧的新行并排显示两条。回归 `loading_a_description_re_tallies_the_rows_on_screen`。
- **同一个坑的另外三处**：`FrDb::slot_signals()` 原来"一个槽只列第一个帧"，于是同槽其它相位的信号**在选点树里根本看不到** → 改成**一个有信号的帧一条**（`every_occupant_of_a_slot_offers_its_signals`）；`slot_signal(slot,name)` 原来只查第一个帧 → 改成搜该槽**全部**占用者（曲线的量程/枚举标签查得到自己那一帧）；触发器编辑框的 `App::fr_signal_names` 改成同槽占用者的**并集去重**；选点树控件 id 加上帧名（两帧同槽不再是同一个 ImGui id）。
- **已知残留（没动）**：信号订阅键还是 `(bus, slot, 信号名)`。同一槽的两个占用者若声明**同名**信号，它们共用一条曲线/一个勾选框（两行勾选会互相跟着亮），且 `decode` 按到达那一帧的布局走。要真正分开得把键换成 `(bus, 帧, 信号名)`，那是工程文件里 SigKey 的一次迁移 —— 等出现这种描述再谈，别为假设的需求改持久格式。
- **回归的守卫是破坏性验过的**：把 tally 的 `occupant` 强制成"一律 Unknown"（=退回旧行为），三条新测试当场失败（`left: 1, right: 2`、`[("FA",4)]` vs `[("FA",2),("FB",2)]`、`left: 1, right: 6`），改回即绿。全量 **625 通过 / 0 失败**，clippy 干净。

### 2026-09-20：展开行的长文本 —— 第一次用剪贴矩形是错的，正解是**列级 NO_CLIP**

- **现象**（用户截图）：FlexRay 行展开后子行的值只看得见一半（`2.54 Mpa |`、`0 [] (正常`），右边五列却空着。原因在 `ui/messages.rs`：子行把值画在**第 2 列 = Bus 列（固定 60 px）**，而 FR 值串是 `物理值 单位 (枚举标签)  (hex)`，远长于 60 px。CAN 走同一条代码路径，只是值短所以没暴露。
- **第一版修法（已推翻）**：`span_row_text` 给单个 item `push_clip_rect(..., intersect=false)` 到窗口右边界。用户复验后报两条:**"还是被边框压住"** + **"向下滚动时展开内容超出表格区域"** —— 手工换掉裁剪矩形同时换掉了表格自己的 `InnerClipRect`（纵向/滚动裁剪），文本于是逃出表格；而边框在后面的通道里画，压在上头。教训：**别在 imgui 表格里手工操作裁剪**，它有自己的一套通道。
- **正解（用户指出方向，读 vendor `imgui_tables.cpp` 确认）**：`TableColumnFlags::NO_CLIP` 加在 **Message 列**上。它跟**表级** `TableFlags::NO_CLIP` 不是一回事 —— 表级那个会整表关掉逐格裁剪（`0616fdc` 的"往下拉右边列消失"就是它 + 冻结表头）；列级那个只在 `TableMergeDrawChannels` 里**跳过"把该列合并通道裁到列宽"这一步**（源码 2819 行 `if (!(column->Flags & ImGuiTableColumnFlags_NoClip))`），表格自己的 `InnerClipRect` 仍然生效 → 文本能向右跑进空列，但**不会跑出表外**。
- **第二版（用户复验后推翻，2026-09-20 同日）**：`same_line_with_pos(SIG_VALUE_X=260)` 把值钉在固定偏移上，于是**名字一长就和值重叠**（用户截图：`Sig_PHPD_DriveMotor1_Contactor_OrderStat` 的尾巴被 `(1h)` 压在下面，`...OrderStatus 1 (吸合) (1h)` 挤成一坨）。固定偏移等于假设"没有名字会超过 260 px"，而真实描述里 40+ 字符的信号名是常态。
- **现在的做法（用户指定）**：**名字占第 1 列（Message）、值占第 2 列（Bus），NO_CLIP 挪到 Bus 列上**。于是两列各画各的、永不重叠，值照样向右摊开（子行右边五列本来就空）。配套：Message 权重 1.0 → **2.0**（Data 保持 1.0）给名字让出宽度；Message 恢复裁剪 —— 名字超过列宽时**截断在列边**而不是压到值上，要看得更全就拖宽表头（`RESIZABLE` 本来就有，且 `NO_BORDERS_IN_BODY` 把拖拽限制在表头行）。`empty_note` 那句说明跟着搬进值那一格（它本来就比名字列长）。
- **Trace 同类缺陷**（同一张截图顺手暴露）：`FrSig` 子行把**信号名**画在 **ID 列（固定 68 px）**，`Drive_Attitude_Alarm_Valid` 被切成碎片 → 名字挪到 **Name 列**（CAN 行的报文名就在那儿，本来就是给名字用的宽度），`└` 留在 ID 列对齐父行的槽号。Trace 的 Name 列是 stretch，不需要 NO_CLIP。
- **顺带**：`dbc::fmt_signal_value` 无条件拼 `" [{type_tag}]"`，FlexRay 没有类型标记 → 每行白印一对空 `[]`（截图里的 `0 [] (正常`），既占宽又像缺陷；现在缺的部分不印（测试名 `the_value_cell_joins_only_the_parts_that_exist` 本来就是这个契约），DBC 里没写 genMsgType 的信号一起受益。
- **验证边界（别当已验收）**：无头 ui_tests **走不到** Messages 的展开分支（树节点默认关闭，控件点击不在这套覆盖里，见"备注"），Trace 那条只被"不 panic"覆盖。所以**要看界面**：展开一条 FR 行确认名字与值各在其列、互不重叠、向下滚不越界，Trace 那边信号名在 Name 列。**全量 628 通过 + clippy 干净只证明没弄坏别处。**
- **代价（看到了再说）**：Message 权重 2.0 之后父行的 **Data 格变窄**（自带 FR 载荷 48 字节 ≈ 143 字符，本来就画不全，两版都是裁断）；而名字超过 Message 列宽的子行**只看得见前半截**，要么拖宽列，要么改成"值紧跟名字"（`same_line`，代价是各行值不再纵向对齐）。

### 2026-09-20：展开的 FlexRay 子行不再整环格式化（重建 51 ms → 20 ms）

- **为什么要单独动它**：上一批把稳定态降到 0.7 µs 之后，剩下的开销只在**重建**那一次：改过滤器、点列头排序、清屏、换描述。实测（`perf_flexray_readouts_under_load`，`--release`，满环 50 000 FR 帧）只列帧 6.7 ms、**勾上"FR 信号" 51 ms**。51 ms 对 16 ms 的帧预算就是"键入一个字母跳一下"。
- **先量再改（`perf_flexray_child_build_split`）**：把展开重建拆成四段分别计时，结论是**解码不是瓶颈，格式化才是**：
  - 判定每行是哪一帧（`frame_ix_at`）：50 000 行 **1.1 ms**；
  - 再把 216 661 个信号从载荷里抠出来并算物理值：**5.4 ms**；
  - 再把它们变成缓存里的行（88 B/行的 `TraceRow` 入表）：**8.6 ms**；
  - 最后一步——把值**格式化成文本**（`"2.54 Mpa (正常)  (1Fh)"`，每行两个 `String`）：**56 ms**。
  即展开重建里 ~90% 是在给**屏幕上根本没有的行**印字。这个探针每轮都把读到的值折进一个会被打印的 sink：不加的话 LLVM 把算术整段删掉，有一次它就"测出"整条 walk 只花 4 ns/行。
- **改法**：`TraceRow::FrSig` 从 `{t_us, slot, signal: String, value: String}` 换成 `{t_us, bus, slot, frame_ix, child_ix, raw, phys}` —— **值照旧在入表时算好（payload 就在手边），文本推迟到绘制**（`ui/trace.rs` 那一格调 `App::fr_child_text` → `FrDb::child_text`）。一条子行存的是"**帧索引 + 该帧声明列表里的第几个信号**"，不是"第几个能放下的信号"：`FrDb::frame_values` 与 `decode` 共用同一处判据（`bits_fit`），所以"预留了几行"与"每行解出什么信号"不可能各说各话（`a_partial_payload_resolves_children_by_their_declared_index` 用"第一个声明的信号恰好放不下"这一形状钉住）。
- **`FrDb` 的解码入口一起换成按帧索引**：`decode`/`decode_signals`/`signal_names` 以前收 `&FrFrameDb`，各自再走一遍"按 PDU 名查表 + 减基"。现在这些在 `assemble` 里一次算成 `children: Vec<Vec<FrChild>>`（`(pdu_ix, sig_ix, bit)`，类型与字段都私有），三条读法都从它出发 —— 顺带把 `decode` 与 `decode_signals` 里那两段几乎重复的遍历合成一处。**逐帧输出与旧实现一致**（真资产那条精确文本断言 `decodes_a_signal_from_the_real_arxml` 未改即过）。
- **推迟出来的索引必须有主人**：子行里的 `frame_ix` 只对"建缓存那一刻该 bus 挂的那份描述"有意义，所以 `RowsBuild` 多带一项 `fr_dbs: Vec<(u8, Arc 地址)>`，换描述 = 指纹变 = 全量重建。这条判据单独验过：把 `extends_to` 里那一行摘掉，`a_new_cluster_description_rebuilds_the_expanded_rows` 当场失败（旧缓存只给一行 `One`，新描述本该给两行）。用 `Arc` 地址而不是版本号计数，是因为**版本号要每个改动点都记得加**，测试里直接 `app.fr_buses.insert(..)` 就会漏；地址是"换了就是换了"的事实本身（新 Arc 必在旧 Arc 还活着时分配，故不会撞址）。
- **结果（同一台机器、同一个探针）**：展开重建 **51 ms → 20 ms**，只列帧 6.7 ms 不变，稳定态 1.5 µs 不变。**顺带的内存账**：200 000 行缓存里不再有 ~43 万个 `String`（旧格式下每条子行两个），重建时也就没有 43 万次 malloc/free。
- **剩下的 20 ms 不是解码**：50 000 父行 + 216 661 子行 = 266 661 条 88 B 的行，光"入表 + 搬进 `VecDeque` + 丢弃旧的"就是十几毫秒。这一条已做，见下一节（**展开子行不再摊平进行列表**）。
- **回归**：`a_deferred_child_row_prints_what_the_eager_decoder_prints`（推迟路径与急解码逐字相同 + 空载荷/越界索引）、`a_partial_payload_resolves_children_by_their_declared_index`（索引口径）、`a_new_cluster_description_rebuilds_the_expanded_rows`（描述换人必须重建）。`cache_shape` 现在把子行**解析成文本**再比较，所以"分批扩展 vs 一次全走"那条等价性测的是用户看得见的字，不只是索引。ui_tests 里注入的 FR 子行改成从真资产现算（`frame_values`），否则"能画"就等于没画过解码那条分支。**全量 628 通过 / 0 失败**，clippy 干净。**可见形状没有改动**：值、单位、枚举标签、`(Nh)` 尾巴与挪到 Name 列的信号名都和上一批一致，那些要看界面的项（跨列、不越界）仍待用户复验。

### 2026-09-20：展开子行不再摊平进 Trace 行列表（重建 20 ms → 7.9 ms，与只列帧同档）

- **上一步留下的账**：文本推迟到绘制之后，展开重建仍是 20 ms，只列帧 6.7 ms。差的十几毫秒**不是解码**，是行列表本身 —— 50 000 父行摊成 266 661 条 88 B 的 `TraceRow`（`TraceRow` 被 `CanFrame` 内联的 64 字节载荷撑到 88 B），每次重建要 push、搬进 `VecDeque`、再把旧表整列 drop。
- **改法**：`TraceRow` 只剩 `Can` 与 `Fr(FrRow, u32)` 两型，**子行不再是列表里的条目**，父行带一个"我有几个子行"的计数；表格的虚拟滚动改成按**摊平后的行数**计数（`table_row_spans` 求前缀和、`locate_row` 用 `partition_point` 把表行号映射回 `(缓存条目, 偏移)`，偏移 0 是帧行本身、1.. 是第 k 个解码信号），绘制那一格再用 `App::fr_child_cell` 从**父行自己的 payload** 现解。于是缓存里再也没有 `FrSig`，`MAX_CACHED_ROWS` 自然只按帧计。
- **计数为什么便宜**：一帧有几个信号放得进 `n` 字节，只与描述有关、与到达无关。`FrDb::assemble` 因此多算一张 `child_ends: Vec<Vec<u32>>`（每帧各信号的**结束位**升序），`child_count(frame, bytes)` 就是一次 `partition_point`。旧写法是每行走一遍 `frame_values().count()`（216 661 次 `sig_of` + 判据），实测那一段就是 14.1 → 7.9 ms 的差。**快路径与慢路径必须同答案**，所以 `the_counted_children_are_the_ones_that_resolve` 把 `child_count` 与 `frame_values().count()` 在合成库（声明顺序故意与位序相反）与真 ARXML 的 9 种载荷长度上逐一对齐。
- **顺带修掉的两件**（都是这一改的必然结果，不是新特性）：① **`MAX_CACHED_ROWS` 以前把子行算进 200 000 上限**，所以勾上"FR 信号"时展开视图会在 50 000 帧处撞顶、比不展开时**少看得见更早的帧**；现在两种视图都是 50 000 条帧行，头部裁剪口径一致。② **列排序以前会把解码子行撒开**（子行是独立条目，按 Len/Flags 排就离开自己的父行）；现在帧行是一个条目，子行跟着父行走。顺手让 FR 行的 Name 排序键从"一律空串"改成**该行的帧名**（`fr_row_name`），否则去掉子行之后 Name 列对 FR 就完全不动了。
- **表头那句"N matching frames"跟着变准**：它读的是缓存条目数，展开时旧版数的是"帧 + 信号行"，现在数的确实是帧。
- **实测（同一台机器、同一个探针，满环 50 000 FR 帧）**：展开重建 **20 ms → 7.9 ms**，只列帧 6.2 ms（差 1.7 ms），稳定态 2.0 µs；缓存条目 200 000 → 50 000（17.6 MB → 4.4 MB，且没有 43 万个 String）。拆分探针同步更新："build rows" 现在量的是 50 000 条带计数的行（5.2 ms），"format text" 仍是 56 ms（只有画出来的几十行才付）。
- **回归**：新增 `table_rows_map_back_to_their_frame_row`（前缀和与映射的边界：帧行、它的第一个/最后一个子行、相邻 CAN 行、无子行帧 —— 这套偏移错位无头测试看不见，只能靠纯函数单测）。`cache_shape` 改成**按表行展开**（一个带三个子行的帧行贡献四条），所以"分批扩展 vs 一次全走"那条等价性仍然逐字比较用户看得见的东西；`the_trace_row_cache_extends_in_place_without_losing_rows` 多加一条"表行数 > 缓存条目数"的断言，摊平没生效时它会响。`a_new_cluster_description_rebuilds_the_expanded_rows` 原样通过（计数来自描述，换描述必须重建 —— 这条判据现在守的是计数，不再是索引）。**全量 630 通过 / 0 失败**，clippy 干净。
- **要看界面**（无头跑不到滚动与裁剪）：展开一条 FR 行，确认子行仍紧跟父行、`└`/名字/值三格位置没变；**拖滚动条到长表底部**看行号与内容是否错位（`locate_row` 的偏移错了就是这个症状）；点 Len/Flags/Name 列头排序，确认解码子行不再散到别处。

### 2026-09-20：FlexRay 静态段占用率（S4 的最后一格，口径由用户选定）

- **背景**：S4 盘点时唯一悬着的项就是"负载/占用率口径"——这不是能自己定的东西（数字要能和 CANoe 对得上），所以一直空着。用户选定**时间占用率（按描述参数算）**，不是"观测到的槽数占比"。
- **口径**：一帧占用的介质时间 = 该路描述的 `gstaticSlot × gmacrotick`（与载荷长短无关，这是 FlexRay 与 CAN 的根本差别：槽是预留的）；占用率 = 窗口内到达帧的占用之和 / 窗口时长（沿用 CAN 的 1 s 滚动窗口 `WINDOW_US`，同样"只由新帧推进"）。写进 `crate::load::FrLoad` + `fr_slot_wire_us`，`ingest_fr_row` 每次到达记一笔，`step` 每拍 `sample()` 喂 Min/Max/Avg，随 `loads_dirty` 一起发布到 `snap.fr_loads`（按 cluster 索引，与 `FR{n}` 同一套编号）。
- **两条明确不做的**：① **动态段不计入**——被动监听只报静态槽，算 minislot 需要参数里根本没有的 action point 索引，编出来的是假数；② **描述里没有槽时长就不报占用率**（`fr_slot_wire_us` 返回 `None`，Statistics 那行显示 `-`，而帧数照记）——0 % 会被读成"这条总线很闲"，而事实是"我们无从计算"。冗余集群不双算：一帧在日志里是一条记录（channel mask = both）。
- **顺带修掉一个真 bug**：`clear_aggregates`（Messages/Statistics 的 Clear 按钮）以前只清 `self.aggs`，**没清 `fr_aggs`** —— FR 行如今与 CAN 同表并列，Clear 之后 FR 计数继续从旧值往上爬，等于按钮对一半表格说谎。现在两者一起清，状态行的计数也是两者之和（回归 `clearing_the_message_counters_clears_the_flexray_tallies_too`）。
- **界面**：Bus Statistics 窗口每个有流量的 FlexRay cluster 一节（`FR{n}（cluster 名 / 无描述）`），行是 静态段占用 [%] / Frames [n/s] / Frames [total] / Slot wire time [µs]，节标题的 tooltip 写明口径与"动态段不计入"。
- **回归**：`a_flexray_cluster_is_charged_one_slot_per_arrival`（100 帧 × 40 µs = 0.4 %，逐位精确）、`the_flexray_window_prunes_and_ignores_a_seek_back`（倒退的一帧计入总数但不进窗口——与 `BusLoad` 同一条纪律）、`an_untimed_cluster_counts_frames_without_a_load`、`slot_wire_time_comes_from_the_cluster_parameters`（0 槽时长 / 0 宏周期都返回 `None`），加两条走完整核心的 `app_tests`。**全量 637 通过 / 0 失败**，clippy 干净。**要看界面**：Bus Statistics 里 FR 那节的数字与 `-` 的分支（无描述那条路）。

### 2026-09-20：FlexRay 也进规格监视（四条判据按调度表判定）

- **为什么现在能做**：CAN 的四条判据（Unknown/Dlc/Cycle/Missing）一直只吃 CAN `aggs`。FlexRay 的"应该怎样"写在描述里更硬的地方——每帧声明自己占哪个槽、哪个周期相位、重复几次、载荷多长，所以期望周期直接是 `cycle_repetition × 宏周期`，不必像 DBC 那样靠 `GenMsgCycleTime` 猜。
- **形状**：`Spec` 加**第二张表** `fr_rows: BTreeMap<((cluster, slot, Option<帧索引>), Kind), Latch>` 与 `fr_previous`，而不是把 CAN 的 `(channel,id,ext)` 键扩成能塞槽号——`(0,5)` 同时是 CAN0 的 id 5 与 FR0 的槽 5，一张表会让两者互相定罪（`drop_channel` 因此只动 CAN 表：增删 CAN 通道不该重编号 cluster 索引）。报告与 CSV 走新的 `App::spec_rows()`，两表在那里合并成统一的 `{bus, addr, name, kind, declared, measured, count, first, last}`，窗口与导出里各自的键格式化随之删掉。
- **四条判据里 FlexRay 特有的两点**：① **Dlc 只判"短于声明"**——槽的载荷是定长的，日志把整槽记下来时帧边界之外本来就有填充，长于声明不是错（CAN 那边帧就是线上的东西，等号判据是对的）；② **Unknown 的主体是"排不出帧"**：`FrOccupant` 解不出帧索引时键里的帧位是 `None`，这正是 Messages 里那条"无名"行，两个视图说的是同一件事。
- **回归**：`the_spec_monitor_judges_flexray_frames_against_their_schedule` 用真 ARXML 一次跑全四条（短载荷→Dlc；隔 5×声明周期再来一帧→Cycle；再静默 `grace+10` 周期→Missing；槽 2000 到达→Unknown），期望周期**从描述算出来**而不是挑一个能过的数。破坏性验过：短路掉 Dlc 条件，断言当场报 `was judged [Cycle, Missing], expected Dlc too`。改动中还有一条旧断言救了场——`clear()` 只该清报告、不该清 `previous`（"清了报告就把时钟也忘了，下一步会跨着刚被抹掉的空档量周期"），我顺手一起清时被它顶回来。CSV 头部新增 `# flexray,FR{n},<描述路径>`：判定前提是那份调度表，缺了报告就无法复核。
- **全量 642 通过 / 0 失败**，clippy 干净。**要看界面**：Specification 窗口里 FR 行的显示（bus 列 `FR{n}`、id 列 `slot N`、name 列帧名或 `not in the schedule`）。

## 结构待办（零散）

- **外部仿真元件动态库加载**：进程内注册表已就绪（`script::register_extern`），动态库 C ABI 插件约定与加载器另议（含沙箱边界）。
- **双类展示层四元组化**：网络视图、TX 选择器、报文过滤器对同值双类只列一条（标准优先）。等有真实双类库需求再做。
- **Vector CAN FD**：FR-2 笔记——Vector 的 FD 参数接口（`XLcanFdConf`）后续迭代提供，目前 Vector 挂接仅经典 CAN。
- **Trace 归档常驻可回看**：磁盘归档已随导出覆盖，"不导出也能回看历史段"需要 UI 立项。

## Roadmap 候选（远期，按需启动）

- **带判定的测试单元**：从 R2 静态分析自然长出的门禁能力（"发送集符合 DBC 声明"作为一条断言）；要不要做成"跑完仿真再给结论"的测试单元，还是保持在 `--check-script` 编译期范围——待决策（`docs/roadmap.md` §6.5）。
- **CTE 自定义语言/关键字**：`let` / `fn` / `on` 目前按标识符着色。正确路径是请上游 cimCTE / dear-imgui-rs 暴露自定义关键字接口（可在已回复的 issue 里顺手提），**不再 fork**。

## 待议（等明确诉求）

- System Variables：字符串型变量（现仅数值）、枚举型。
- CSV 口径细化（State Tracker 等）：等用户明确提出。
- 独立的时间范围双输入框（现有跳转已覆盖核心需求，低优先级）。

## 明确暂缓 / 边界外（当前定位下的否决——定位若变（如转向台架 / HIL），须显式重启决策，不许慢慢渗进来）

- **面板设计器 / HMI 控件**：CANoe 的"重"本身；需要 on key / 控件事件一整套动态事件模型。轻量纯文本监控面板已转正（见晚间批次）。
- **换脚本语言底座（Lua / JS / Python）**：与"编译期可判定"正面冲突，得到的是表达力不变的阉割版 + 更多代码（依据见 `docs/roadmap.md` §4 与 R2）。
- **工业级测试报告（HTML / XML / PDF）**：与"零配置出结论"冲突；CSV + CLI 退出码已够。
- **多机分布式测量、实时系统与时间同步（PTP / GPS）**：定位外；核心是普通 std::thread。
- **用户 / 权限管理、国际化**：当前受众不构成瓶颈；中文字体与 IME 已解决实际痛点。
- **网关与跨总线路由模型**：依赖多数据库与节点体系先行，另立项。
- **诊断层（UDS / CDD / ODX / 刷写）**：按"更多协议"那一轮一起谈。
- **MF4 读取**：卡在"先拿到 Vector 真实导出样本"，没有样本不可验证。
- **LIN / FlexRay 信号解码 / 以太网**：协议那一轮一起谈（FlexRay RX-only 监听已按上面的分期先行）。

## 弯路记录（教训与纠正状态）

按影响排序。已纠正的保留条目作为回归防线；未纠正的指向对应待办。

1. **节点清单双源**（影响最大；✅ 已随 P0 角色模型纠正）：仿真节点名单存了两份——DBC 发送节点一份、`sim_nodes` 一份，"以谁为准"悬空。纠正：`NodeRole` 两态（模拟/离线）取代 `sim_nodes`，角色只能声明在 DBC 已有节点上，旧工程列表迁移为 Simulated。（原三态中的 `Monitor` 2026-09-15 退役：与 Absent 行为完全相同。）
2. **帧类硬编码假设**（✅ 已纠正）：`build_node_inputs` 硬编码 `extended: false`——扩展报文的 `sig()` 永远读不到值；脚本 `send` 长缓冲无校验——能发出无 FD 标志的 64 字节坏帧。纠正：帧类进键模型、FD 自动取整 + 长度上限（fe3c331 之前的 955dac5）。
3. **脚本内建裸奔**（✅ 已纠正）：`ramp`/`sine_wave` 从已清空的栈取参（一调即崩）、`srand` 栈失衡——都是实现先落地、测试后补时暴露的。教训：内建落地即带测试，不留"先跑通再说"。
4. **跨层契约回归**（✅ 已纠正）：extern 解析链改造时，把"钩子拒绝调用 = 报错"静默改成了 nil。教训：改解析链先写契约测试钉住两条路径各自的语义。
5. **提交不完整**（✅ 已纠正）：一次提交漏掉 bus.rs，历史提交点编译不过。教训：提交前全量 `cargo build --tests`，而不是只验证改动的文件。
6. **测试写 CWD**（✅ 已纠正，翻车两次）：录制类测试不设路径 → 默认派生 CWD `record_*.asc`，并行运行同秒同名互删文件；修正后又出现漏网（rearm 测试）。教训：每个新录制测试显式 temp 路径，全量测试跑完检查 `git status`。
7. **PowerShell 批量改文件**（✅ 教训已记，但同日再犯一次）：批量重写文本引入 BOM，或以系统码页误读 UTF-8 中文注释导致乱码、甚至吞掉行首属性（`#[cfg(test)]` 被并入注释行）。教训重申：**批量文本修改只用 Edit 工具**；PowerShell 只允许只读检查。可用的替代：ripgrep / coreutils / sed / jq / fd（已安装）。
8. **裸 id 展示层的双类妥协**（已知妥协，未纠正）：选择器对同值双类只列一条、ImGui 控件 ID 需手工区分。等真实双类库需求。
9. **50k Trace 环静默丢帧**（✅ 已纠正）：长抓取丢头部。磁盘归档方案已落地（热环溢出追加写临时归档，导出先回放归档再写热环）。
10. **UI 用了字体缺字形的符号**（✅ 已纠正，2026-09-11）：合并字体没有 ●○（U+25CF/U+25CB）等几何符号，渲染成"？"；全角"·"和"→"在中文字体里有（→ 后仍换成 `->` 更稳）。教训重申：**UI 字符串优先用 ASCII 能表达的东西**（色标 `[S]`/`[-]`、状态点 `*`/`o`/`.`），图形符号需要字体方案立项时一并解决。
11. **CanKingService 常驻占虚拟通道 init access**（✅ 已实证定位，2026-09-11）：CAN King 的后台服务 `CanKing`（自启动，GUI 关了也在跑）持有 Kvaser 虚拟通道的 init access，导致 roxy-can 收发挂接始终 canERR_NOTFOUND、降级只收。裸标志矩阵实证：该驱动 `ACCEPT_VIRTUAL(0x8000)` 一律 PARAM（不可用），唯一出路是停掉服务（管理员：`sc stop CanKing` + `sc config CanKing start= demand`）。注意虚拟通道的只收句柄其实仍能发车——"只收"标签在虚拟通道上不代表不能 TX。
12. **canGetChannelData 在本机必 AV；通道名走 canGetHandleData**（✅ 已实证，2026-09-11）：umbrella canlib32 的 canGetChannelData 在本机对任何 item（含文档内 item 13/26、256 字节缓冲、先 canLocateHardware）都 AV 杀进程。可行路径：**开一个 NO_INIT 句柄（不动总线参数不 BusOn）后用 canGetHandleData(item 13)** 读用户友好通道名（"Kvaser Leaf Light v2 #0 (Channel 0)"），item 码与通道级共用（canCHANNELDATA_*）。enumerate() 已改走此路径，下拉显示真实设备名。教训：FFI 探针必须单 item 单进程隔离，AV 会带走整个测试进程。
13. **Windows 默认定时器分辨率 15.625 ms 吃掉唤醒精度**（✅ 已纠正，2026-09-13）：核心事件循环按 `next_deadline` 算好等待时长，但 `recv_timeout` 在 Windows 上经系统定时器中断解析，默认 64 Hz——死线醒来必然晚 0~15.6 ms，脚本 `set_timer(200)` 实测 206~215 ms。修复用操作系统正规机制（`timeBeginPeriod(1)`，随核心线程生命周期持有），不做"多睡几次凑近"式补丁；周期定时器另需按计划时刻（而非晚发时刻）重锚，否则一次晚醒永久漂移后续所有节拍。教训：**"调度对、睡不准"先查 OS 计时器分辨率**，别急着怀疑自己的死线数学。
14. **imgui 模态草稿必须逐帧回写**（✅ 已纠正，2026-09-13）：变量/触发编辑弹窗每帧从 App 侧克隆草稿，控件键入只落在帧内克隆——ImGui 内部状态照常显示，但 Apply 校验的是陈旧副本（等于没改，且报错文本活不过当帧）。修复：每帧把克隆回写 App，成功产物与校验错误也走同一路径。教训：**InputText 显示态和 Rust 侧缓冲是两份状态**，凡是"每帧 clone → 控件改 → 帧末读取"的形状都必须回写。
15. **PowerShell 改状态文件再犯**（✅ 已纠正，2026-09-13，教训 #7 重演）：清理测试残留时用 PowerShell `ConvertTo-Json` 重写自动保存文件——编码码页读入中文乱码、重序列化悄悄掏空数组。教训重申且加码：**状态文件要么用 Edit 工具、要么用程序的 serde 路径处理，PowerShell 只做只读探查；写回前必须先确认读到的形态正确。**
16. **强杀进程留 Vector 虚拟通道残留状态**（✅ 已缓解，2026-09-15）：`taskkill /F` 强杀持有 Vector 虚拟通道的进程后，驱动层端口/激活状态有秒级残留——回环探针第一轮收不到帧，等待后自愈。教训：探针带一次重试即可；**真机验收尽量正常退出程序**，不要习惯性强杀。
17. **`grep -c $'\r'` 查行尾是假绿灯**（✅ 已纠正，2026-09-20）：本轮 Edit 工具把 README.md 整个翻成 CRLF 并进了提交（`git diff --numstat` 109/109 才暴露），而我提交前的检查 `grep -c "$(printf '\r')"` 对一个含 109 个 CR 的文件返回 **0**；`$'\r'` 形式在这个 shell 里更糟——它退化成空模式，于是"每个文件都有满行 CR"。正确写法：**`tr -cd '\r' < file | wc -c`**（数字是 CR 字节数，期望 0），改完再用 `git diff --ignore-cr-at-eol --numstat` 证明内容没动。教训：**校验命令必须先用已知为真的样本测一次它会不会报错**——一条永不报警的检查比没有检查更危险。
18. **"全局一份"的模型是延时炸弹**（✅ 已纠正，2026-09-20）：FlexRay 侧三处"全局唯一"（一份 `FrDb`、一个 `FrWatch`、一个 `Option<String>` 工程字段）在只有一路时看不出问题，但每一处都在第二路出现时静默出错：帧名去别的 cluster 表里查、第二个端口的流量并进第一路、工程只存得下第一份描述。同批还有 `TriggerCond::bus()` 把 CAN 通道号与 FR 总线索引混成同一个整数（`remove_bus` 会照着 CAN 的删改规则挪走 FR 规则）。纠正：键里带实体（`fr_buses`/`fr_watches` 按 bus、`can_bus()`/`fr_bus()` 分开）。教训：**加"第二种实体"之前先把隐式全局做成按键集合**，别等它和第二个实例一起上线；返回裸 `u8` 表示"某个总线索引"的 API 必须说明是哪套编号空间。

19. **样式守卫宏被 `{{ }}` 吞掉 = 静默无效**（✅ 已纠正，2026-09-21）：把 CAN 行改成"平时像文本"的 `quiet_field!` 写成 `($ui:expr) => {{ let _bg = push…; let _bd = push…; }}`——内层块在宏展开处就结束了，两个 guard 当场 pop，`input_text` 画的时候样式早已恢复。编译过、661 测试全绿（ui_tests 不看像素）、界面毫无变化，是用户的截图发现的。修法：宏展开成**裸 `let` 语句**（不带内层块），调用点自己开 `{ quiet_field!(ui); ui.input_text(…).build(); }`，pop 落在那个块尾。教训：**"push/pop 成对"的样式包装，生效与否编译器与测试都不替你查**，只能靠眼睛或断言；同类的"绿色不等于跑过"还有下面那条 skip 假绿。
20. **改原生控件的配色去"统一外观"是手搓**（✅ 已纠正，2026-09-21）：为了把 CAN 行做得"像 FlexRay 那样是文本不是表单"，我写了个把 `FrameBg` 设透明的宏，用户先看截图发现没生效（#19），修好后直接否掉这个方向：**用原生控件、用原生控件的样式**。落地方式改成——比特率从输入框换成**下拉**（`CAN_ARB_KBPS` / `CAN_DATA_KBPS` 常量表 + 工程现值折进列表，选错比特率只会静默误解码，所以不给自由输入；`0 = 无 FD 数据段` 从此可达，旧输入框的 `max(1)` 把 0 吃掉了），总线名回到原生输入框；两张表的"统一"只靠**同样的列序、同样的列宽（第三列同为 190）与"值在第一行、动作在第二行"**达成。教训：外观一致性要靠选同一类控件与同一套布局，不是覆盖控件配色——后者既要维护魔法数字，又和主题/字体缩放打架。
21. **表里不要放"等你配置"的占位行**（✅ 已纠正，2026-09-21）：FlexRay 表当初为了不"另开窗口才能加路"，把 `fr_description_targets()` 里那个"第一个空索引"直接画成一行 `FR1（未配置）`——用户第一反应是"我什么时候多了这条总线？"。纠正：加路由标题行的 **`+ Add FlexRay`** 按钮做（与 CAN 的 `+ Add bus` 同形，FlexRay 的"加一条"= 挑一份集群描述，落在 `next_flexray_bus()` 上，取消即什么都不加），行集合换成 `fr_bus_rows()` = 已描述 ∪ 已监听 ∪ **日志里真有流量的那路**（后者标"（日志里有这路流量）"，它是需要挂描述的现实状态，不是占位）。`fr_description_targets()` 随之下线。教训：**占位行是把"操作的入口"错做成"对象"**——入口放按钮上，表里只列真实存在的对象。
22. **"文件读不到就跳过"的资产测试是假绿灯**（✅ 已纠正，2026-09-21）：`the_trace_text_filter_matches_fr_frame_names` 用 `std::fs::read_to_string("assets/arxml/PowerTrain.arxml")`，而**那份 ARXML 是 GBK**——read_to_string 对非 UTF-8 一律 Err，于是这个测试从写下起每次都是"打印一句 skip 然后 ok"，主体一次也没跑过。今天给 dbc_only 写测试时照抄了这个形状，**破测（把新分支退回旧写法）居然仍然通过**才暴露：`--nocapture` 一看，"not present -- skipped"。逐个验过自带资产的编码：只有 `PowerTrain.arxml` 不是合法 UTF-8（`Logging.asc`、两份 FIBEX、两份 DBC 都是），所以只有它中招。修法：测试侧 `read_fr_asset`（`fs::read` + `dbc::text_from_bytes`，与产品同一条解码缝），并且**提交进仓库的资产读不到就 panic**，不再"跳过"。教训：**任何 skip-on-unreadable 的资产测试都要先证明它上一次真的跑起来过**（`--nocapture` 看有没有那句 skip）；新写的行为测试必须做一次破测，通过了才叫覆盖——这一条今天救了两回（同一晚还有一次：`Pick` 的持久化测试靠"把 JSON 里的新键删掉"当旧文件读，如果那个删键没生效，测试也是假绿）。

### 2026-09-21：FlexRay 与 CAN 齐平的另一批（手选、事件、口径、发送者、菜单）

七次提交，全部由"用户已拍板的两件"+"第二轮盘点 B 类"推出来；细节与破测记录在 A/B 两处对应条目里（A1 `on fr slot`、A2 `Pick`、B4 录制白名单、B6 仅 DBC、B8 发送者）。今天新增的、盘点里没有的两件：**dbc_only 逐行问"有没有数据库说明这条到达"**（见 B6，这是行为变化）与 **Trace 的 FlexRay 行右键菜单补上 CAN 那一侧的动作为 "Watch FR{n} slot N" + "Clear filter"**（走 A2 的 `Pick`，加进本窗口 Manual 集合并切作用域；**故意做成增补而不是替换**——一键不该悄悄丢掉用户自己勾好的其他条目）。

**要看界面（无头跑不到，全量 660 通过只证明没弄坏别处）**：
1. Trace 里**右键一条 FlexRay 行**：标题应是 `FR0 slot 13.2 · 帧名`，菜单四项（Watch / Clear filter / Copy payload / Copy slot.cycle），点 Watch 之后该窗只剩这个槽（连同已勾过的），左上作用域变成 `Manual (n)`。CAN 行的菜单没动。
2. **Messages/Trace 勾"仅 DBC"**：挂着描述的那一路，被调度的 FR 行现在**留在表里**（以前整列 FlexRay 一起消失）；"这一路没描述"和"这一相不排这一帧"两种行仍然出去，展开行里那句说明会点明是哪一种。
3. Network 树 FlexRay 节**现在只剩 ECU 一组**（同一天里先做了按槽分组的调度表、随后被用户定掉，见下面"分工"）：展开应只有 `ECU (n)`，绑定得出发送方的行带"发 N 帧"（自带 PowerTrain 描述 48 帧里 12 帧绑得到）；加载一份 FIBEX 时那组显示"描述未声明 ECU"那句话。
4. 工具栏录制过滤框写 `FR0:13` 再录一段混合流量：文件里应只剩这一路的这个槽（Trace/统计仍看全部），CAN 帧不受影响（框里没写 CAN id 时 CAN 侧全录）。
5. 之前欠的：脚本编辑器右栏的 **FlexRay tab**（已从"函数"下面挪出来单独成页）、Bus Statistics 的 FR 节、Specification 窗口的 FR 行、`on fr slot` 脚本在回放 BLF 时的反应（`examples/flexray_gateway.rxcan` 可直接挂）。

**分工（用户 2026-09-21 定，别再往回做）**：**Network 只列拓扑与 ECU，不排帧不排槽**——"具体看调度等信息是别的软件的事情，就像 CANoe 和 Fibex Explorer 的组合一样"。同一天里 `draw_flexray_section` 先按"参数行 / ECU / 帧按槽分组（带实测计数与周期）"做完并被提交（`11d5665`→`add0833` 一路），随后按这条删回只剩 ECU 一组；`FrDb::frame_sender` 留着，因为它现在服务的是 **ECU 行上的"发 N 帧"**（ECU 属性，不是调度表）。以后想再往 Network 里加"每槽有什么"之前先重读这条。**Buses 窗口同日一并收掉（用户第二句："让 Bus 中 FlexRay 的展示和 CAN 的一样，不用展示调度表"）**：底部那份"调度表"（各路全部帧的 slot/周期/重复/通道、启动帧高亮）删除，FlexRay 区改成**与上面 CAN 那张表同构的表格**——Name / FIBEX-ARXML / kbit/s·周期（**只读**：FlexRay 的位时就是调度表本身，改它得改描述文件）/ 硬件（选中空闲端口即挂，与 CAN 行同一动作形状；已挂的行是 `[V] ch{n}（只收）`+断开）/ 行末撤下描述。顺带清掉的：全局"给这路 [FR0*] 加载集群描述…"下拉、"挑描述并挂接…"合并按钮（两列各管各的，正是 CAN 的形状）、`App::fr_pick`/`App::fr_db_pick` 两个会话字段、`App::pick_fibex_for`（那个"替你挑第一个没描述的路"的入口本来就是"不替你猜"的反面）。行集合 = `fr_description_targets()`（已描述 ∪ 已监听 ∪ 日志里出现过的 ∪ 第一个空位），所以纯回放两路日志时第二条路就有一行、可以直接给它挂描述；既无描述又无监听的行标"（未配置）"。这条与上面 Network 那条是同一个分工判断，不是两次独立的删减。

## 备注

- UI 无 imgui 自动化测试床：部分补齐（`src/ui_tests.rs` 无头冒烟床——imgui 不接渲染器也能逐帧跑真实绘制路径，全窗口齐开/脚本编辑器/控件交互等场景由测试守护）。控件交互（点击、选区、输入法）仍靠人工验收，判定逻辑用无头测试自动证明。
- 唤醒精度远期可选：事件驱动核心的"最后 <1 ms 自旋收尾"开关（详见 `docs/architecture.md` 核心时钟模型）。
- CTE 宽字形补丁历史：曾以本地 fork（chemPolonium/dear-imgui-cte-sys）挂接，上游 goossens/ImGuiColorTextEdit #88 合并并经 dear-imgui-rs 0.18.0（源码 patch 22e98fb）带入 crates.io 后，fork 已退役（2026-09-15）。
