# TODO

只记录**尚未完成**的工作项。已完成能力的行为说明见 `README.md` 与 `docs/usage.md`；系统结构与设计决策见 `docs/architecture.md`；已完成批次的明细见 git log（本文件曾长期记录批次明细，2026-09-15 起按"只记未完成"的既定原则精简）。

## 2026-10-08 VN7640 真机批次（四项落地 ✅，明细见 git log；下面只列还欠的）

已落地：Write 窗口长行折行；状态行长消息不再压住左侧状态串（截断 + tooltip 给全文）；硬件下拉回显"配置过/刚试过"的那一口；**发车被线路拒绝不再静默**（第一次进 Write 带驱动原因，之后只累计，Buses 行 `发车被拒 ×N`），状态栏 `MEASURING (virtual)` 改为说帧去哪儿（`simulation` / `real bus · N` / `simulation · hardware parked`）。

1. **FlexRay 真机收帧仍未验证**：这台 VN7640 的 6 个 CAN 口 + 2 个 Vector 虚拟口全部 `caps=0x00000000 / flexray:false`。probe 逐口试开：ch0 `xlOpenPort` 成功但 `xlFrSetConfiguration` 被拒 `112 XL_ERR_INVALID_ACCESS`、**授予本应用的权限位为 0**；ch1/2/3/5 `204 INVALID_CHANNEL_MASK`；ch4 `117 NOT_IMPLEMENTED`；虚拟口 `255`。→ 要带 FlexRay 选件的授权或真 FR 接口。任务 #11/#45 保持 pending，文档里不许出现"已真机验证"。
2. **CAN 口不对称待查**：同一台设备，Vector ch1 挂 CAN 成功（500k 收发、经典模式），ch0 却 `204 INVALID_CHANNEL_MASK`。两种解释：该口在 Vector Hardware Config 里被占用/未分配，或我们的 `index → channelMask` 口径与枚举不一致（probe 与 GUI 同为 `1<<index`，且 ch0 在 FR 侧反而开得起来）。**等用户查过 Hardware Config 再下结论**，别急着改映射。
3. **屏幕验收欠四项**（形状类改动，逻辑与测试都过，眼睛没看）：Write 长行折行、状态行截断与 tooltip、硬件行回显所选口、`发车被拒 ×N` 红字与新状态串。另外 FR 行的同一回显只验了 CAN 侧（FR 侧同一查表逻辑 + 单测）。
4. 任务 #49 仍开着：回放节拍断言在任何机器上都贴线（CI 已因此吃掉一次发布），且它 panic 会毒掉共享 imgui 锁连坐 13 个 UI 测试。

## 2026-09-17/18 夜间批次：用户清单七项 + FlexRay 解析（全部落地 ✅，明细见 git log）

录制路径默认进工程 Record/ 目录；DBC/脚本/FIBEX 的 GBK 编码容忍；Trace 工具条收拢；VN7640 能力位诊断（probe 逐通道 FR 试开）；Graphics Y 轴防遮挡 + 曲线面板横向滚动；CAN+FlexRay 在 Trace/Messages 混排合并（含 FR 信号展开、帧名过滤、CSV 导出）；脚本编辑器三栏化（左大纲/收发、右函数/SysVar/报文）；**FlexRay 数据库解析**（`fr_db.rs`，roxy-fibex 同源：FIBEX 2.x/3.x + AUTOSAR R4.x + 调度表 + 信号解码 + 驱动集群配置推导，真实 PowerTrain.arxml 48 帧回归锁定）。

## 定位（2026-09-11 评审确立，一切取舍的基准）

**竞争对手是开源工具（SavvyCAN / BUSMASTER / candump+PlotJuggler / python-can+cantools），不是 CANoe。** 目标：比开源更流畅、更易用，功能更强，但保持轻。竞争四维度：**打开即分析 / 检索顺手 / 渲染不卡 / 零配置出结论**。CANoe 的"重"（面板 HMI、诊断栈、测试报告、多机分布式、实时与时间同步、多协议、用户权限）明确不做——那些不是更好的功能，它们就是"重"本身。既有优势不许丢：DBC 解码完整性、事实源纪律（DBC 唯一事实源 + 角色两态 + 驱动可插拔）、工程可复现（.rxproj + profiles + 无头 CLI）。逐项对照与依据见 `docs/roadmap.md`（§1-§2）。

## 2026-09-15 晚间批次：稳健性 + 表达力收尾 + 监控面板 + BLF（全部落地 ✅）

1. ~~**核心线程 panic 隔离**~~ ✅：`spawn_lane` 循环三处执行点（命令处理/队列排空/测量步进）包 `catch_unwind`——核心代码（非脚本，脚本有熔断）的 bug 不再静默杀死 bus-core 线程，转成 Write 错误 + 停测，循环继续服务命令。
2. ~~**DBC 自动重载 × `$` 引用失配**~~ ✅：重载后原地重算各节点 `$报文::信号` 名字→id 映射，变化/消失进节点日志 `[reload]` 行。
3. ~~**print 数值格式控制**~~ ✅：`format(fmt, ...)` 内建（`%d/%u/%x/%X/%f/%.Nf/%e/%g/%s/%%` + 宽度/`-`/`0` 标志），返回字符串进 print/日志。
4. ~~**轻量监控面板 v1**~~ ✅：View > Monitor——`[色点] 标签 值 单位` 文本行，行内阈值着色规则（方向/阈值/色号行内可调），行随工程保存并自动重订阅；Clear 清空。
5. ~~**BLF 录制写入**~~ ✅：录制按扩展名选后端——`.blf` 走二进制容器写入（LOGG 头 + zlib 压缩 LOG_CONTAINER；经典帧 CAN_MESSAGE、FD 帧 CAN_FD_MESSAGE_64、RTR/扩展标志齐全，错误帧 v1 不写），写→读回环测试逐帧一致。后续（2026-09-16）：录制格式改为**工具栏 combo 显式选择**（`_<date>.asc` / `_<date>.blf`），摆勾时 combo 决定扩展名（覆盖手敲路径里的扩展名），不再隐式依赖命名。再后续（2026-09-16，`145a321`）：错误帧改为写 `CAN_ERROR_EXT` 对象并有写→读回环测试——上面那句"错误帧 v1 不写"自此作废。
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

现状与 CAN 的差距（盘点结论，逐项带现位置）：**FR 只有两个命令**（`SetFrDb` bus.rs:149、`SetFrWatch` bus.rs:299），CAN 侧约 30 个；FR 不是 `Channel` 实体——全局仅一份 `fr_db` + 一个 `fr_watch`（`HwMap::fr_watch: Option<FrWatch>` hw/mod.rs:202，最多一路），FlexRay-only 端口还被 CAN 挂接下拉显式排除（hw/mod.rs:87-91）；`FrRow` 无总线索引（trace.rs:21）、实时 `ab` 恒为 2（hw/mod.rs:350）、BLF 的 `slot` 实为 frameId（blf.rs:572）；FR 信号靠 `FR_SIG_BASE|slot` 合成 id 挤进 CAN 的 `SigKey`（app.rs:51），A/B、cycle、多 cluster 分不开且通道增删重映射不安全；`ingest_fr_row`（bus.rs:1581）相对 `ingest`（bus.rs:3609）跳过 frame_counter / bus_loads / 通用 aggs / 触发器 / spec / 节点派发 / recorder / 录制白名单 / trace 归档与导出；持久化只有一个 `fr_fibex` 路径（config.rs:550），恢复时既不重挂 watch 也不推 DB 给 core（config.rs:1347，现靠 app.rs:891 回放前兜底）。**本轮范围：S1-S4 接收侧 + S5 硬件在环；发送与调度（静态/动态槽发车、xlFrSendFrame、Generator/脚本认识 FR）不在本轮**——那是 FR-4 的"发车"半边，等接收侧定型后另批。（**2026-09-21 更新**：Generator 已经认识 FR——会话侧发车，见下面《同日第五批》；`xlFrTransmit`、动态段发车与脚本 `fr_send` 仍未做。）

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
  - **2026-09-22 首次真机实测（本机：DIY VN7610 两路 CAN + 正版 VN7640 六路；VN7640 CH2 ↔ VN7610 CH1，带终端电阻）——结论：卡在权限，不在参数**：
    - `xlGetDriverConfig` 的 `channelBusCapabilities` 在**这台机器的每一个通道上都读到 0x00000000**（含 VN7640 全部六路）→ `flexray:false` 处处成立，`enumerate_flexray()`（按那位过滤）返回空表。**这个位不能再当判据用**（`--vector-probe` 早就因此改成"逐路真开一次"，是对的；产品里凡按位过滤的地方都要照此办理）。原始记录转储与解码过程存在 `target/scratch_fr_probes.patch`（探针名 `scratch_dump_driver_config_records` / `scratch_fr_sequence_walk` / `scratch_vector_fr_exports`，需要复现时把补丁 apply 回来即可）。
    - **通道 2（"VN7640 Channel 1"）是唯一能按 FlexRay 打开的口**：`xlOpenPort(busType=FLEXRAY)` 成功（port=0），`xlFrGetChannelConfiguration` 读得动（status=0x0，即"没有有效集群配置"），但 `xlFrSetConfiguration` **一律 112 XL_ERR_INVALID_ACCESS**——全零配置、由 `PowerTrain.arxml` 生成的配置、`xlActivateChannel` 之后再设，三种都是 112，**所以不是集群参数不对**。同时 `xlOpenPort` 回来的**授予权限 = 0x0000000000000000**（一个权限都没给这个应用）。
    - 通道 3/4/5/7 → 204 XL_ERR_INVALID_CHANNEL_MASK（不是 FlexRay 口，符合"CH2 被接成 CAN 与 VN7610 对话"）；通道 6 → 117 XL_ERR_NOT_IMPLEMENTED。
    - **这台机器的 vxlapi64.dll 的 FlexRay 面只有四个入口**：`xlFrSetConfiguration` / `xlFrGetChannelConfiguration` / `xlFrReceive` / **`xlFrTransmit`（在！所以"仿真 ECU"的发车半边可以写）**；`xlFrSetAccessPermission`、`xlFrEnableMcu`、`xlFrDisableMcu`、`xlFrGetStatus`、`xlCanFr*`、`xlChPortGetCaps`、`xlSetApplicationCapabilities` 全部 **MISSING**（用 `GetProcAddress` 逐个实测）。也就是说：**没有"申请 FlexRay 权限"这一步可走**，授予权限为 0 只能是驱动侧的事——**待用户查：Vector License Manager 里这台机器的驱动有没有 FlexRay 选件授权；Vector Hardware Config 里 "VN7640 Channel 1" 这一路是否被别的应用（CANoe/CAPL 会话、Device Manager）占着**。授权/占用一解决，剩下的就是把 `open_rx` 跑通 + `xlFrTransmit` 接上。
    - 顺手改的两处：`open_rx` 的配置失败信息现在**带上授予权限掩码**并据此分岔（权限为 0 → 说"没授权/被占用，去看 License Manager / Hardware Config"；权限非 0 → 才说"检查集群参数"），旧文案一律怪参数，是把人往错的文件里引；`vector_open_probe_loops_a_frame_over_the_virtual_bus` 原来取 `channels[0..2]`，接上真机后那就是 **VN7610 的两个物理 CAN 口**（测试会往上开端口发车！）——改成按名字挑 "Virtual Channel"，不足两个就跳过。

