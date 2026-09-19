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
- ~~**S1 FrRow 携带总线归属**~~ ✅（2026-09-19 部分）：`FrRow` 增 `bus: u8`（日志一次抓取只有一个 cluster → 回放侧恒 0；实时 watch 打自己的索引），`FrSlotAgg` 与核心表改按 `(bus, slot)` keyed，`SetFrWatch`/`attach_fr`/`FrWatch`/`FrWatchView` 全程带 `bus`，Trace 总线列、Messages 行与导出 CSV 都显示 `FR{n}`；回归 `the_same_slot_on_two_flexray_buses_stays_two_rows`。**剩余**：`ab` 真实化依赖真机确认事件缓冲 channelMask 偏移 → 并入 S5。
- **S1b 事件语义**：`frameId → frame` 与真实 slot 分开解析（BLF 只有 frameId，`blf.rs:572` 现把 frameId 塞进 `slot`；实时 VX1070/VN7640 给真 slot）——回放要按 frameId 解析帧，不能冒充 slot 键。
- **S3 FR 通道列表一等化**：`fr_fibex: Option<String>` → `fr_channels: Vec<FrChannelCfg>`（每路：硬件端口、自己的 FIBEX/ARXML、使能、显示名）；`Option<FrWatch>` → 按 bus 索引的多路；工程打开即挂 watch + 推 DB 给 core，删 app.rs:891 的回放前兜底补丁；Buses 窗口 FR 区从"单选挂接"改为通道表（对齐 CAN 的 AddChannel/RemoveChannel/SetChannelConfig 命令面）。
- **S4 接收侧对齐 CAN**：`ingest_fr_row` 补 frame_counter、每路 FR 负载与周期/抖动统计（沿用 aggs 形状，槽占用率口径另定）、FR 过滤（`workspace.rs:759` 的 `trace_fr_match` 现为一律丢弃）、录制写出 FR 帧（BLF FR_RCVMESSAGE 对象 + ASC `Fr RMSG` 行，回环自证）、`export_trace`（export.rs:10 现 CAN-only）纳入 FR、触发条件支持 slot/帧到达。
- **S5 硬件在环（有 VN7640，可实测）**：多路 FR 同时打开、各自集群配置、`xlFrGetChannelConfiguration` 回读校验、真机确认 `ab` 与 slot 语义；CLI 探针扩到逐路报告。
- 明细见对应任务与 git log。

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

## 备注

- UI 无 imgui 自动化测试床：部分补齐（`src/ui_tests.rs` 无头冒烟床——imgui 不接渲染器也能逐帧跑真实绘制路径，全窗口齐开/脚本编辑器/控件交互等场景由测试守护）。控件交互（点击、选区、输入法）仍靠人工验收，判定逻辑用无头测试自动证明。
- 唤醒精度远期可选：事件驱动核心的"最后 <1 ms 自旋收尾"开关（详见 `docs/architecture.md` 核心时钟模型）。
- CTE 宽字形补丁历史：曾以本地 fork（chemPolonium/dear-imgui-cte-sys）挂接，上游 goossens/ImGuiColorTextEdit #88 合并并经 dear-imgui-rs 0.18.0（源码 patch 22e98fb）带入 crates.io 后，fork 已退役（2026-09-15）。