### 2026-09-20 第二轮盘点：FlexRay 与 CAN 还差什么（逐项带位置，含已定的设计）

本轮已落地（不再列为差距）：**S4 收尾**——静态段占用率（`load::FrLoad`，口径见《总线统计与规格监视》）；**Network 视图**列出每路 cluster 的 ECU（`ui/network.rs::draw_flexray_section`；**这里曾经排过一整张按槽分组的调度表，2026-09-21 用户定掉：拓扑视图只列 ECU，调度归 FIBEX/ARXML 编辑器**，见下面"分工"那条）；**脚本读数** `fr_sig(cluster, slot, "Name")`（`HostInput.fr_signals`，从 `fr_aggs` 现解，无节点时零开销）；**State 窗口枚举标签**走描述自己的 VALUE 表（`ui/state.rs::table_label` 的 Fr 分支）。

2026-09-21 追加落地：**Trace 展开子行不再摊平进行列表**（父行只带一个子行计数，文本推迟到绘制时生成——同一次刷新从 70 ms → 7.9 ms，与不展开的 6.2 ms 同档；顺带修掉两个潜伏错误：子行占用 200k 缓存额度、排序把子行打散）；**Messages/Trace 展开行的信号名与值分列**（名在第 1 列、值在第 2 列走**列级** `NO_CLIP`，见《FlexRay 的显示》）；**脚本编辑器右栏列出 FlexRay 槽与信号**（点一条插入 `fr_sig(...)`）；**窗口手选集合 `Pick` 化**（A2）；**`on fr slot` 事件处理器 + `fr_cycle()`**（A1，FlexRay 从此能唤醒脚本，"听 FR 答 CAN"的网关形状，见下面 A1 条）。

剩下的差距，按"要不要用户拍板"分两类：

**A. 需要拍板的两件（2026-09-21 已拍板，两件都按建议做）**

1. ~~**`on fr slot` 事件处理器**~~ ✅（2026-09-21，任务 #27）。语法与取值语义都按上面的建议落地：`HandlerKind::FrSlot { bus: Option<u8>, slot: u16 }`（script/mod.rs），派发在 `BusCore::dispatch_fr_nodes`（两个到达点各一份：BLF/ASC 回放环与实时 watch），节点侧 `ScriptNode::dispatch_fr_row`。**两处值得记住的判断**：① **路号可写但节点通道不参与过滤**——FlexRay 帧没有 CAN 通道，`on fr 0 slot 13` 是唯一能分路的写法，不写路号=所有路；② **`send(frame_id())` 在 `on fr slot` 里编译期拒绝**（槽号当 CAN id 发出去是两套编号混用最典型的错），语法上等价于 CAN 侧那条"id 来源封闭"的规则而不是例外。`fr_cycle()` 是新内建（0..63，非 FlexRay 事件里为 0）。**性能**：整条派发路径在 `fr_listeners = nodes.any(waits_on_flexray)` 后面，每步一次判断；没有 `on fr slot` 脚本时 FlexRay 回放**一个字节都不多走**（row.clone 也在闸内，只为把到达留给处理器）。防线：`a_flexray_arrival_wakes_its_slot_handler`、`a_flexray_handler_can_name_its_cluster`、`fr_cycle_reads_the_arrival_and_nothing_else`、`a_failing_flexray_handler_parks_the_node`、`flexray_handlers_compile_to_their_slot_address`、`flexray_handlers_get_the_same_sanity_checks`、端到端 `a_flexray_arrival_drives_a_script_reaction`（live watch）与 `replaying_a_flexray_log_drives_the_slot_handlers`（回放环，两个派发点各测一个）。**两条断言做过破测**：把路号过滤改成恒真 → 第二个测试红；把 hw 点的 ingest/派发对调 → 端到端测试红（`fr_sig` 报"not seen yet"）；删掉回放点的派发 → 最后一个红。**仍没有的**：`set_fr_sig`/`fr_send`/动态段槽发送（写方向整条缺席，见上面"发车"半边）。**同日第五批更新**：会话侧的交互式生成器已落地（FlexRay 槽能按周期发帧、带信号激励，进 Trace/统计/规格/脚本/录制），见下面《2026-09-21 同日第五批》；缺的只剩**端口那一段**（`xlFrTransmit` 与槽归属，等 S5 台架）与脚本侧的 `fr_send`/`set_fr_sig`。
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
22. **把设计理由写进界面提示 = 口语 + 看不懂**（✅ 已纠正，2026-09-21）：FlexRay 描述列的按钮标 `换…`（换什么？），悬停提示写成"挑一份这路 cluster 的描述…放错路是静默的错…按钮长在哪一行就归哪一路，不替你猜"——那是**给自己看的辩护文**（回答"为什么不自动认文件"这个设计问题），不是给操作者看的说明；用户截图指出后重写：按钮 `加载描述…`/`更换描述…`（动词带宾语），提示只说"做什么、按什么口径、出错会怎样"。同批纠正：`断开`→`断开并移除`（它确实连描述一起移除，标签必须说实话）、`撤下`→`移除`、`无描述`→`未加载描述`、`不排这一帧`→`未在 slot N 调度此帧`、`只数帧、不报占用率`→`只统计帧数，不给出占用率`，并给 CAN 行末的 `x` 与 `+ Add FlexRay` 补上此前唯一缺失的一句说明。**两条可复用的收口**：① 提示里点到别的控件时**用那个控件的常量名**（`FR_DETACH_LABEL` 同时喂按钮与两条拒绝状态行，`load_label` 同时喂按钮与"断开并移除"的提示），文案就不会和屏幕上的字走散，测试也改成断言 `status.contains(FR_DETACH_LABEL)`（做过破测：把状态行退回旧措辞即红）；② 会被列边裁掉的**信息**（cluster 名、完整路径）不要并排塞进行里——挪到第三行 + 悬停给出全文，这是 `visible-but-clipped` 的老教训在文本上的另一半。设计理由该留在 `docs/usage.md` 与 TODO，界面上只留"怎么做"。教训：**写提示前先问这句是给谁读的**——操作者在按钮前只想知道这一步会发生什么，辩护词是给 reviewer 的。**同日第三轮又挨一条**：把 `换…` 改成 `更换描述…` 只解决了一半——上面 CAN 那行的同一个动作叫 `Open...`，下面 FlexRay 这行叫 `更换描述…`，用户第二句意见就是"类似的意思，上面是 open，下面就是更换描述"。**同一个动作在两处必须同一个词**（改回 `Open...`），而且提示要**一句**：我那版三段式（做什么＋为什么不自动认文件＋回放怎么用）被直接判为"帮助太长了"。
23. **"文件读不到就跳过"的资产测试是假绿灯**（✅ 已纠正，2026-09-21）：`the_trace_text_filter_matches_fr_frame_names` 用 `std::fs::read_to_string("assets/arxml/PowerTrain.arxml")`，而**那份 ARXML 是 GBK**——read_to_string 对非 UTF-8 一律 Err，于是这个测试从写下起每次都是"打印一句 skip 然后 ok"，主体一次也没跑过。今天给 dbc_only 写测试时照抄了这个形状，**破测（把新分支退回旧写法）居然仍然通过**才暴露：`--nocapture` 一看，"not present -- skipped"。逐个验过自带资产的编码：只有 `PowerTrain.arxml` 不是合法 UTF-8（`Logging.asc`、两份 FIBEX、两份 DBC 都是），所以只有它中招。修法：测试侧 `read_fr_asset`（`fs::read` + `dbc::text_from_bytes`，与产品同一条解码缝），并且**提交进仓库的资产读不到就 panic**，不再"跳过"。教训：**任何 skip-on-unreadable 的资产测试都要先证明它上一次真的跑起来过**（`--nocapture` 看有没有那句 skip）；新写的行为测试必须做一次破测，通过了才叫覆盖——这一条今天救了两回（同一晚还有一次：`Pick` 的持久化测试靠"把 JSON 里的新键删掉"当旧文件读，如果那个删键没生效，测试也是假绿）。

24. **状态栏不是一个可以回头读的地方**（✅ 已纠正，2026-09-21 夜间批）：三处消息写成过 `self.status` 就以为报出去了——① `Config::apply` 里"描述文件读不到"，紧接着工程载入路径自己把栏盖成 `project loaded`，**那条报错从写下起一次也没被渲染过**；② 步进内部的消息（`replay finished`、触发器命中），下一帧（几十毫秒）就被覆盖；③ DBC 自动重载的提示。都是"只在一次性显示里存在过一瞬间"= 等于没发生。修法不是给状态栏加历史（那是另一个窗口），而是**把 announce 与 record 分成两件事**：`CoreLoop::note(kind, text)` 一处缝负责"上栏 + 入环"，两种驱动的步进、命令状态、panic 停机全走它；前端的带原因失败走 `App::fail`。教训：**凡是要用户事后能查的信息（错误、拒绝、部分失败），落笔处就要同时写进持久的那份日志**；`-> Vec<String>` 这种"把问题交给调用方、由它在正确时机报"的形状，比在被调用方里 `app.status =` 更诚实——因为**谁最后盖栏谁才有资格说话**。附带一条测量纪律：threaded 驱动那条缝无头床跑不到，这类"看得见的面"的改动必须在 TODO 里挂成屏幕检查项，不能拿 675 绿当验过。
25. **每帧重新做命中测试 = 快手上拖动必断**（✅ 已纠正，2026-09-22）：游标拖动写成"这一帧 `is_mouse_down` 且指针离线 ≤6 px 就算拖着"，于是指针一帧移动超过 6 px（60 Hz 下随手一划就是几十 px）就脱离抓取，**同一下的按住被平移分支接走**：线停住、画面跳、线又回来——用户的描述正是"拖动 cursor 时很卡"。修法是把归属交给**按下那一次**：`GraphicsWindow::cursor_drag` 记住这次按压归哪条线，按住期间无条件跟着指针（也允许指针跑出绘图区），松手才放。教训：**任何"按住 A 拖"的手势都必须有 press 期间的所有权状态**，按帧重算命中区域只在指针静止时才成立；而且**别拿"看起来不卡"当验收**——把症状写成一条端到端测试（合成鼠标事件走真实的 `plot_area`：压线 → 一帧跳 40 px → 断言游标跟上了 *且* `t_offset_s` 没动），它当场就红：`moved -0.004s，应 0.806s`（负号正是视图被平移走的证据），修完绿。顺带把那条性能床（`a_plot_window_never_starves_the_replay`）从 1 条曲线改成 6 条 + 每帧移动游标，**实测 A/B**：带游标 247–258 laps/1.5 s、worst lap 1.37–1.65 ms，不带 251 laps/1.5 s、worst 2.14 ms ——**读数块不是瓶颈**，所以这次的"卡"确实只有手势这一个原因（别再往绘制成本上找）。

### 2026-09-21 同日第五批：FlexRay 的交互式生成器（会话侧发车）

用户否掉了"等台架再动发送半边"的排队：**"先推，然后为 FlexRay 加上交互式生成器，类似 CAN 里已有的"**。落地范围要说清楚——**这是会话侧的发车，不是线缆上的发车**：FlexRay 端口仍然只收（`xlFrTransmit` 没接），生成的帧从 `accept_fr_row`（新抽出的唯一接收缝，回放环 / 实时 watch / 生成器三处共用）灌进会话，于是 Trace/Messages/Statistics、负载与规格、触发器、`on fr slot` 脚本、录制文件全都看见它；行上的 ON 悬停与节标题都写明"进入本会话，不上线缆"，不假装能发车。

- **条目身份 `(路, 槽)`**，与曲线键、触发条件、`Pick::Fr`、`fr_sig(路,槽,"名")` 同一套编号；**一个槽一条目**（同槽按周期相位轮着排几帧是调度的事，不是两条激励）。`FrTxMsg`/`FrTxView` 是 `TxMsg`/`TxView` 的对应物，去掉 CAN 才有的 `node`/`fd`/`extended`，加上 `undescribed`。
- **入口只给描述里调度的槽**（`FrDb::scheduled_slots()` 下拉），不做"输入任意槽号"：没有描述就没有帧名、没有信号、没有周期可依据。默认周期 = 该帧 `cycle_repetition × gdCycle`，与规格监视用的是同一个声明值；行上"描述 {n}ms"按钮一键换回（CAN 的 `DBC {n}ms` 同形）。
- **⚠️ 同日即被用户否掉一半（第六批）：条目不属于"一个专门窗口"，属于它的 ECU。** 原话："现在FlexRay的交互式生成器使用了专门窗口，这不好，要像CAN一样整合在Network中……没有规定好哪个节点在哪个frame上发是fibex xml的问题，不要为这种问题擦屁股"。落地：`FrTxMsg` 加 `node`（= `FrDb::frame_sender(该槽第一占用帧)`，与 CAN 的 `TxMsg.node` 同性质：Add 时导出、不入工程文件），Network 的 ECU 叶改成**可选中**，右侧详情对该 ECU 渲染 `ui/tx.rs::render_fr_ecu_generator`（Add 下拉只列**描述绑给它发送**的帧 + 与 CAN 同一套行控件）；`ui::tx::fr_section` 与 Interactive Generator 窗口里的 FlexRay 段**删除**。`add_fr_tx` 在描述没说明发送者时**拒绝添加**并把缺的是哪一条写在状态行（没加载描述 / 未调度这个槽 / 没说明由谁发送）——**不猜归属、不留飘在树外的条目**。树里的 ECU 行 = 描述声明的 ECU ∪ 现有条目的 `node`，所以换掉或撤掉描述不会把用户搭好的激励藏起来。选择状态另开一个字段 `App::net_fr_sel: Option<(u8, String)>`（CAN 节点索引与 cluster 里的 ECU 名是两套编号，不共用 `net_selected`；两边互斥，点一个清另一个）。
- **信号激励是真的**：`FrDb::encode_signal`（`decode::pack_raw` + `from_physical`，与 `frame_values` 严格互逆）+ `edit_signals(frame_ix)` 把 CAN 那套拖拽/激励/参数弹窗原样接过来——`ui/tx.rs::signal_rows` 抽成两边共用（含 `GenRow { Can | Fr }` 让两个弹窗认出行属于哪份清单；CAN 与 FR 的索引会撞，所以 widget id 带 `c`/`f` 前缀，hex 草稿另开 `fr_data_edit` 字段）。发射时按**该帧当时占槽的那一版**编码（`frame_ix_at(slot, cycle, 2)`），不是按编辑时看到的第一占用者——否则多帧共槽的槽会把值写进解码器不看的位置。
- **回放静音与 CAN 同规则**：`scan_log_ids` 现在一次扫描同时返回 CAN `(ch,id)` 与 FlexRay `(bus,slot)` 两份集合，条目命中即整个回放期间静默（chip=MUTE，On 保持），避免同一信号两个发送者混进每条曲线。
- **没有角色闸**：FlexRay 侧没有 DBC 节点也没有节点角色（Network 那棵树只列 ECU 声明，ECU 不是可切换的仿真对象），所以条目只有 On/MUTE 两种静默状态。**这不代表 Network 要长出角色下拉**——那条分工没变。
- **持久化**：`Config::fr_tx`（`#[serde(default)]`，旧工程读为空），且**恢复顺序在描述之后**（否则每条都会退化成 8 字节裸填充，名字与周期都丢了）——`apply` 里刻意放在 `push_fr_db_to_core()` 后面。
- **防线**：`a_generated_flexray_slot_fills_the_session_on_its_schedule`（默认周期、条目落在描述绑给它的那个 ECU 上、计数进 `frame_counter`、pin 的值读回来、Step 激励的两端都出现在发出去的字节的解码值里）、`a_flexray_slot_the_description_leaves_unowned_gets_no_entry`（没描述 / 没说明发送者两种都拒绝，状态行说清缺哪一条）、`a_flexray_entry_outlives_the_description_it_came_from`（撤掉描述后条目仍在、仍发原样字节、还挂在同一个 ECU 行下）、`a_generated_slot_stands_down_while_the_replayed_log_carries_it`（该槽只有日志那 4 帧，跨占用者求和，免得被调度相位骗）、`a_flexray_generator_entry_round_trips_through_a_project`（含 `node` 与"删掉 `fr_tx` 键的旧文件照读"）、`an_encoded_signal_reads_back_from_the_same_frame`（编解码互逆 + 未知信号/字节不够时拒绝）、`fr_db::every_occupant_of_a_slot_offers_its_signals` 补的 `frame_ix_of_slot` 断言、`the_network_view_draws_a_flexray_generator_row`（ui 床：选中 ECU → 面板、行、两个弹窗都画得出来）。**破测做过四处**：去掉 `!muted` → 静音那条红（11 vs 4）；把 `eval_phys(src, at_us)` 换成常量 → 主测试红（激励读到全 0）；把恢复的 `on: t.active` 换成 `false` → 持久化那条红；把"描述没说明发送者也照样建条目"改回去 → 归属那条红。**顺带发现的两条真实语义**：① 激励周期若正好等于槽周期，每帧都采在同一相位上，波形看着就是常量——采样混叠不是漏发，测试把激励周期设成 4× 槽周期才验得出两头；② 测试里给信号挑"代表值"不能用 `factor × n` 猜（自带 ARXML 的 EcoMode 只有 2 bit，5 被夹成 3），要用 `decode::to_physical(原始码字, …)` 反算，任何宽度/符号/因子下都精确。
- **仍没有的**：线缆上的发送（`xlFrTransmit` + active chip mode + 槽归属，等 S5 台架）、脚本侧 `fr_send`/`set_fr_sig`、动态段 minislot 发车。

### 2026-09-21：FlexRay 与 CAN 齐平的另一批（手选、事件、口径、发送者、菜单）

七次提交，全部由"用户已拍板的两件"+"第二轮盘点 B 类"推出来；细节与破测记录在 A/B 两处对应条目里（A1 `on fr slot`、A2 `Pick`、B4 录制白名单、B6 仅 DBC、B8 发送者）。今天新增的、盘点里没有的两件：**dbc_only 逐行问"有没有数据库说明这条到达"**（见 B6，这是行为变化）与 **Trace 的 FlexRay 行右键菜单补上 CAN 那一侧的动作为 "Watch FR{n} slot N" + "Clear filter"**（走 A2 的 `Pick`，加进本窗口 Manual 集合并切作用域；**故意做成增补而不是替换**——一键不该悄悄丢掉用户自己勾好的其他条目）。

**要看界面（无头跑不到，全量 660 通过只证明没弄坏别处）——✅ 用户 2026-09-21 逐条看过：都没问题**：
1. Trace 里**右键一条 FlexRay 行**：标题应是 `FR0 slot 13.2 · 帧名`，菜单四项（Watch / Clear filter / Copy payload / Copy slot.cycle），点 Watch 之后该窗只剩这个槽（连同已勾过的），左上作用域变成 `Manual (n)`。CAN 行的菜单没动。
2. **Messages/Trace 勾"仅 DBC"**：挂着描述的那一路，被调度的 FR 行现在**留在表里**（以前整列 FlexRay 一起消失）；"这一路没描述"和"这一相位未调度该帧"两种行仍然出去，展开行里那句说明会点明是哪一种。
3. Network 树 FlexRay 节**现在只剩 ECU 一组**（同一天里先做了按槽分组的调度表、随后被用户定掉，见下面"分工"）：展开应只有 `ECU (n)`，绑定得出发送方的行带"发 N 帧"（自带 PowerTrain 描述 48 帧里 12 帧绑得到）；加载一份 FIBEX 时那组显示"描述未声明 ECU"那句话。
4. 工具栏录制过滤框写 `FR0:13` 再录一段混合流量：文件里应只剩这一路的这个槽（Trace/统计仍看全部），CAN 帧不受影响（框里没写 CAN id 时 CAN 侧全录）。
5. Buses 窗口把某一路 FlexRay 改个名（例如"动力总成"）：Trace 的 Bus 列、Messages/Statistics 的行、作用域下拉、Bus Statistics 节标题、规格监视报告与两种 CSV 的 `bus` 列**都应跟着改名**（CAN 侧本来就是这个名字，FlexRay 现在同一条路）；**框下面那行灰字 `#n` 不变**（它不并排放在框后面：那要给输入框定宽，两张表的格子就对不齐了），右键菜单标题与触发器的 Bus 下拉写的是 `名字 (FRn)`——录制过滤的 `FR0:13` 与脚本的 `fr_sig(0, ..)` 认的仍是这个 n。清空那格或原样打回 `FRn` = 没名字，保存的工程文件里不写这一条。
6. Buses 窗口同日第三、四轮（按用户第二、三句意见改）：FlexRay 描述列的按钮现在就叫 **`Open...`**（与 CAN 那行同一个动作同一个词，不再"上面 Open、下面更换描述"），三行排版＝文件名 / `Open...` / `名字（N 帧）`（**不加"声明"之类前缀**：列头已经是 FIBEX/ARXML、行上又印着文件名），文件名悬停给完整路径＋声明的 cluster 名；**所有提示收成一句**（`+ Add FlexRay`、两行末的 `x`、挂接下拉、断开并移除、速率列、占用率）。
7. 之前欠的：脚本编辑器右栏的 **FlexRay tab**（已从"函数"下面挪出来单独成页）、Bus Statistics 的 FR 节、Specification 窗口的 FR 行、`on fr slot` 脚本在回放 BLF 时的反应（`examples/flexray_gateway.rxcan` 可直接挂）。

**分工（用户 2026-09-21 定，别再往回做）**：**Network 只列拓扑与 ECU，不排帧不排槽**——"具体看调度等信息是别的软件的事情，就像 CANoe 和 Fibex Explorer 的组合一样"。同一天里 `draw_flexray_section` 先按"参数行 / ECU / 帧按槽分组（带实测计数与周期）"做完并被提交（`11d5665`→`add0833` 一路），随后按这条删回只剩 ECU 一组；`FrDb::frame_sender` 留着，因为它现在服务的是 **ECU 行上的"发 N 帧"**（ECU 属性，不是调度表）。以后想再往 Network 里加"每槽有什么"之前先重读这条。**Buses 窗口同日一并收掉（用户第二句："让 Bus 中 FlexRay 的展示和 CAN 的一样，不用展示调度表"）**：底部那份"调度表"（各路全部帧的 slot/周期/重复/通道、启动帧高亮）删除，FlexRay 区改成**与上面 CAN 那张表同构的表格**——Name / FIBEX-ARXML / kbit/s·周期（**只读**：FlexRay 的位时就是调度表本身，改它得改描述文件）/ 硬件（选中空闲端口即挂，与 CAN 行同一动作形状；已挂的行是 `[V] ch{n}（只收）`+断开）/ 行末撤下描述。顺带清掉的：全局"给这路 [FR0*] 加载集群描述…"下拉、"挑描述并挂接…"合并按钮（两列各管各的，正是 CAN 的形状）、`App::fr_pick`/`App::fr_db_pick` 两个会话字段、`App::pick_fibex_for`（那个"替你挑第一个没描述的路"的入口本来就是"不替你猜"的反面）。行集合 = `fr_description_targets()`（已描述 ∪ 已监听 ∪ 日志里出现过的 ∪ 第一个空位），所以纯回放两路日志时第二条路就有一行、可以直接给它挂描述；既无描述又无监听的行标"（未配置）"。这条与上面 Network 那条是同一个分工判断，不是两次独立的删减。

### 2026-09-21 夜间批：队列任务"比 CANoe 还缺什么"（四件收掉，一叠记账）

队列里的原话："继续思考这个应用还有什么可以完善的，尤其是用户体验和 canoe 相比不足的部分，想办法改进"。先做了一次全量盘点（Trace 的行选择与导航、过滤与预设、Write 历史、回放控制、生成器入口、Buses/Network），逐条对着 CANoe 的同一面看。下面三件是"改得起、改完当场少一类踩坑"的，其余记在本批末尾的清单里，没有动手。

- **Trace 的精确地址写法 `id:` / `slot:`**：过滤框的普通文本一直是**子串**搜索，所以行右键的 "Filter this ID" 写进 `1AB` 会连 `1AB0`、`21AB` 一起留在屏幕上，FlexRay 那侧打个 `13` 命中槽 113 与 130——**"只看这一行"这个动作以前根本没有精确形式**。新增 `TraceFilter::exact: Option<ExactAddr>`（`Can{id,ext}` / `Fr(slot)`），菜单项改写 `id:1AB` / `id:1ABCDEFx`（**帧类别跟着走**：标准 0x1AB 与扩展 0x1AB 在表里是两行）；**用冒号不用等号**，因为 `Name=3` 在这个框里已经是信号值条件，而 DBC 里真会有叫 `Slot`、`ID` 的信号。两套编号依旧不互解：`id:` 让 FR 行全出去、`slot:` 让 CAN 行全出去（与 `(0,5)` 那几条老口径同一个立场，不新造翻译）。输入框的 hint 现在一行里给出三种写法。防线 `the_trace_row_menu_filters_to_the_row_it_was_opened_on`（带"子串搜索仍然是子串搜索"的对照断言，免得把老行为当 bug 修掉）与 `the_trace_filter_takes_an_exact_slot_or_id`（末尾另钉一条**语义**：`id:` / `slot:` 打一半、或后面那个数超出 u16 时，**不猜**——整串退回普通子串搜索，表空但框里就是用户打的字）；**破测**：把 `exact` 从 `trace_match_lens` / `trace_fr_match` 两处判定里摘掉，两条都红。
- **时间范围与两处展开随工程保存**：`TraceCfg` 补 `time_from` / `time_to` / `filters_open` / `fr_expand`（全部 `#[serde(default)]`）。原来的形状是"存了过滤文本，没存它旁边那两个秒数框"——重开工程后表格只剩 1.0–2.5 s 之外的一片空行，读起来像表坏了而不是过滤器在起作用。防线 `the_trace_time_range_and_expansion_round_trip`，含"把四个新键从 JSON 里删掉、当旧工程读回来"那一半。
- **状态消息不再只活一帧**：这次修的是一处**从没被渲染过**的报错。`Config::apply` 遇到描述文件丢失时写的是 `app.status`，而工程载入紧接着就把那一行盖成 "project loaded"——**用户永远看不到少了哪一份**；步进内部的消息（回放跑完、触发器命中、post-roll 结束）同理，下一帧就没了。落地：`CoreLoop::note(kind, text)` 成为唯一的"announce + 记录"缝，命令状态、两种驱动的步进状态、DBC 自动重载的提示、核心 panic 停机那句全走它（此前只有 `apply` 记环，`step` 侧与 threaded 侧不记）；前端侧新增 `App::fail`，把带原因的失败（工程读不到 / 解析不了 / 保存失败 / 导出失败 / 日志打不开 / 描述读取与解析失败）同时写进 Write 环，`App::report_load_problems` 则让每条载入问题单独成行、状态栏只留 `… · N 项未能载入（Write 窗口有明细）`。`Config::apply` 的签名因此变成 `-> Vec<String>`：**它自己没法报告，因为调用方下一句就盖栏**。防线三条：`a_step_message_stays_in_the_write_window`、`a_project_load_problem_outlives_the_loaded_line`、`a_refused_project_open_is_still_readable`，**逐条破测**（退回 `app.status = …` / `pending_status = Some(…)` / `self.status = …` 三处各红一条；第三条破测的输出直接指出旧环里只剩 "replaying" 和 "stopped"，正是那个被丢掉的消息）。threaded 那条缝（真 GUI 跑的分支）无头床跑不到，只能靠下面的屏幕检查。
- **收起的筛选行不再隐形，"Clear filter" 也不再只清一半**（本批第四件，与上一条同一立场：**看不见在生效 = 没说谎**）：payload / 帧类型 / 仅 DBC / 时间范围 挤在 `筛选` 按钮后面的折叠行里，行一收起控件没了、条件照旧在滤——上一件（把时间范围存进工程）反而把这处放大成"重开工程后表里少了一截而屏幕上没有任何原因"。落地两条：① `TraceWin::hidden_conds()` 建在 `filter_lens()` 上，**只报真在过滤的**（payload 写 `zz`、时间框写 `abc` 解析不出＝不在报，帧类型停在 Any、框留空也不在报），按钮写 `筛选 ·N`、悬停一行点名 `payload · 帧类型 · 仅 DBC · 时间范围`（主行上一直看得见的 文本/方向/作用域 不进去，`fr_expand` 是加行不是藏行、更不进去）；② 两个行菜单的 "Clear filter" 从各写一遍改成同一个 `ui::trace::clear_filter`（**两处手写过的同一动作已经漂过一次了**），抬走 文本 / payload / 帧类型 / 仅 DBC / 时间范围 / 方向 并把作用域放回 All，**唯独保留该窗口的 Manual 手选集合**（那是用户一条条勾出来的名单，不是一时输入的过滤式——与"Watch 那个槽是增补不是替换"同一条决定）。防线 `the_row_menu_clear_filter_lifts_every_condition`（**逐个条件单独验**：造一行让 文本 / payload / 帧类型 / 仅 DBC / 方向 / 时间范围 / 作用域 各自都能单独把它藏掉，断言"藏得住"再断言"Clear 抬得走"，最后单验手选集合活着）与 `a_closed_filter_row_names_the_conditions_still_working`；**破测两处各红**：把 `clear_filter` 退回旧的四字段 → 红在 "lifts payload"；把 `hidden_conds` 的 payload 判据换成"框里有字就算" → 红在"填了但解析不出来的不该算"。
- **盘点里没做、下次动手的顺序建议**：① Trace 的**行选中 / 光标 / 两个时间戳相减（Δt）**——CANoe 的 Trace 里用得最多的动作，我们目前只有右键复制（**Δt 与游标已做**：见 2026-09-22 两条游标批次；曲线侧的 A/B 也已做；**剩下的是"选中当前行"这一半**——高亮你点的那一行、以及按行导航）；② **跟随新行 / 滚动锚定**（长跑时表贴底会一直抖，向上翻又被拽回，现在只能按"暂停"）；③ **列冻结**（横向滚动后时间戳与 Name 跑出视野）；④ **命名/保存过滤预设**（工程里没有"这组过滤叫什么"的位置）；⑤ FlexRay 的**线缆侧发送**（S5，等台架与用户）；⑥ 脚本侧 `fr_send` / `set_fr_sig`；⑦ 回放块的 FlexRay 归属（等"块挂哪一路"这个决定）。

**要看界面（这一批：无头床只证明判定与持久化对了，屏幕上的形状没验）**：
1. Trace 里**右键一条 CAN 行**（标准、扩展各一次）：表里只剩那一行，过滤框里是 `id:1AB` 或 `id:1ABCDEFx`；手打 `slot:13` 时 CAN 行全出去、只剩这一路的槽 13（FlexRay 行的菜单仍旧只有 Watch / Clear filter，那一步走手打）。
2. 时间范围填 `1.0`/`2.5`、勾上 FR 信号展开，保存工程关掉重开：两个框里还是那两个数、行还是展开的（老工程文件照开）。
3. **打开一份缺文件的工程**（把 `.rxproj` 里 `config.fr_buses[].path` 改成一个不存在的路径）：状态栏出现 `project loaded: … · 1 项未能载入（Write 窗口有明细）`，Write 窗口里那条红色 error 写着是哪一个文件、解析器怎么说的。
4. **回放到末尾**：`replay finished at X.XXs` 之后还能在 Write 窗口翻到，不用在跑完那一瞬盯着状态栏。
5. **导出失败**（导到只读目录，或导进一个正被占用的文件名）：错误行留在 Write 窗口里。
6. 第五批欠的那条：Network 里点 FlexRay 的 ECU → 右侧生成器面板、Add 下拉只列"描述绑给它发"的帧、On/周期/Send now/信号激励与两个弹窗、回放时 MUTE chip。
7. 本批第四件的形状：Trace 窗口把 `筛选` 行**收起**、里面留着任一条件（填个时间范围 `1.0`/`2.5` 最直观）→ 按钮应写成 `筛选 ·1`，悬停一行点名；展开那行后按钮回到 `筛选`（条件看得见就不再报数）。然后**右键任意行 → Clear filter**：时间框、payload、帧类型、仅 DBC 都应被清空、行全部回来，而该窗口 Manual 里勾过的条目仍在（作用域下拉切到 Manual 能数出来）。CAN 与 FlexRay 两种行都试一次（两处现在共用一个实现）。

### 2026-09-22：Graphics 的两条游标（把"量一段时间"这件事补上）

用户点名要的功能：**"现在给Graphics加可显示两个游标的功能"**。原来的 `Cursor` 是**跟着鼠标走的一条悬停线**：手一离开图就没了，读到的数无法复述给别人，也量不出两点之间——那正是 CANoe 曲线窗口 X1/X2 游标对存在的理由。改成 **A/B 两条落在时间上的游标**（悬停线**删除**，一屏三条竖线是噪声；读数块右上角说清楚缺哪条）。

- **一个手势只动一样东西**（这条是全部交互规则）：按住某条线（±6 px 内）= 拖它，并且**该帧的平移让位**；双击空处 = 放游标（先填 A 再填 B，两条都有了动**离得远**的那条——近的那条本来就能拖）；其余的按住拖动照旧是平移。规则写成一个纯函数 `cursor_claim(held, double_clicked, mx, xs) -> PointerClaim{None|Drag(n)|Place(n)}`，**"拖赢放置"的顺序**也被测试钉住（在一条线上双击不该跳到另一条去）。
- **游标记的是时间不是像素**：平移/缩放/实时推进都让线跟着视图走，而它标的点不动；跑到视野外的游标**不画线**（时刻仍在读数块里——那才是读数的地方），`x_at_t` 因此**刻意不夹取**（夹了就把一个不在图上的测量画成贴在边界上，读起来像在量那个边界）。`t_at_x` 反过来夹：在数据区外面按，落在第一个/最后一个瞬间。
- **读数块**（右上角，避开左上角色例块与底部时间轴）：`A 1.000s  B 1.222s  Δt +222.000 ms`，下面每条曲线 `名字 A值 → B值 Δ差值`（Δ 带符号，B 在 A 左边就读负）。**缺的东西一律印 `-`**：没放的游标、那个时刻没有样本的曲线都不印 0（`0` 会被读成"它掉到 0 了"）。游标一条都没放时那块直接写 `双击图面放置游标`——**它出现在读数将出现的地方，不是别处的占位行**。
- **不随工程保存**（`show_cursor` 开关照旧存）：游标指的是采样环上的位置，新一轮测量一清，存回来的时刻就没人站着了。`GraphicsWindow::cursor_s: [Option<f64>; 2]` 在 `new_graphics_window` 与 `Config::apply` 两处都是 `[None, None]`。
- **防线**：`a_cursor_lands_where_the_pointer_was_and_keeps_its_time`（映射互逆 + 夹取方向）、`a_drag_grabs_the_nearest_cursor_line_only_when_it_is_under_the_pointer`（含"没放过的线不可 grab"、"两个都远 = 不归游标"）、`a_double_click_chooses_which_cursor_it_moves`、`a_press_owns_a_cursor_only_when_it_grabs_or_double_clicks_it`（`PointerClaim` 那条优先级）、`the_cursor_readout_gives_both_values_and_the_change_between_them`（含 µs/ms/s 换档、负 Δ、`-`）、`the_graphics_window_draws_a_cursor_pair`（ui 床：两条/一条/关掉三态都画得出来，且**画图不动测量**）。**破测做过四处**：去掉 6 px 闸 → "both out of reach" 红；`place_cursor` 的 `>` 翻成 `<` → 选错目标；`x_at_t` 加夹取 → 视野外那条断言红；`cursor_claim` 里把 Place 提到 Drag 前面 → "双击落在已抓的线上仍属拖" 红。
- **落地即被用户判为"拖动很卡"（同日即修，见弯路 25）**：原因是拖动归属按帧重算命中半径，快手上脱手、同一下按住被平移接走。修法是 `GraphicsWindow::cursor_drag` 记住这次按压归哪条线；端到端复现测试 `a_fast_drag_keeps_its_cursor_and_leaves_the_view_alone`（合成鼠标事件走真 `plot_area`，红时数字：`moved -0.004s，应 0.806s`），并把性能床那条改成 6 曲线 + 每帧移动游标做 A/B（**读数块不是瓶颈**，具体数字在弯路 25）。
- **仍欠的**：真实手感只能看屏幕（下面的检查 8）；**Trace 的游标/Δt** 是另一件事（曲线窗口量两条曲线之间的时间，Trace 表量两行之间的时间，CANoe 两个都有），仍列在本批末尾清单第①条。

**要看界面（这一批新增）**：
8. Graphics 窗口勾上 **Cursors**：图面右上角先出现 `A - B - 双击图面放置游标`；**双击**某处 → A 落在那儿（琥珀色竖线 + 顶部 `A` + 底部时刻），再双击别处 → B（青蓝色），读数块变成 `A …s  B …s  Δt …ms` 加每条曲线一行 `名字 A值 → B值 Δ`。按住某条线左右拖 → 只有那条跟着走、**画面不平移**；在空处按住拖 → 还是平移。放大缩小/推进实时时两条线随视图走而时刻不变；把某条拖出视野后它不画线但时刻还留在块里。窄窗口/一格一信号的堆叠布局下再看一眼那块有没有压到曲线。

### 2026-09-22：Trace 表也长了这对游标（盘点清单第①条做一次）

曲线侧的 A/B 刚落地，就把它补到表格里——CANoe 的 Trace 同样有游标与 ΔT，而"这两行之间过了多久"是查丢帧、查握手最常问的一句。**游标就是一个时刻**：`TraceWin::mark_us: [Option<u64>; 2]`（会话状态，与 Graphics 同一个不保存的理由——它指的是行环里的位置，新一轮测量一清就没人站着了）。放的手势换掉：表格没有可拖的线，所以是**行右键 → Set cursor A / Set cursor B**（CAN 与 FlexRay 两种行都给，一次右键只动一条），再设一次即移动；**Clear cursors** 只在有东西可清时出现（没有可清的东西就不占一行菜单）。

- **读数与曲线窗口共用同一句**：`cursor_head` / `fmt_dt` 从 `ui/graphics.rs` 提到 `ui/mod.rs`，两扇窗说同一个量用的是同一句话；**唯一参数是小数位**——图上 3 位（毫秒够用），表里 6 位（行与行就差微秒，四舍五入到毫秒会把 1.0004s 和 1.0001s 读成同一个时刻）。Δt 照旧带符号、缺的印 `-`，不拿 0 冒充没量的东西。
- **哪些行被涂**：`mark_tint(mark_us, t_us)` 纯函数（A 优先，两条同刻的行读作 A），底色**最后设**，所以被标的错误行仍然是"错误 + 游标"两件事都看得见；CAN 行与 FR 行各接一处，画床那条（`trace_draws_merged_flexray_rows_without_panicking`）把两个游标分别压在一条 CAN 行和一条 FR 行上跑五帧。
- **防线**：`a_cursor_tints_the_row_at_its_instant_on_either_bus`（钉住"按时刻不按行号"与 A/B 两色）、`the_cursor_header_reads_the_same_in_both_windows`（3 位与 6 位、`-`、负 Δt、µs/ms/s 换档）。**破测两处各红**：`position` 换成 `rposition` → 同刻读作 A 那条红（拿到 B 色）；`format!("{:.*}s", dec, t)` 写死 `{:.3}` → 6 位那条红（1.000123 被压成 1.000）。
- **仍欠**：游标的**手感与形状**只能看屏幕（下面检查 9）；表格里还没有**行选中/高亮当前行**（游标不是选中，CANoe 两个都有），清单第①条剩这一半。

**要看界面（这一批新增）**：
9. Trace 表里**右键一行 → Set cursor A**，再右键另一行 → **Set cursor B**：两行应有底色（A 琥珀、B 青蓝，与 Graphics 那对同色），表头 `N matching frames …` 后面多出 `A 1.234567s  B 1.456000s  Δt +221.433 ms`；再右键同一行选同一个游标应当是移动而不是新增；FlexRay 行同样试一次；右键**错误帧**行放一个游标，确认"红底 + 游标"两件事都还看得出来；**清空 Trace**（或开始新一轮测量）后读数那句**照旧留在表头**——那两个时刻仍是这一轮里真实的两个点，只是不再有行被涂色（游标不跟着清，要看没有旧行的表就自己按 Clear cursors）；窄窗口下看表头那行会不会换行挤掉别的说明。清干净用菜单里的 **Clear cursors**（没设时它不该出现）。

### 2026-09-22：FlexRay 的"什么时候发、发多宽"改回 FlexRay 自己的口径

用户给的方向（原话）：**"我感觉FlexRay对时序的要求比CAN的更强，所以它们发送的机制还是不一样的，尽量统一界面（降低理解负担），但是FlexRay的发送设定可以和CAN的不一样"**。第五批做生成器时把 CAN 行的两个设定原样抄了过来，那正是错的地方——所以这次**只改这两处，其余界面刻意保持一致**。

- **机制差异说清楚**：CAN 是争用总线，任何时刻都能请求发送，仲裁给优先级，"周期"只是请求的节奏，错过一帧只有规格监视知道；FlexRay 静态段是 TDMA，**槽号 + 基周期 + 重复数**由调度表规定，节点只能在自己那一拍发，载荷宽度也是它定的。于是抄来的三样里：`周期 ms` 单位错了、`0 = 事件触发` 语义不存在、"打的字节数决定 DLC"根本不允许。
- **周期改成数通信周期**：`cycle_modal` 的 FR 分支输入框标题 `周期`，Apply 写 `n × 周期时间`，旁边实时给 `每 n 个通信周期 = X ms`；`0` 被拒（句子说明"静态槽没有事件触发……要它不发用 On 勾选框"），行上的按钮从 `event` 改成 `不发`、`10 ms` 改成 `N 周期`（tooltip 给 ms 与理由）。恢复按钮跟着改成 `描述 {n} 周期`。**没有描述或没声明周期时间就没有可数的网格**：退回按 ms 编辑并在 tooltip 说明原因，不凭空造一个网格（`fr_cycle_time_us` 返回 `None` 就是这条路）。新增 `App::fr_declared_len`、纯函数 `generator::fr_cycle_draft`（文本→(周期数, µs)，带拒绝理由）与 `generator::cycles_of`（µs→周期数，**只在整倍数上说得出话**，7.5 ms 对 5 ms 网格是 `None`，于是行显示 `7 ms` 而不是四舍五入成"每 2 周期"骗人）。
- **载荷宽度归调度表**：`set_fr_tx_base(tx, data, len)` 现在**按声明宽度补 0 且不再改 `tx.len`**（旧代码 `tx.len = data.len()`，打 3 字节就把 26 字节的槽缩成 3 字节——那是 CAN 的 DLC 习惯漏进了 FR）；`SetFrEntryHex` 改由 `fr_hex_refusal` 给理由（空 / 超宽 / 不是 hex 三种分开说），理由走命令状态 → 状态栏 + Write 环（上一批那条 `note` 缝在这儿正好接上）。hex 框的悬停先说清"载荷 n 字节（这个槽的宽度由调度表规定）：不足补 0，多出来的不收"。
- **两条被改写的旧断言（重要，别改回去）**：`a_flexray_entry_outlives_the_description_it_came_from` 与 `a_flexray_generator_entry_round_trips_through_a_project` 原来都断言"打的字节就是发出去的字节 / `data_text == "AA 55"`"，那正是被推翻的旧语义。两条都改成**在打字之前先记下槽的宽度**再断言它没变（`left: 3, right: 26` 就是破测时该红的样子）。**踩到的坑**：第一版我把断言写成 `agg.payload.len() == app.fr_tx_list[0].len`——自己跟自己比，破测照样绿；必须拿"打字之前"的数当参照。
- **防线**：`the_flexray_cycle_box_counts_cycles_and_refuses_event_fills`（1/2/12 周期、0 与空与 `abc` 与 `1e6` 与超上限各拒其由、无网格时拒数、`cycles_of` 的三种 `None`）、`a_flexray_payload_is_bounded_by_its_slot_width`（补 0、超宽被拒且**内容没变**、状态行带那个数字、非 hex 另给理由），加上面两条改写过的旧测试。**破测做过四处**：去掉 `data.resize(len,0)`/改回 `tx.len = data.len()` → 三条测试同时红（3 vs 26、2 vs 26）；摘掉 `fr_hex_refusal` → 状态行那条红；让 `fr_cycle_draft` 接受 0 → 周期框那条红；`cycles_of` 去掉整倍数条件 → "off the grid" 那条红。
- **刻意没动的**：行的列顺序与形状、On / MUTE / 总关 三个 chip 的词、Add、x、信号拖拽与激励那套（这部分抄 CAN 抄对了，语义没变形）、`生成器：N/M 发送中` 摘要。差异只落在"一个控件 + 一行读数 + 一个框的边界"。**建议过但等用户定口径的**：把 `Send now` 换成 FR 真正需要的"跳过下 N 拍"（丢一拍触发接收侧 Missing/Dropped）、行内反馈改周期口径（`第 12 拍该发 · 实发 11 · 缺 1`）、把"一个槽按相位轮着排哪几帧"在行里展开、脚本侧 `fr_send`/`set_fr_sig`。

**要看界面（这一批新增）**：
10. Network 里选一个有描述绑定的 ECU → Add 一个 FlexRay 槽 → 点行上的周期按钮：标题应是 `Frame_…  slot 82  on FR0`，输入框标 `周期`，填 `2` 时旁边出 `每 2 个通信周期 = 10 ms`，填 `0` 时那行橙字说明"静态槽没有事件触发……"且 Apply 灰掉；`描述 N 周期` 按钮只在当前值与声明不一致时出现。行上的周期按钮应显示 `N 周期`（悬停给 ms），撤掉描述后应显示 ms 并说明没有网格。
11. 同一行没有信号时展开 hex 框：悬停应写"载荷 N 字节……"；打 `11 22 33` 应看到框里补零到 N 字节、行仍按 N 字节发；打 N+1 个 `AA` 应被拒且**框里的内容回到原样**、状态栏出现带那个数字的一句（Write 窗口里也留一条）。

### 2026-09-22：hint 不再被自己的框裁掉 + 信号值条件两种总线都算

用户从建议清单里点了 2 和 4（原话：**"做2、4"**）。

- **2 —— 定宽框裁掉自己的 hint**。`ui::hint_width(ui, HINT, min)`：用**活字体**量 hint 的宽度（`calc_text_size`），加 12 px 边框余量，再取"布局原本想要的宽度"作下限。改了六处：Trace 过滤框（120 px 装 `名称 / hex 子串 · id:1AB · slot:13`，中文更宽，等于只剩半句）、payload 框（84 px）、时间两框（52 px）、工具栏录制过滤（110 px 装 `id / FR slot filter`——**它是行末最后一个控件，改成吃该行剩下的宽度**，下限仍是 hint 需要的）、回放块的 id 过滤框（140 px 装 `id 过滤，如 100, 3F4x`）。**没改的**：idfilter/信号搜索（本来就 `content_region_avail` 满宽）、sysvars 的 Min/Max（`-1.0` 填满）、以及量过确认放得下的那几个（`hex id`、`search name / ID`、`0, 30, 60, 90`、`新名字`）。理由写进 helper 的注释：**被裁一半的 hint 比没有 hint 更坏**——它读起来像框里的内容，而不是提示；而这几个 hint 恰好是几处唯一讲语法的地方（`id:`/`slot:` 就是上一批新加的）。行放不下时 imgui 换行而不是裁掉，所以变宽是安全的降级。
- **4 —— `Name>10` 不再是 CAN 专属**。旧行为是"框里有值条件就让所有 FR 行出去"，文档写的口径也是那句（"那是 CAN 侧的判据"）。现在 FR 行走同一条判定：`frame_ix_at(slot, cycle, ab)` 解析出**这一帧**（不是槽的第一占用者），`decode_signals` 取物理值比较——**与曲线、与展开子行印的同一个数**，所以不会"过滤留下它、图上看着不对"。这一帧没这个信号 / 这一路没挂描述 / 调度表解不出这一相位 → 该行不出，与 CAN 侧"DBC 解不出这个报文就 drop"同一条规则（**不留后门**）。比较抽成 `CmpOp::holds(v, want)`，两边共用，同一个写法在两种电缆上不可能有第二种含义。
- **口径变化要同时改文档两处**：《Trace/FlexRay 显示》里那句"写信号值条件时 FR 行不出"是旧口径的原文，已改写；《Trace 过滤》补一条新的（含"多个条件是且"与解不开时的行为）。**注意**：这条旧口径**没有测试钉过**（全量跑一遍无人红），所以它只是文档里活着、破测时才被换掉——以后写"某判据不适用于 X"这类口径时，顺手钉一条断言，否则它就是暗的。
- **防线**：`a_flexray_row_is_filtered_by_the_value_its_frame_carries`（自带描述里真解得开的第一路信号，零载荷读出的那个值当基准：`> phys-1` 留、`> phys` 走（`>` 严格）、`>= phys` 留、`== phys` 留、`NoSuchSignal>0` 走、没描述的 7 路走、但**同一个行在没有值条件时留下**——证明是条件在藏它，不是文件缺失）。**破测两处各红**：把名字查找换成恒不命中（＝旧的整批 drop）→ 第一条断言红；`CmpOp::Gt` 写成 `>=` → "严格大于"那条红。

**要看界面（这一批新增）**：
12. Trace 窗口：过滤框的 hint 现在应完整可见（框变宽了）——在**窄** Trace 窗口里看一眼这一行是否换到第二行（换行可接受，裁字不可接受）；工具栏录制行最后那个框应长到贴住窗口右边缘，`id / FR slot filter` 一句看完；回放块的 id 框同理。
13. 挂上 `assets/arxml/PowerTrain.arxml` 后回放一段 FlexRay：在 Trace 过滤框写某个 FR 信号名加条件（例如 `CarSpeed>0`，名字从展开子行里抄）——FR 行应留下能解码的那些、没有该信号/没描述的行出去，且**留下的那些展开子行里的数值确实满足条件**（这是"同一个数"的肉眼验证）。

### 2026-09-30：FlexRay 一路能被删除了（并且删除不重编号）

用户原话："**没有两台 VN7640，没法实现 FlexRay 的硬件联通，但是发送 flexray 消息、删除 flexray 总线的功能都可以做**"。这一条做**删除**；发送另立一条。

- **设计决定：删除 FlexRay 路不搬家编号，与 CAN 的 `remove_channel` 相反**。CAN 的通道号是**列表位置**，删一条就前移，前端跟着重映射；FlexRay 的"路号"是 **cluster 身份**——BLF 给每一帧盖上它自己的 `clusterNo`（`src/log/blf.rs`，读侧就是 `bus = cluster_no`），脚本里打的是 `fr_sig(1, 13, ..)`、`on fr 1 slot 13`，录制过滤框打的是 `FR1:13`，触发器存的也是这个数字。把这些整体前移一格，等于让**已经打进文本里**的引用悄悄指向另一张网——这才是最坏的失败。所以 `remove_fr_bus` 只删引用这一路的东西，别的一路都不动；空出来的号由 `next_flexray_bus()`（第一条没有描述的路）在下次 `+ Add FlexRay` 时填上，号因此仍然小而密，只是**不会被人搬动**。工程文件本来就逐条写 `bus`，空档存得下来、读得回来（`a_removed_flexray_bus_does_not_renumber_the_survivor_through_a_save`）。
- **两个动词与 CAN 行对齐**（同一动作在两处用两个词会被读成两件事）：硬件列的按钮从 `断开并移除` 改成 **`解挂`**——与 CAN 行同一个词、同一件事：关端口，路、描述、曲线全留；行末 `x` = **删除这一路**（端口 + 描述 + 引用它槽号的一切）。`DETACH_LABEL`（原 `FR_DETACH_LABEL`）现在两张表共用一个常量，拒绝文案"先点本行的解挂"照旧对得上屏幕上的字。原先"只移除描述、不动监听"那个第三态一并取消：要那个效果就 `解挂`，要彻底删掉就 `x`。
- **删除到底删了什么**（两边清点，全部有断言）：核心侧 `hw.detach_fr`（端口随路关）+ `fr_dbs` + `fr_loads`（置 dirty）+ `fr_trace` 里这一路的行（新增 `FrRing::drop_bus`，**不计入 `dropped`**——那个数的含义是"因容量被裁"）+ 由幸存行重建 `fr_aggs` + `subs` 里的 `SigKey::Fr{bus}` + `spec.fr_rows`/`fr_previous`（`Spec::drop_fr_bus`）+ `fr_tx_list` + `injected_fr` + `replay_fr_ids` + 条件指向这一路的触发规则。前端侧 `fr_buses`/`fr_names`/`fr_name_edit`/`fr_tx_pick`/`net_fr_sel`/`trig_draft`（编辑器开着且写的就是这一路）+ 三个窗口的 `Pick::Fr` 手选集合 + Graphics/Data/State 的 `SigKey::Fr` 曲线与 State 那张按 key 存的色号/规则/覆盖表 + Monitor 行 + 作用域退回 All（`reset_fr_scope`；只有整路删除才退，`解挂` 不退）。
- **两处故意不动**，理由写进 `remove_fr_bus` 的注释：脚本源码里的 `on fr 1 slot 5` / `fr_sig(1, ..)`，和录制过滤框里的 `FR1:5`。它们是**打字进来的引用**，不是按 key 存的表，静默改写用户打的字比留着更坏；删掉这一路后那个号没有描述也没有端口，于是匹配不到任何东西（若按同一个号新加一路，它们就重新生效——框里写的是什么，行为就是什么）。
- **状态行由核心写**，并把用户给这路起的名字一起报（`RemoveFrBus` 带 `label`，前端在下发前解析好 `fr_bus_label`）：线程化前端里前端自己写的状态会被一帧后到达的核心快照冲掉，"谁最后写条谁说话"这条老规则又一次说了算。
- **防线**：`removing_a_flexray_bus_takes_everything_keyed_on_it`（两路各挂 mock 监听、各有曲线/手选/订阅/发送条目/规则/规格判定，各来一帧真流量，删低号那路：幸存路的**号**、行、聚合、订阅、曲线、白名单、规则、规格行与间隔记忆全在，被删的全没，`fr_bus_rows()==[1]`、`next_flexray_bus()==Some(0)`）。规格那两笔**踩到一个坑**：`start_virtual` 走 `reset_run` 会整个换掉 `Spec`，所以测试里的判定与 `fr_previous` 必须**在最后一次步进之后**种、并且种在**没有流量的槽**上，否则断言检查的是监视器自己的账，不是测试种的账。`a_flexray_bus_can_be_detached_and_removed`（原 `a_described_bus_can_be_attached_or_forgot`）按新两动词重写：解挂关端口留描述，删除两者都关；被删掉的那条"监听中拒绝单独移除描述"的拒绝文案随第三个动作一起退役。**破测七处各红**：注释掉 `FrRing::drop_bus` → "FR0's rows are out of the ring"；注释掉前端曲线清理 → "only FR1's curve is left"；`triggers.retain` → "a rule about a cluster that is gone can never fire"；`spec.drop_fr_bus` → "no verdict about a cluster that is gone"；`subs.retain` → "FR0's subscription is gone"；`fr_tx_list.retain` → "FR0's generator entry went with it"；前端 `fr_names.remove` → 工程 JSON 里还留着已删那路的名字。

**要看界面（这一批新增）**：
14. Buses 窗口 FlexRay 区：已挂接的行应是**两行布局**（第一行 `[V] ch2（只收）`，第二行 `解挂`），窄窗口下按钮不被格子裁掉（与 CAN 行同形）；点 `解挂` 后这一行**还在**、描述列的 `Open...` 还能用、硬件列换回空闲端口下拉；点行末 `x` 后整行消失，状态行写 `FlexRay 路 <名字> 已移除`（改过名就用名字）。旧文案"断开并移除"不应再出现在界面上。
15. 两路都在时删掉**号小的**那路：剩下那行灰字里的 `#n` **不变**（还是 `#1`），它的曲线、Trace/Messages 里的行、发送条目都还在；被删那路的曲线从图例消失，作用域指着它的窗口退回 All。

### 2026-09-30：脚本能发 FlexRay 了（`fr_send` / `set_fr_sig`）

用户在两条路之间点了**脚本发车**，并明确"拿不到 vxlapi.h，先别猜布局"——所以**上线发车（`xlFrTransmit`）继续挂在 S5**，这一批只做不需要硬件、也不需要猜内存布局的那一半。

- **两个内建，都追加在 `HOST_FNS` 末尾**：编译器按**位置**把名字解析成 id，中间插一条会让已存工程的字节码全体错位——这条规矩写进了表旁边的注释。
  - `fr_send(路, 槽, b0..b7)` / `fr_send(路, 槽, buf)`：内核只校数字与 254 字节上限，**宽度归描述**（内核看不见描述）。
  - `set_fr_sig(路, 槽, "名", 值)`：值经 `FrDb::encode_signal` 编码进**该槽的生成器条目基载荷**，与 Network 里 ECU 面板改一个信号值是同一件事（复用 `set_fr_tx_base`，所以条目宽度照旧不动）。
- **发送口径 = 生成器条目的口径**：帧从 `accept_fr_row` 那个唯一的接收漏斗进会话，于是 Trace/Messages/统计/负载/规格/触发/`on fr slot`/录制全都看见它，而**它不出线缆**（面板标题那句"本会话的流量"照样是真的）。三个派发点（节点定时器、CAN 到达、FlexRay 到达）各排入 `fr_from_scripts`，由**该步或下一步的生成块**收走——因为 FlexRay 端口轮询排在生成块之前、而 CAN 帧遍历排在它之后。这条节奏同时就是**自激上界**：一个"收到自己发的帧就再发一次"的脚本每步最多多一帧，不需要再造一个轮次上限（实测：把丢弃规则去掉后日志里正是 `[1,3,3,3]` 这种每步一条）。
- **两种丢弃都写进节点日志，不静默**：载荷比该槽声明的宽 → **整帧丢弃** + 一行"FR{路} slot N 声明 W 字节，脚本给了 M"；`set_fr_sig` 缺描述 / 缺条目 / 那个帧不声明这个名字 → 整条写入丢弃 + 一行原因（**先问描述再问条目**：路都不存在时"没有集群描述"才是用户该补的那一件）。截断会发出谁都没要过的帧，所以宁可一帧不发——与 #42 给 hex 框定的那条同一条口径。
- **脚本造不出一条路**（补做，破测过）：`fr_send` 到一个既没描述也没监听的路号，原先会照常 ingest，于是 `fr_loads` 有了 FR7 的键、Buses 表多出一行写着"日志里有这路流量"——**替一段打错的脚本编出一条没人收过的总线**。现在核心侧按"描述 ∪ 监听"认路（与前端 `fr_bus_rows` 同一个判据），不认就整帧丢弃 + 日志一行"FR7 不是已配置的路（先加载集群描述或挂监听）"。**槽**没排帧不算：路在、槽就照打的长度发（`a_script_frame_enters_the_session_like_any_arrival` 里那个 slot 6 正是这一条）。防线 `a_script_frame_for_an_unconfigured_bus_is_refused`；把 `known` 写成恒真 → "FR7 was never invented: [0, 7, 0, 7, …]"，一眼看得见 phantom。
- **`fr_out`/`fr_sig` 两条队列的生命周期照 `emitted` 抄**（`drain_vm` 挪进 runtime，`take_*` 交给总线），所以 `on start` 里的写入要等第一次派发才被取走——与 `emit_value` 现状一致，不是新坑也没有顺手改。
- **防线**（5 条，**破测 4 处各红**）：`fr_send_queues_the_slot_address_and_payload`（数字/字节/缓冲三形 + 4 条越界与空载荷 + 编译期 arity 拦 `fr_send(0,5)`）、`set_fr_sig_queues_the_write`（Int 值落成物理数 + 四类错参数）、`a_script_frame_enters_the_session_like_any_arrival`（到达唤醒的处理器同一步就看见自己发的帧；槽 6 无描述→照打的长度；且**不级联**）、`a_script_frame_wider_than_its_slot_is_dropped_with_the_reason`（3 字节的帧从未进表 + 日志有那句 + 节点没熔断）、`a_script_writes_a_flexray_signal_into_its_entry`（自带 ARXML 里取一个"有发送 ECU 且信号宽度够"的槽：写值读回同一个数、条目基载荷变了、`fr_send` 那一帧补齐到声明的 26 字节）、`a_flexray_signal_write_without_its_entry_is_refused`（两种缺失各一行原因，且不 invented 条目）。破测：注释掉 FlexRay 派发点的 `drain_node_fr` → 会话里没有出现过的帧；把宽度 `match` 换成 `let payload = data` → 三条断言红（3 字节进了表、补齐长度变成 1）；让 `apply_fr_sig_writes` 不写 → 条目载荷没变红。
- **界面一侧**：脚本编辑器"函数"里的 总线控制 段现在多两个可点条目（`fr_send(0, 0, 0x00);` / `set_fr_sig(0, 0, "Signal", 0)`）。清单本身由 `HOST_FNS` 生成，无需另处登记。

**要看界面（这一批新增）**：
16. 脚本编辑器：函数列表里选 `fr_send` / `set_fr_sig` 插入的代码能直接编译（右栏 FlexRay 信号拖进去当第三个参数）；跑一段带 `fr_send` 的脚本，节点日志能看到丢弃原因（如果故意给宽载荷）。

### 2026-09-30 夜间批：点一行就是"这一行"（Trace 的选中、↑/↓ 走行、Ctrl+C 复制）

用户下班前把队列交给我（"现在是晚上，我要去睡觉了，你可以尽情发挥"），队列原话仍是"**继续思考这个应用还有什么可以完善的，尤其是用户体验和 canoe 相比不足的部分**"。挑的是《2026-09-21 夜间批》盘点里剩下的 **①**（行选中 / 按行导航——"CANoe 的 Trace 里用得最多的动作，我们目前只有右键复制"）。②③④（跟随/滚动锚定、列冻结、命名过滤预设）留在队列里：它们都要用户在屏幕上定形状，不适合夜间单方面改。

- **状态**：`TraceWin.pick: Option<TracePick { t_us, fr }>`，**会话状态**、理由与 `mark_us` 同一条（它指进行环里的位置，新一轮测量一清就没人在那儿了），所以工程文件不动、旧工程照开。
- **键里为什么带"哪一行"（不是只带时刻）**：仿真器把**一步之内的所有帧盖同一个微秒**，所以同一时刻多条行是常态而不是巧合。第一版键只有 `(时刻, 哪一路)`，**release 起来自己截图看**就露馅了：点 61.820000 的 CAN1 id 200，id 100 那行同时亮了，按一次 ↓ 跳两行。现在键是 `(时刻, 总线/路, 地址, 是否 FlexRay)`——两套编号不互解（`0x100` 与槽 256 是两回事），而展开的 FlexRay **子行**故意与父行同键：那一组就是一帧到达，一起亮、一起被一步跳过（`TraceRow` 缓存本来就是"一帧一条"，子行只是画出来的）。教训写在这：**"屏幕上才看得见"的缺陷，测试全绿也照样在**——能自己起 GUI 截图核对的形状，别留给用户早上第一次点击。
- **底色用表格的"交替底色"那一格（bg0），游标/错误/远程用另一格（bg1）**：读的是 imgui `TableSetBgColor` 的实现而不是猜——`RowBg0` 覆盖的是按奇偶交替的那层，`RowBg1` 是叠在它上面的另一层，所以**选中 + 标记 + 错误行三件事能同时看得见**，不需要为"哪个覆盖哪个"排优先级，也不需要动任何原生样式（那条老规矩：统一外观靠选原生控件与布局，不靠改写原生配色）。
- **判定逻辑在纯函数里**（`workspace::step_pick`）：↓ 往旧的方向（列表新行在上）、**两头夹紧**（走到底不清空选中）、没有选中时从最新一行落地、**选中那行已经离开表**（过滤掉了 / 环裁了 / 清了）也重新落在最新一行——"键盘走进死路会被读成控件坏了"。一次按键最坏线性扫一遍缓存（≤200k 行，~0.2 ms），只在按键那帧发生。
- **接管条件**：`is_window_hovered() && !is_any_item_active()`——指针不在这个窗口就不抢键，有输入框在打字时 ↑/↓ 与 Ctrl+C 仍是**它自己的**（把过滤框的方向键抢走就是给用户下套）。
- **复制**：`fmt_fr_row` 与 CAN 侧 `fmt_row` **同样八列、同样顺序**（时间/Bus(带 A/B)/slot.cycle/名/长度/`-`/hex/`Rx`），Bus 那格与表格里同一函数出串，所以"复制出来的就是看见的"；顺手给 FlexRay 行的右键菜单补上 **Copy row**——同一个动作在两处用了两个词会被读成两件事，而 CAN 行一直有这一项。
- **可发现性**：表头只在**有选中时**多一句 `已选中一行 · ↑/↓ 换行 · Ctrl+C 复制该行`。底色本身教不了按键，而常驻一句关于不在的状态的提示是噪声。
- **防线**：`the_keyboard_walks_the_row_list_and_clamps_at_both_ends`、`a_pick_names_one_row_at_a_shared_instant`（同一微秒的 CAN1 id 100 / CAN1 id 200 / FR0 槽 13 三个键两两不同，且 ↓ 一次只走一行）、`a_copied_flexray_row_carries_the_table_columns`（自带一份合成描述，断言逐列内容与"描述名优先于日志自带名"，不依赖资产文件）、`ui_tests::a_picked_row_draws_on_both_streams_beside_the_marks`（错误行 + 游标 A/B + 选中同时存在的两条绘制路径，含展开子行）。**破测三处各红**：`step_pick` 去掉 `.or(from)`（夹紧）→ "and down at the bottom does too"；去掉 `rows.front()` 重落点 → "the first press picks the newest row"；把 `addr` 从 CAN 键里抹掉 → "two CAN ids in the same microsecond"（这条就是屏幕上看到的那个缺陷，现在由断言钉住）。合成 db 而非真资产这一步是有意的：`assets/arxml/*.arxml` 里那份是 GBK，历史上出过一次"读不到就跳过"的假绿（见《弯路记录》那条）。

**要看界面（这一批新增）**：
17. Trace 表：点一行 → 该行亮起蓝底，表头出现 `已选中一行 · ↑/↓ 换行 · Ctrl+C 复制该行`；按 ↓ 逐行往下走（新行在上，↓ 走向旧的），到底不动；勾上 FR 信号展开后， ↓ 是**整帧跳**（父行 + 它的信号子行一起亮、一起过）；再点另一条总线里同一微秒的行，只亮你点的那一组；右键 → Copy row（FlexRay 行现在也有这项）与 Ctrl+C 贴出来的是同一串，八列与表里对得上；在过滤框里打字时按 ↑/↓ 与 Ctrl+C 应该**照旧是输入框的**（表不动）。

## 备注

- UI 无 imgui 自动化测试床：部分补齐（`src/ui_tests.rs` 无头冒烟床——imgui 不接渲染器也能逐帧跑真实绘制路径，全窗口齐开/脚本编辑器/控件交互等场景由测试守护）。控件交互（点击、选区、输入法）仍靠人工验收，判定逻辑用无头测试自动证明。
- 唤醒精度远期可选：事件驱动核心的"最后 <1 ms 自旋收尾"开关（详见 `docs/architecture.md` 核心时钟模型）。
- CTE 宽字形补丁历史：曾以本地 fork（chemPolonium/dear-imgui-cte-sys）挂接，上游 goossens/ImGuiColorTextEdit #88 合并并经 dear-imgui-rs 0.18.0（源码 patch 22e98fb）带入 crates.io 后，fork 已退役（2026-09-15）。
