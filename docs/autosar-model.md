# AUTOSAR 统一模型（第 0/1 阶段冻结稿）

日期：2026-10-29
基线：`roxy-can` @ `21580f6`
上游计划：[`docs/autosar-plan.md`](autosar-plan.md)

本文是第 0 阶段的产物，也是第 1 阶段的编码契约。它把“特性支持到哪一层、
实体长什么样、ID 和 key 怎么表示、工程文件怎么写、旧工程怎么搬”固定下来。
除非在评审中明确推翻，后续阶段按本文实现，不再临时改形状。

状态标记：

- **冻结**：第 1 阶段按此编码，改动需要改本文并补迁移。
- **待样本**：字段形状已定，深度要等真实 ARXML 验证。
- **明确不做**：导入时给诊断，不进模型。

---

## 1. 特性表

深度代码：

| 代码 | 含义 |
| --- | --- |
| `R` | 读入、解码、编码、观测、发送全链路 |
| `D` | 只解码/展示，不参与编码与发送 |
| `M` | 只进模型（名称、关系、元数据），运行时不执行 |
| `I` | 只用于导入匹配/诊断，不进入运行时 |
| `X` | 不支持；遇到时给出明确诊断并跳过 |

### 1.1 拓扑与节点

| 特性 | ARXML 来源 | 模型落点 | 深度 | 说明 |
| --- | --- | --- | --- | --- |
| Cluster | `CAN-CLUSTER`、`FLEXRAY-CLUSTER`、`ETHERNET-CLUSTER` | `Cluster` | `R` | 统一总线列表的唯一身份；kind 只是属性 |
| Network / 物理通道 | `*-PHYSICAL-CHANNEL`、cluster conditional | `Network` | `R` | 一 cluster 至少一 network；ETH 的 VLAN 在 physical channel |
| ECU | `ECU-INSTANCE` | `Ecu` | `M` | 发送方判定、Trace 过滤、矩阵展示 |
| ECU 连接器 | `COMMUNICATION-CONNECTOR`、`COUPLING-PORT` | `Connector` | `M` | 含 ETH MAC / VLAN membership / default VLAN |
| CAN 位率 / FD | `BAUDRATE`、`CAN-FD-BAUDRATE` | `Network::Can` | `R` | 负载统计与 FD 判定 |
| FlexRay 参数 | cluster 的 `SPEED`、`gdCycle` 等 | `Network::FlexRay` | `R` | 槽/周期解码、硬件参数 |
| Ethernet PHY / VLAN | `PHYSICAL-LAYER`、`VLAN-IDENTIFIER` | `Network::Ethernet` | `R` | 帧匹配打分 |
| MAC / EtherType | ARXML 只有 ECU 单播 MAC；EtherType 不在 ARXML | `Connector::mac`、`Frame::ethertype_hint` | `I` | EtherType 由项目覆盖或捕获头提供 |

### 1.2 帧

| 特性 | ARXML 来源 | 模型落点 | 深度 | 说明 |
| --- | --- | --- | --- | --- |
| CAN 标准/扩展帧 | `CAN-FRAME` + `CAN-FRAME-TRIGGERING` | `Frame::Can` | `R` | ID、extended、FD、周期 |
| CAN FD BRS | `CAN-FD-FRAME-SUPPORT`、触发参数 | `Frame::Can::fd` | `R` | BRS 是发送参数，不改变位布局 |
| FlexRay 帧 | `FLEXRAY-FRAME` + `FLEXRAY-FRAME-TRIGGERING` | `Frame::FlexRay` | `R` | slot、base cycle、repetition、payload preamble |
| Ethernet 帧 | `ETHERNET-FRAME` / `GENERIC-` / `USER-DEFINED-` / `IEEE-1722-TP-` | `Frame::Ethernet` | `R` | 四种标签都进同一模型，保留 variant |
| IEEE 1722 元数据 | 1722 帧的 stream id、sub-type、version、presentation time | `EthernetFrameMeta::Ieee1722` | `D` | 只展示，不凭它匹配 |
| 帧→PDU 映射 | `PDU-TO-FRAME-MAPPING` | `FramePduMapping` | `R` | `START-POSITION` 按位；CAN 1:1，其余 1:N |
| 帧触发周期 | `PDU-TRIGGERING`、`FRAME-TRIGGERING` | `FrameTiming` | `M` | 解码不需要；统计、Generator 默认值使用 |
| Frame Port / 收发方 | `FRAME-PORT`、`FRAME-ECU-RELATION` | `FrameEcuRelation` | `M` | 优先于 PDU group 推断 |

### 1.3 PDU 与信号

| 特性 | ARXML 来源 | 模型落点 | 深度 | 说明 |
| --- | --- | --- | --- | --- |
| 信号 PDU | `I-SIGNAL-I-PDU` | `Pdu::Signal` | `R` | 基本通信单元 |
| 信号→PDU 映射 | `I-SIGNAL-TO-I-PDU-MAPPING` | `SignalPduMapping` | `R` | 起始位、字节序、update bit |
| I-Signal 布局 | `I-SIGNAL/LENGTH` | `Signal` | `R` | 位长、符号、浮点来自 base type |
| CompuMethod | `COMPU-METHOD` | `Signal::compu` | `R` | identity / linear / text table；其余降级 raw |
| BaseType | `SW-BASE-TYPE` | `Signal::base_type` | `R` | `2C` / `NONE` / `IEEE754` 决定符号与浮点 |
| 初始值 | `I-SIGNAL/INIT-VALUE`、DBC `GenSigStartValue` | `Signal::init` | `R` | 原始值存模型，物理值按 compu 换算 |
| 量程 | `DATA-CONSTR`、compu 的 `LOWER-LIMIT`/`UPPER-LIMIT`、DBC `[min|max]` | `Signal::min/max` | `R` | 开区间标志保留；没有声明就是 `None` |
| 单位 | `UNIT/DISPLAY-NAME` | `Signal::unit` | `R` | 只存显示名 |
| 文本值表 | `COMPU-SCALE` 的 `VT` | `Signal::values` | `R` | 原始整数→文本 |
| SystemSignal 扇出 | `SYSTEM-SIGNAL` + 多个 `I-SIGNAL` | `SystemSignal` | `M` | 同一逻辑信号的多个网络实例；显示时合并分组 |
| Signal Group | `I-SIGNAL-GROUP` | `SignalGroup` | `M`（第 6 阶段 `R`） | 成员与组级发送方 |
| Update bit | `UPDATE-INDICATION-BIT-POSITION` | `SignalPduMapping::update_bit` | `M` | 当前解码忽略，保留位置 |
| 注释 | `DESC/L-2` | `Description` | `M` | 目录悬停使用 |

### 1.4 高级 PDU

| 特性 | ARXML 来源 | 模型落点 | 深度 | 说明 |
| --- | --- | --- | --- | --- |
| Multiplexed PDU | `MULTIPLEXED-I-PDU`、`DYNAMIC-PART` | `Pdu::Multiplexed` | `R`（第 6 阶段） | 选择器字段、静态部分、动态部分与 selector code |
| Container PDU | `CONTAINER-I-PDU`、`CONTAINED-PDU-TRIGGERING-REFS` | `Pdu::Container` | `R`（第 6 阶段） | short/long/no header；header id / fixed offset |
| Secured PDU | `SECURED-I-PDU`、`SECURE-COMMUNICATION-*` | `Pdu::Secured` | `D`（第 6 阶段） | 结构解码；没有密钥不做认证校验 |
| Transformation | `TRANSFORMATION-PROPS`、`TRANSFORMATION-TECHNOLOGY` | `Transformation` | `R` identity/linear，其余 `D` | identity / linear 可编码；未知类别只显示 raw |
| E2E | `END-TO-END-PROTECTION`、profile 引用 | `E2eProtection` | `D`（第 6 阶段） | 只读校验 + 统计，不阻断解码 |
| 加密 SecOC 发送 | 需要密钥与 CSM | — | `X` | 没有密钥材料，不做加密与 MAC 生成 |

### 1.5 明确不做

| 项 | 处理 |
| --- | --- |
| LIN / J1939 | 导入时 `X`；出现 `LIN-CLUSTER`、`J1939-*` 只记录诊断，不建 cluster |
| SOME/IP、SD、DoIP、TSN、SoAd/socket | `X`；即使 ARXML 里出现也不进模型，Buses 不显示 |
| 交换机拓扑、VLAN untagged membership 细节 | `X`；只保留能用于帧匹配的 VLAN 与 MAC |
| ARXML / DBC / FIBEX 写回、矩阵编辑 | 不做；`roxy-arxml` 与 `roxy-dbc` 只作读侧参考 |
| RXA 桌面布局、编辑器往返保真 | 不移植 |
| ISO-TP / 分段传输 | 不在本次通信模型内；PDU 只按单帧解码 |

### 1.6 缺失与不支持时的行为

| 情况 | 行为 |
| --- | --- |
| 引用目标不存在 | 实体保留名称与引用文本，`Issue::DanglingRef`，运行时不参与解码；不中断整份导入 |
| 同一路径重复 SHORT-NAME | 先出现者生效，后者报 `Issue::DuplicateShortName`；不静默覆盖 |
| 同一名字在不同包 | 路径不同即不同实体；key 只用 ID，不用名字 |
| PDU kind 不支持 | 帧和 PDU 仍出现在 Messages；子信号标记 `unsupported`，不冒充空信号 |
| Multiplexed 选择器无法解析 | 只解静态部分；动态部分计入 `unsupported_parts`，逐帧只报一次 |
| Container 成员引用缺失 | 已解析成员照常解码，缺失成员计入诊断 |
| SecOC 没有密钥 | 载荷 PDU 正常解码；认证状态为 `Unverified`，不显示“通过” |
| Transformation 类别未知 | 显示 raw 值，物理值列留空并标注 `unsupported` |
| Ethernet 帧匹配歧义 | 不解码，显示候选数；等待项目绑定 |
| FRAME-LENGTH 约定不确定 | 按项目开关解释；匹配失败时在诊断里写明当前约定 |
| 多 cluster 引用 | 按源文件/cluster 名分别建 cluster；不再“取第一个” |

---

## 2. 实体定义

### 2.1 ID

全部是持久化 newtype，JSON 里写整数：

```rust
pub struct ClusterId(pub u32);
pub struct NetworkId(pub u32);
pub struct EcuId(pub u32);
pub struct ConnectorId(pub u32);
pub struct FrameId(pub u64);
pub struct PduId(pub u64);
pub struct SignalId(pub u64);
pub struct MappingId(pub u64);
pub struct SystemSignalId(pub u32);
pub struct SignalGroupId(pub u32);
```

- `ClusterId` 全工程唯一、单调分配、永不复用（见 §3.1）。
- `FrameId` / `PduId` / `SignalId` 由导入时按源路径确定性分配（见 §3.3）。
- 所有 ID 的文本形式是十进制；工程文件写数字，Web API 同时给数字与显示名。

### 2.2 Cluster / Network

```rust
pub struct Cluster {
    pub id: ClusterId,
    pub kind: ClusterKind,          // Can | FlexRay | Ethernet
    pub name: String,               // 用户可见名，工程内唯一
    pub aliases: Vec<String>,       // 迁移/重命名前的名字
    pub description: String,
    pub sources: Vec<ClusterSource>,// 描述来源，按顺序
    pub networks: Vec<NetworkId>,
}

pub enum ClusterSource {
    Dbc { path: String },
    Arxml { path: String, cluster: String },
    Fibex { path: String },
    Manual,                          // 无描述文件，仅手工建的总线
}

pub struct Network {
    pub id: NetworkId,
    pub cluster: ClusterId,
    pub name: String,
    pub protocol: Protocol,          // Can | CanFd | FlexRay | Ethernet
    pub params: NetworkParams,
}

pub enum NetworkParams {
    Can(CanParams),
    FlexRay(FlexRayParams),
    Ethernet(EthernetParams),
}

pub struct CanParams {
    pub bitrate_kbps: u32,
    pub fd_data_kbps: u32,
    pub sample_point_permille: Option<u16>, // 模型保留，运行时不用
}

pub struct FlexRayParams {
    pub speed_kbps: u32,
    pub cycle_time_ms: f64,
    pub macrotick_us: f64,
    pub coldstart_attempts: u32,
    pub action_point_offset: u32,
    pub minislot_action_point_offset: u32,
    pub dynamic_slot_idle_phase: u32,
    pub minor_version: u32,
    pub extra: BTreeMap<String, String>, // 驱动需要的原始键值
}

pub struct EthernetParams {
    pub physical_layer: Option<EthernetPhy>,
    pub vlans: Vec<u16>,             // physical channel 上声明的 VLAN
    pub frame_length_includes_header: bool,
}
```

对照：roxy-arxml 的 `Network` 把 cluster 名、介质、CAN FD、VLAN、PHY 塞在一个
结构里。我们拆成 `Cluster` + `Network`，因为 roxy-can 的总线列表、名称、
工程绑定都以 cluster 为单位；FR 的调度参数在 roxy-arxml 中根本没有，
从现有 `FrClusterParams` 平移。

### 2.3 ECU / Connector

```rust
pub struct Ecu {
    pub id: EcuId,
    pub name: String,
    pub description: String,
    pub mac: Option<[u8; 6]>,
}

pub struct Connector {
    pub id: ConnectorId,
    pub ecu: EcuId,
    pub network: NetworkId,
    pub kind: ConnectorKind,          // Can | FlexRay | Ethernet | Lin
    pub default_vlan: Option<u16>,
    pub vlan_memberships: Vec<u16>,
}
```

对照：`Ecu`、`EcuNetworkRelation`、`EcuVlanMembership`、`normalize_mac`。
`ConnectorKind::Lin` 可以读进模型用于报“不支持”，但不生成 cluster。

### 2.4 Frame

```rust
pub struct Frame {
    pub id: FrameId,
    pub cluster: ClusterId,
    pub network: NetworkId,
    pub name: String,
    pub length: u32,                  // 字节
    pub address: FrameAddress,
    pub timing: FrameTiming,
    pub ethernet: Option<EthernetFrameMeta>,
    pub pdu_mappings: Vec<MappingId>,
}

pub enum FrameAddress {
    Can { id: u32, extended: bool, fd: bool },
    FlexRay { slot: u16, base_cycle: u8, repetition: u8, payload_preamble: bool },
    Ethernet { vlan: Option<u16> },
}

pub struct FrameTiming {
    pub cycle_us: Option<u64>,        // PDU/Frame triggering 周期
    pub startup: bool,                // FlexRay startup frame
    pub event: bool,                  // 事件触发（CAN/ETH）
}

pub enum EthernetFrameMeta {
    Base,
    Generic,
    UserDefined,
    Ieee1722 {
        stream_identifier: u64,
        sub_type: u64,
        version: u64,
        presentation_time_s: Option<f64>,
    },
}
```

对照与扩展：

| roxy-arxml `Frame` | 本模型 | 差异 |
| --- | --- | --- |
| `can_id`、`extended`、`can_fd` | `FrameAddress::Can` | 相同 |
| `network`、`length`、`cycle_ms` | `cluster/network/length/timing` | 周期改微秒，CN 仍可读毫秒 |
| `ethernet: Option<EthernetFrame>` | `EthernetFrameMeta` | 只留 variant 与 1722 元数据 |
| `vlan_id` | `FrameAddress::Ethernet::vlan` | 相同 |
| — | `slot/base_cycle/repetition/payload_preamble` | roxy-arxml 没有 FR 地址，新增 |
| — | `ethertype_hint` | ARXML 不携带；由项目覆盖/捕获头提供，放 `Frame` 的可选覆盖表 |

EtherType 不进入 `EthernetFrameMeta` 的规范字段，因为 AUTOSAR 的
`ETHERNET-FRAME` 不声明它。项目级覆盖放在 `FrameOverrides`（见 §4.2），
匹配时优先级：项目覆盖 > 捕获头 > 无。

### 2.5 PDU

```rust
pub struct Pdu {
    pub id: PduId,
    pub name: String,
    pub length: u32,
    pub kind: PduKind,
    pub header_type: ContainerHeaderType,
    pub selector: Option<SelectorField>,
    pub secured: Option<SecuredPduMeta>,
    pub transformation: Option<TransformationId>,
    pub e2e: Option<E2eProtectionId>,
}

pub enum PduKind { Signal, Container, Multiplexed, Secured }

pub struct SelectorField {
    pub start_bit: u32,
    pub length: u32,
    pub byte_order: ByteOrder,
    pub signal: Option<SignalId>,
}

pub struct SecuredPduMeta {
    pub header: SecuredPduHeader,     // NoHeader/Header08/16/32
    pub payload: Option<PduId>,
    pub data_id: Option<u64>,
    pub freshness_value_id: Option<u64>,
    pub secured_area: Option<(u32, u32)>, // offset, length (bits)
    pub auth_info_length: Option<u32>,
    pub verification: SecVerification,    // Structural | Unverified
}
```

对照：roxy-arxml 的 `Pdu`/`PduKind`/`SecuredPdu`/`ContainerHeaderType`/
`SelectorField` 基本原样。差异：

- `SecuredPduMeta` 把认证参数收成一个结构，不复制 roxy-arxml 的多张
  props 表；分析器不需要编辑复用关系。
- `verification` 明确写“结构解码/未验证”，避免 UI 把“解出来”说成“验过”。
- `transformation` 与 `e2e` 是新增引用，roxy-arxml 没有读侧实现。

### 2.6 映射

```rust
pub struct FramePduMapping {
    pub id: MappingId,
    pub frame: FrameId,
    pub pdu: PduId,
    pub start_bit: u32,
    pub byte_order: ByteOrder,
}

pub struct SignalPduMapping {
    pub id: MappingId,
    pub signal: SignalId,
    pub pdu: PduId,
    pub start_bit: u32,
    pub byte_order: ByteOrder,
    pub update_bit: Option<u32>,
}

pub struct ContainedPduMapping {
    pub id: MappingId,
    pub container: PduId,
    pub pdu: PduId,
    pub header_id: u32,
    pub offset: u32,                  // no-header container 的字节偏移
}

pub struct MultiplexedPduPart {
    pub id: MappingId,
    pub mux: PduId,
    pub part: PduId,
    pub selector_code: Option<u32>,   // None = 静态部分
    pub initial: bool,
}
```

### 2.7 Signal

```rust
pub struct Signal {
    pub id: SignalId,
    pub pdu: PduId,
    pub name: String,
    pub system_signal: Option<SystemSignalId>,
    pub length: u32,
    pub start_bit: u32,
    pub byte_order: ByteOrder,
    pub update_bit: Option<u32>,
    pub signed: bool,
    pub float: bool,
    pub factor: f64,
    pub offset: f64,
    pub unit: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_open: bool,
    pub max_open: bool,
    pub initial_raw: Option<f64>,
    pub values: Vec<(i64, String)>,
    pub base_type: Option<BaseTypeId>,
    pub compu: Option<CompuMethodId>,
    pub comment: String,
    pub transformation: Option<TransformationId>,
    pub group: Option<SignalGroupId>,
}
```

`start_bit` / `byte_order` 是映射的冗余投影：运行时展开计划只读 `Signal`，
不再回查 `SignalPduMapping`。导入时两者必须一致；不一致以映射为准并报
`Issue::SignalLayoutConflict`。

对照：字段来自 roxy-arxml `Signal` + `IsignalPduMapping` + `BaseType`。
新增 `float`（由 base type 的 `IEEE754` 得出，现有 CAN 解析已经这样做）、
`transformation`、`group`、`initial_raw`（导出为物理值时再乘 factor/offset）。

### 2.8 关系与高级元数据

```rust
pub enum RelationRole { Sender, Receiver }

pub struct FrameEcuRelation { pub frame: FrameId, pub ecu: EcuId, pub role: RelationRole }
pub struct PduEcuRelation   { pub pdu: PduId,   pub ecu: EcuId, pub role: RelationRole }
pub struct SignalEcuRelation{ pub signal: SignalId, pub ecu: EcuId }

pub struct SignalGroup { pub id: SignalGroupId, pub name: String, pub sender: Option<EcuId> }

pub struct Transformation {
    pub id: TransformationId,
    pub name: String,
    pub kind: TransformationKind,
}

pub enum TransformationKind {
    Identity,
    Linear { scale: f64, offset: f64 },
    TextTable(Vec<(i64, String)>),
    Unknown { category: String },
}

pub struct E2eProtection {
    pub id: E2eProtectionId,
    pub profile: Option<u32>,
    pub data_id: Option<u32>,
    pub payload: Option<PduId>,
}
```

### 2.9 运行时视图（不持久化）

```rust
pub struct DecodePlan {
    pub frame: FrameId,
    pub version: u32,
    pub reads: Vec<SignalRead>,       // 位读取，按 payload 偏移
    pub writes: Vec<SignalWrite>,     // 编码用
}

pub struct SignalRead {
    pub signal: SignalId,
    pub bit_offset: u32,
    pub bit_len: u32,
    pub byte_order: ByteOrder,
    pub signed: bool,
    pub float: bool,
    pub factor: f64,
    pub offset: f64,
    pub mux: Option<MuxCondition>,    // selector == code 才解
}

pub struct FrameRoute {
    pub cluster: ClusterId,
    pub match_key: FrameMatchKey,
}

pub enum FrameMatchKey {
    Can { id: u32, extended: bool },
    FlexRay { slot: u16, cycle: u8 },
    Ethernet { vlan: Option<u16>, flow: EthFlowKey },
}
```

展开计划是第 4 阶段的性能门槛所在：每帧解码只走 `reads`，不遍历
`Frame → mapping → Pdu → mapping → Signal`。Container/Mux 的展开结果带
`version`，模型重载时整体替换。

---

## 3. 标识与 key（冻结）

### 3.1 决策 1：ClusterId 全局持久化

- `ClusterId(u32)` 在工程内全局唯一，单调分配，删除后不复用。
- kind、名称、序号都不是身份。`CAN1` 改名为 `Powertrain`、删除后再加一条
  CAN，都不会让旧 key 指到新总线。
- 工程文件保存 `nextClusterId`；开发期没有对外工程，因此不读 v1、不做迁移。
- 硬件通道、回放文件通道、录制通道都通过**独立映射表**指向 `ClusterId`，
  不再把“第几路”当身份。

理由：三套 `BTreeMap<u8, ...>` 的空洞、复用和跨协议撞号，正是当前
`SigKey` 需要 `Can/Fr/Eth` 三个变体的根源。全局 id 让订阅、触发器、
脚本、工程引用都指向同一实体。

### 3.2 决策 2：SigKey 文本形态

运行时：

```rust
pub struct SignalKey {
    pub cluster: ClusterId,
    pub frame: FrameId,
    pub signal: SignalId,
}
```

`pdu` 不进入身份：一个 `I-SIGNAL` 在 AUTOSAR 中只属于一个 PDU
（`constr_3514`），由 `SignalId` 可反查。运行时展开计划里带 `PduId` 只为
Container/Mux 路由。

持久化/API 文本：

```
sig:{cluster_id}:{frame_id}:{signal_id}     例：sig:1:17:42
derived:{definition_id}                     例：derived:brk_pressure
```

用户输入（脚本、触发器、派生表达式、搜索框）接受显示别名：

```
[Cluster:Frame:Signal]                      例：[CAN1:EngineData:Speed]
```

解析规则：

1. 先按当前模型的名字找 cluster → frame → signal；大小写不敏感，歧义时报
   候选数，不猜。
2. `[CAN1:0x100:Speed]` 这类带地址的写法仍接受：frame 段先按名字，再按
   CAN ID / FR slot / ETH 帧名。
3. 解析成功后立即规范化为 `sig:...` 写回工程；显示时再还原别名。

旧 v1 key 文本已移除：`0:256:Name`、`fr:0:25:X`、`eth:0:Frame:X` 不再解析，
`derived:id` 仍保留。只接受 `sig:` 文本与 `[Cluster:Frame:Signal]` 别名。

脚本源码只接受 cluster 文本与别名；解析失败给编译诊断，不静默指向同名的
另一条总线。工程文件只写 v2。

### 3.3 决策 1 的配套：实体 ID 分配

- 导入时对每个元素算 `canonical_source_key`：
  - ARXML：`{source_id}:{element_path}`，例如 `arxml:2:/Pkg/Frames/EngineData`；
  - DBC：`dbc:{source_id}:message:{name}` / `:signal:{name}`；
  - FIBEX：`fibex:{source_id}:{element_path}`；
  - 手工：`manual:{cluster_id}:{uuid}`。
- `id = fnv1a64(canonical_source_key) & 0x7FFF_FFFF_FFFF_FFFF`，为 0 时取 1。
- 同一工程内碰撞时线性探测，并把 `source_key → id` 写进 `idAliases`。
- 源文件里元素改名/移动后，旧 key 找不到哈希；导入器先查 `idAliases`，
  命中则沿用旧 id 并更新 alias。这样“重命名不改 id”。
- 普通工程不带 `idAliases`；它只在碰撞或改名后出现。

---

## 4. 工程 schema v2（冻结草案）

### 4.1 顶层

工程文件只有 v2（`format` + `version` + `clusters` + typed 段），不再接受
早期的 `{ "config": ... }` 快照。

```json
{
  "format": "roxy-can-project",
  "version": 2,
  "nextClusterId": 4,
  "clusters": [
    {
      "id": 1,
      "kind": "can",
      "name": "CAN1",
      "aliases": [],
      "sources": [
        { "kind": "arxml", "path": "Matrix.arxml", "cluster": "CanCluster" }
      ],
      "can": { "bitrateKbps": 500, "fdDataKbps": 2000 },
      "ether": null,
      "nodeRoles": { "ECU_Brake": "Simulated" }
    },
    {
      "id": 2,
      "kind": "flexray",
      "name": "FR1",
      "aliases": ["Chassis"],
      "sources": [
        { "kind": "arxml", "path": "Matrix.arxml", "cluster": "FrCluster" }
      ],
      "flexray": { "speedKbps": 10000, "cycleTimeMs": 5.0 },
      "nodeRoles": {}
    },
    {
      "id": 3,
      "kind": "ethernet",
      "name": "ETH1",
      "aliases": [],
      "sources": [
        { "kind": "arxml", "path": "Matrix.arxml", "cluster": "EthCluster" }
      ],
      "ether": {
        "physicalLayer": "100BASE-T1",
        "vlans": [100],
        "frameLengthIncludesHeader": false
      },
      "nodeRoles": {}
    }
  ],
  "idAliases": [
    { "kind": "frame", "source": "arxml:1:/Pkg/Frames/OldName", "id": 17 }
  ],
  "frameOverrides": [
    { "cluster": 3, "frame": "CamFrame", "ethertype": 8905, "dstMac": "01:00:5E:00:00:01" }
  ],
  "replay": [
    { "kind": "can", "fileChannel": 0, "cluster": 1 },
    { "kind": "flexray", "fileChannel": 1, "cluster": 2 }
  ],
  "busMap": {
    "can": [1, 2],
    "flexray": [3],
    "ethernet": [4]
  },
  "generators": [
    { "kind": "can", "cluster": 1, "signalRefs": ["sig:1:17:42"], "cycleUs": 100000 }
  ],
  "views": { "...": "窗口快照，信号引用一律写 sig:/derived: 文本" },
  "derived": [ "..." ],
  "web": { "...": "前端工作区，v2 起信号引用也写 sig:/derived:" }
}
```

字段约束：

- `clusters` 是唯一总线列表；不再出现 `channels`、`fr_buses`、`eth_buses`、
  `can_arxml`、`fr_names`、`eth_frame_length_includes_header`。
- `sources` 顺序即优先级；同一 cluster 可同时挂 DBC 与 ARXML，但同一元素
  以先出现者为准并报诊断。
- 导入的模型本身**不写进工程**：路径、cluster 名、覆盖与 id 别名足够
  重新构建。这样 ARXML 更新后工程能重新加载，而不会带着过期模型。
- 视图/触发器/派生里的信号引用只写 `sig:` / `derived:`；显示名从模型查。
- `web` 仍由前端定义，但 core 在迁移时会把其中的 `ch/bus/ext/frSlot/ethFrame`
  重写为 cluster id + key（见 §4.4）。

### 4.2 帧覆盖

`frameOverrides` 是唯一允许绕过 ARXML 的运行时事实：

| 字段 | 用途 |
| --- | --- |
| `ethertype` | ARXML 不携带；匹配与发送使用 |
| `dstMac` / `srcMac` | 发送帧头；接收匹配打分 |
| `lengthIncludesHeader` | 覆盖 cluster 级帧长约定 |
| `vlan` | 覆盖帧触发推导出的 VLAN |

---

## 4.3 工程只读写 v2

软件开发期没有对外发布的工程文件，因此**没有 v1 读取或迁移**：版本不是
`2` 的文件直接报 unsupported。`clusters` 是唯一的总线列表，CAN/FR/ETH 的
名称、sources、位率和节点角色都由它恢复；gen/trigger 行用 cluster id 记录，
`busMap` 只负责在打开时换回路号。
---

## 5. 现有代码引用图（第 4 阶段迁移面）

### 5.1 后端

| 位置 | 现状 | v2 替换 | 阶段 |
| --- | --- | --- | --- |
| [`src/observe.rs:391`](../src/observe.rs#L391) | `SigKey::{Can,Fr,Eth,Derived}` | `SignalKey` + `derived:` 分支 | 4 |
| `src/observe.rs:460-490` | `text()` / `from_text()` 四种文本 | `sig:` 规范文本 + 旧解析器 | 4 |
| [`src/channel.rs:51`](../src/channel.rs#L51) | `Channel`（CAN 专用 + DBC） | `Cluster` + `Network` + `CanParams` | 1-3 |
| [`src/app.rs:46`](../src/app.rs#L46) | `FrBusCfg { path, db }` | `Cluster` + `sources` | 1-3 |
| [`src/app.rs:53`](../src/app.rs#L53) | `EthBusCfg { path, db }` | `Cluster` + `sources` | 1-3 |
| `src/app.rs:266/274/279/284` | `fr_buses`、`eth_buses`、`can_arxml`、`eth_clusters` 四张表 | `ClusterRegistry` / `Comm` | 1-4 |
| `src/channel.rs:371-722` | `import_arxml_path`、`next_eth_bus`、`load_*_description_cluster` | 统一 importer + cluster registry | 2-4 |
| `src/can_arxml.rs` | ARXML → DBC 形状 `SymbolTable` | 直接建统一模型；DBC 形状只留给 DBC importer | 2 |
| `src/fr_db.rs` / `src/eth/db.rs` | 各自 `FrDb` / `EthDb` + 解码索引 | 统一模型 + `DecodePlan` | 2-4 |
| `src/config.rs:614` | `Config` 顶层 | v2 顶层（§4.1） | 7 |
| `src/config.rs:80/237/290/307/317/388/429/455/501/548/560/571` | `ChannelCfg`、`SignalCfg`、`GfxCfg`、`DataCfg`、`MonitorRowCfg`、`StateCfg`、`TxCfg`、`FrTxCfg`、`TriggerCfg`、`FrBusFile`、`EthBusFile`、`CanArxmlFile` | 全部改为 cluster + `signalRef` | 7 |
| `src/bus.rs` | `Vec<Channel>` 解码、`SigKey::can/fr/eth` 路由、Panel 发送 | cluster 路由 + `DecodePlan` + 统一编码器 | 4-5 |
| `src/replay_map.rs:42` | `can/fr/eth` 三张 `BTreeMap<u8, Option<u8>>` | `BTreeMap<(LogBusKind,u8), ClusterId>` | 5 |
| `src/generator.rs` | CAN/FR 生成器按 bus 索引 | cluster + frame id | 5 |
| `src/trigger.rs` | `TriggerCfg {ch,id,ext}` | cluster + `SignalKey` | 4-5 |
| `src/script/compiler.rs:286,841` | `send_frame` 的 frame 路径与 bus 解析 | cluster + frame id；旧路径经 resolver | 5 |
| `src/script/mod.rs` | `Script.frame_consts`、`send_refs` | 同上 | 5 |
| `src/reference.rs` | 参考回放的 `SigKey` / ETH 帧匹配 | `SignalKey` + `FrameRoute` | 5 |
| `src/export.rs` | CSV/ASC/BLF 导出按 `SigKey.text()` | 写 `sig:` + 显示列 | 5 |
| `src/state_seg.rs` | 状态分段按 `SigKey` 键 | `SignalKey` | 4 |
| `src/workspace.rs` | `Pick::{Can,Fr,Eth}` | `Pick::{Frame,Flow}` + cluster id | 4-7 |
| `src/hw/vector.rs` / `src/hw/kvaser.rs` | CAN 通道序号 | 会话级 cluster→硬件通道绑定 | 5 |
| `src/web.rs:4420-4457` | `WebState.buses/fr_buses/eth_buses` 三份 | `WebState.clusters` + `busMap` | 4-7 |
| `src/web.rs:5359-5376` | `WebFrBus` / `WebEthBus` 序列化 | 统一 `WebCluster` | 7 |
| `src/web.rs:4166` | `WebCatalogChannel` 三类拼接 | 统一 catalog（cluster+frame+signal） | 4-7 |

### 5.2 前端

| 位置 | 现状 | v2 替换 | 阶段 |
| --- | --- | --- | --- |
| [`web/src/types.ts:41`](../web/src/types.ts#L41) | `frBuses`、`ethBuses` 与 `channels` 并列 | `clusters` + `busMap` | 7 |
| `web/src/types.ts:131-147` | `FrBus`、`EthBus` | `Cluster`（kind 字段区分） | 7 |
| `web/src/types.ts:477-482` | `Channel` 只有 CAN | `Cluster` + `Network` | 7 |
| `web/src/components/BusesModal.tsx:14-15,378-612` | 三套加载入口 | **Add cluster** + 按扩展名/importer 选择；详情按 kind | 7 |
| `web/src/derived.ts` | `DerivedReference {key,bus,frame,frameId}` | `signalRef` 字符串 + 解析结果 | 4-7 |
| `web/src/components/DerivedPanel.tsx` | 三栏 bus/frame/signal | cluster/frame/signal，引用写 `sig:` | 4-7 |
| `web/src/components/SignalPicker.tsx` | `ch/id/ext/fr_slot/eth_frame` | `signalRef` | 7 |
| `web/src/components/PagePanel.tsx` / `panelPage.ts` | frame action 的 `bus/source/frameId/ext` | `cluster/source/key` | 7 |
| `web/src/components/TraceTable.tsx` / `SignalChart.tsx` | 行/曲线按 bus 编号 | cluster id + key | 4-7 |
| `web/src/app.tsx` | workspace、选择、命令拼接 bus 字段 | `busMap` 迁移后全部用 cluster | 7 |

### 5.3 测试与夹具

| 位置 | 用途 | v2 处理 |
| --- | --- | --- |
| `src/app_tests.rs`（55 处 `SigKey`） | 运行时行为 | 第 4 阶段按新 key 迁移；断言保留 |
| `src/headless_tests.rs`（12 处） | 端到端命令 | 第 4-5 阶段迁移 |
| `src/can_arxml_tests.rs` | CAN ARXML 解析 | 第 2-3 阶段换成统一模型断言 |
| `src/test_assets.rs` / `test_assets_data.rs` | FR/ETH 夹具 | 第 2 阶段补多协议样本 |
| `web/src/*.test.ts` | 前端纯函数 | 第 7 阶段随 `signalRef` 迁移 |

---

## 6. 语料

### 6.1 仓库内已有

| 样本 | 来源 | 覆盖 | 用途 |
| --- | --- | --- | --- |
| `PowerTrain.arxml` | `src/test_assets_data.rs::POWERTRAIN_ARXML_GZ`（gzip 内嵌，运行时写到 `target/test-assets`） | FlexRay cluster、48 帧、24 PDU、compu | FR 基线 |
| `PowerTrain_v2.xml` / `PowerTrain2_v2.xml` | 同上 `POWERTRAIN_FIBEX_GZ` / `POWERTRAIN_FIBEX2_GZ` | FIBEX FR | FIBEX 适配器 |
| `EthSample.arxml` | `src/test_assets.rs::ETH_ARXML`（运行时生成） | 单 ETH cluster、VLAN 100、帧/PDU/16 位信号 | ETH 基线 |
| CAN ARXML 内联夹具 | `src/can_arxml_tests.rs` | CAN cluster、地址、帧、PDU、信号、compu | CAN 基线 |
| Container / Mux / Secured 夹具 | `roxy-arxml/src/import_tests/{container,multiplexed,secured}.rs`（只读参考） | 元素形状与引用结构 | 第 2/6 阶段改写为 roxy-can 夹具 |

### 6.2 必须补的三类样本

| 样本 | 现状 | 来源/期望 |
| --- | --- | --- |
| 多协议单文件 | 缺 | 需要一份同时含 CAN + FR + ETH cluster 的 ARXML；可用现有三份夹具合并生成，验证 cluster 不再“取第一个” |
| Container / Multiplexed | 有参考夹具，无 roxy-can 夹具 | 以 roxy-arxml 的 `container.rs` / `multiplexed.rs` 为语义参考，生成带短/长/无 header 容器与动态部分的样本；期望逐位解码 golden |
| SecOC / E2E / Transformation | **缺真实样本** | roxy-arxml 只有 SecOC 写回夹具，没有 E2E/Transformation 读侧；必须取得 OEM ARXML 或按 AUTOSAR 规范自制样本并标注“规范推导”。在样本到位前，这三项保持 `D`/`M`，不承诺位级兼容 |

样本统一放在 `target/test-assets`（运行时生成），仓库不保存二进制/大文件；
真实 OEM 样本只在本地验证，不提交。

---

## 7. 决策记录

### 7.1 决策 1：ClusterId（冻结）

采用全局持久化 `ClusterId(u32)`。理由与规则见 §3.1。被否决的方案：

- **kind + 序号**：删除/重排会复用身份，旧 key 会指错总线；三套编号
  空间正是当前复杂度的来源。
- **名称作身份**：重命名会断引用，且不同来源可能同名。

### 7.2 决策 2：SigKey 文本（冻结）

运行时 `SignalKey{cluster,frame,signal}`；持久化 `sig:{cluster}:{frame}:{signal}`；
显示别名 `[Cluster:Frame:Signal]`；旧 v1 文本已移除。理由见 §3.2。

保留旧解析器的成本很低（一个 resolver），但不再写出；这样用户脚本不会
在升级当天全部失效，而工程文件只有一个规范形态。

### 7.3 决策 3：高级特性深度（冻结）

| 特性 | 深度 | 理由 |
| --- | --- | --- |
| Multiplexed PDU | 完整解码/编码 | 选择器是普通信号，动态部分是互斥布局；不做的代价是大量商用车矩阵不可读 |
| Signal Group | 模型 + 组级发送方 | 解码沿用成员信号；组只影响方向判定与展示 |
| Container PDU | 完整解码/编码 | short/long/no header 的布局可由 header id/偏移确定；SOME/IP 仍在范围外 |
| Transformation | identity / linear 编解码；其余只读 raw | 这两类覆盖绝大多数矩阵；未知类别不能假装换算 |
| E2E | 只读校验 + 统计 | 校验不改变信号值；不阻断解码 |
| SecOC | 结构解码，认证 `Unverified` | 没有密钥材料；发送后置，且需要密钥管理，超出分析器范围 |

“只读校验”指：能读出 Data ID、freshness、AuthData 位置和长度，能报告
“未验证/结构不符”；不实现 CRC、CMAC、AES 或密钥导入。

### 7.4 决策 4：旧工程（冻结）

不兼容也不迁移：工程只读写 v2。开发期没有对外工程，保留 v1 读取只会把
旧字段带进每条新路径。

### 7.5 决策 5：性能（冻结）

- 第 4 阶段起以 `DecodePlan` 为唯一解码路径；任何回退到模型树遍历的实现
  不允许合并。
- 基准固定为：50k Trace 重建、8 路 × 12k 曲线、10Hz 发布；记录 p50/p99
  与发布 JSON 大小。
- Container/Mux 展开缓存的 `version` 在模型重载时递增；旧计划由
  `Arc` 持有到当前帧处理完，不在热路径加锁。

高级结构的实测（2026-10-30，release 构建，Ryzen AI 9 H 365；
`cargo test --release perf_advanced_feature_paths -- --ignored --nocapture`）：

| 路径 | p50 | p99 |
| --- | --- | --- |
| 普通信号解码 | 67.3 ns | 69.8 ns |
| Mux 动态部分解码 | 73.9 ns | 82.0 ns |
| Container 子 PDU 解码 | 71.9 ns | 76.4 ns |
| Linear Transformation 解码 | 69.0 ns | 70.8 ns |
| E2E 计数器读取 | 36.8 ns | 38.3 ns |
| SecOC 计划构建 | 263.0 ns | 367.9 ns |

SecOC 没有逐帧的额外校验：头、secured area 与认证长度在建计划时检查一次，
之后仍走内层 PDU 的同一计划。

发布基准（同机 release，`cargo test --release perf_snapshot_publish_under_load -- --ignored --nocapture`）：
探针是 6 条曲线 ×72 000 点（曲线总点数高于冻结基线的 8×12k）、50k Trace、
20 ms 一帧的发布负载。

| 路径 | mean | p50 | p99 |
| --- | --- | --- | --- |
| 全缓存重采样的一帧 | 16.2 µs | 15.4 µs | 24.6 µs |
| 空闲帧（仅共享） | 7.2 µs | 7.2 µs | 8.0 µs |
| 旧的深拷贝实现 | 4446.6 µs | — | — |

发布 state JSON 为 6 529 字节（空实时 series 队列）；最坏情况比旧深拷贝便宜约
275 倍。

---

## 8. 第 0 阶段退出检查

| 检查项 | 状态 |
| --- | --- |
| 特性表覆盖必须 / 高级 / 不做，且每项有落点与报错行为 | 完成（§1） |
| 实体字段与 roxy-arxml `AUTOSAR.md` / `model/mod.rs` 对照 | 完成（§2） |
| 决策 1–3 冻结 | 完成（§7.1–7.3） |
| 决策 4–5 有明确规则 | 完成（§7.4–7.5） |
| 现有 `SigKey` / `Channel` / `FrBusCfg` / `EthBusCfg` 引用图 | 完成（§5） |
| schema v2 JSON 草案 | 完成（§4.1–4.2） |
| 工程只读写 v2 | 完成（§4.3，无迁移层） |
| 多协议 / Container-Mux / SecOC-E2E 语料 | 部分：多协议待生成，Container/Mux 有参考夹具，SecOC/E2E 缺真实样本（§6.2） |
| 能逐项回答“该元素在 roxy-can 中对应什么/不支持时如何报错” | 完成（§1） |

未闭环项只有语料：它不阻塞第 1 阶段的模型与 schema 编码，但阻塞第 2 阶段
的“真实多协议 ARXML 生成完整模型”退出条件，以及第 6 阶段的 SecOC/E2E
位级验收。

---

## 9. 代码落点（第 1–4 阶段首个切片）

| 文件 | 内容 |
| --- | --- |
| `src/model/mod.rs` | 实体、ID newtype、Comm 查询/完整性检查、导入器 trait |
| `src/model/id.rs` | 确定性元素 ID、改名绑定、集群 ID 分配 |
| `src/model/key.rs` | `SignalKey` / `SignalRef`、`sig:` 文本、`[Cluster:Frame:Signal]` 别名（无旧 key 解析） |
| `src/model/schema.rs` | schema v2 DTO、字段校验、`idAliases`、帧覆盖、回放映射 |
| `src/model/arxml.rs` | 统一 ARXML 读取器（三协议 cluster/帧触发/PDU/信号/compu/ECU/端口/高级 PDU 模型） |
| `src/model/dbc_import.rs` | DBC → CAN cluster 适配器 |
| `src/model/legacy_import.rs` | FIBEX → FlexRay cluster、Ethernet DB 适配器 |
| `src/model/plan.rs` | `DecodePlan` / `PlanCache`：Frame→PDU→Signal 展开，含 Container/Mux/Secured 偏移 |
| `src/model/runtime.rs` | 按计划解码/编码：物理值、符号、float、mux 选择器门控 |
| `src/model/registry.rs` | 工程级 cluster 注册表：全局 id、按源去重、三协议共存、导出 schema v2、从 schema v2 重新导入 |
| `src/model/project_cfg.rs` | `Config` 总线列表 → v2 `clusters`，供保存写入 |
| `src/bus.rs` | CAN/FR/ETH 分配全局 `ClusterId`；订阅、三协议 Messages 展开、CAN/FR 触发器、脚本节点输入、三协议 Panel 物理编码、CAN/FR Generator 与三协议脚本发送走统一模型，旧实现仅作无模型时回退 |
| `src/project.rs` | `.rxproj` 保存 schema v2（`clusters`/`busMap`/`config`），打开时恢复 ClusterId；只读 v2，v1 文件直接拒绝 |
| `src/web.rs` + `web/src/components/BusesModal.tsx` | Web 状态输出统一 `clusters`；Buses 顶部渲染 Cluster 表，单一 **Add cluster** 菜单按来源分流 CAN/ARXML/FIBEX；编辑仍按 kind 分卡 |

单测：

- `src/model/tests.rs`：ID/key/schema/查询 20 条。
- `src/model/arxml_tests.rs`：三协议导入、真实 FlexRay 矩阵、高级 PDU、ECU 端口/信号组。
- `src/model/dbc_tests.rs`：DBC 帧/信号/节点、初始值、导入器边界。
- `src/model/legacy_tests.rs`：FIBEX、Ethernet DB、DBC↔ARXML 同消息等价性。
- `src/model/plan_tests.rs`：普通 PDU、Container、Multiplexed、Secured 的展开偏移与 mux 条件。
- `src/model/runtime_tests.rs`：物理值/符号/float 解码，编码回读，mux 选择器门控。
- `src/model/registry_tests.rs`：多 cluster ARXML、三协议共存、源去重、schema v2 导出。

## 10. 当前进度

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| 0 需求与语料 | 完成 | 特性表、字段映射、引用图、schema v2、决策 1–5；cantools（MIT）`system-4.2.arxml` 已作为真实语料嵌入测试夹具，覆盖 SecOC 与 4.2 组级 E2E；带 counter/CRC 位置的真实 E2E 语料仍缺 |
| 1 冻结模型与 schema | 完成 | 模型、ID、key、schema v2 编译通过；新模块不依赖旧三套总线类型 |
| 2 统一 ARXML 导入 | 已接入实际导入路径 | CAN/FR/ETH、compu/初始值/量程（含 `DATA-CONSTR`）、PDU 周期、ECU 端口/信号组、Container/Mux/Secured 模型；真实 FR 矩阵、ETH 夹具、多协议单文件通过；Buses 的 ARXML 导入与工程重开都把文档模型直接送进核心（`SetChannelModel` / `SetFrModel` / `SetEthModel`，核心按全局 cluster id 重绑），FlexRay 计划按日志口径把 PDU 起点重定位到帧首 PDU；cantools `system-4.2.arxml` 验证了 SecOC 的 `PAYLOAD-REF`→`PDU-TRIGGERING`→`I-PDU-REF` 间接引用，以及 `DATA-TRANSFORMATION-SET` 里按组声明的 E2E profile/Data ID；组级声明不含 counter/CRC 位置，计划明确报 unsupported 而不是静默丢弃。另在本地只读验证了一套厂商 CAN 矩阵（24 帧 / 37 PDU / 100 信号、2 个真实 Transformation，含 `SOMEIP_E2E_Transformer_PROFILE_04_B` 与 SOME/IP 默认链）：导入无错误，信号到变换对象的链接正确，未知类别保持显式 unsupported |
| 3 DBC/FIBEX 适配器 | 首版 | DBC、FIBEX、Ethernet DB → Cluster；一条 DBC↔ARXML 等价性测试；大规模等价语料待补 |
| 4 运行时切换 | 订阅已切 | BusCore 为 CAN/FR/ETH 分配全局 `ClusterId` 并各自构建 `Comm`/`PlanCache`；三类订阅值都走统一展开计划，CAN 复用报文也已按模型 mux 条件解码，三类 Messages 展开行由核心用模型解码后随快照下发；CAN/FR 信号触发器、脚本节点输入与 `set_sig/get_sig` buffer 内建、订阅折叠、Messages/Trace 展开、Reference 回放与 CLI CSV 导出都只走模型计划，生产代码已没有旧解码调用；Messages 与 Statistics 的 CAN 帧名与信号数、以及 FR/ETH Messages 的信号数都从模型快照读取（标准/扩展同号可分，容器子 PDU 也计数），生产路径不再查描述表；SignalPicker 的 catalog 三协议都从统一模型生成（含初始物理值与容器/mux 成员），键一律写 `sig:can|fr|eth:<cluster>:…`。观测键只带 `ClusterId`：生成键、相等/排序、工程文本都以 cluster 为准，bus 索引由核心的 cluster→bus 映射在需要路由时解析；旧 `ch:id:name` / `fr:` / `eth:` 文本仍由 `BusCore::resolve_key_text` 按旧总线表读入并换成 cluster 键。CAN 报文聚合（Messages/Statistics 的计数、周期、抖动与规格检查）也按 `ClusterId` 键控，删/加路时只在键里去掉被删 cluster，存活路不重编号。生成器的帧名、发送 ECU、长度与周期、节点发送表以及 Replay Block 的节点过滤都优先取模型的 FRAME-PORT/PDU-PORT 关系；帧注释进模型的 Frame.comment（DBC 的 CM_ 由适配器写入），catalog 不再读描述表；规格检查与 Messages/Trace/Statistics/CSV 的帧名、声明周期与 DLC 都先取模型（含 dbc_only 过滤）；描述表不再参与生产读取：帧名、规格判定、触发超时、Trace 的值过滤与 profile 的节点校验都从模型解析 |
| 5 发送/硬件/回放 | 首个切片 | Panel 三协议物理值发送、Generator 的 CAN/FR 驱动信号、脚本 `send_frame` 的三协议编码均走统一编码器；触发器的反应发送也走模型计划（镜像按模型信号 id 编码，模型不认识该帧时发条目原字节）；Generator 的 CAN/FR 载荷与脚本 `set_fr_sig` 写入也只走模型编码（模型不认识的帧发送条目原字节，不再回退旧表），Panel Setpoint 的三协议写入同样按模型编码并保留条目其余字节，模型缺失时直接拒绝编码，不再回退描述表；脚本 `send_frame` 的 FR/ETH 帧同样只按模型编码，帧不在模型里就报错（描述表的旧编码器只剩测试用途，Ethernet 的那个已删除）；回放映射运行时也以 cluster 为目标（表项是 ClusterId，三个协议各一张表），删路只把指向它的表项变成「不回放」；schema v2 的 ReplayCfg 增加显式 disabled，ReplayCfg 的 disabled 显式表达「不回放」。硬件端口按物理路号寻址，附件随通道（即簇）移动并有测试锁定；VN1610 双口 CAN/FD 双向收发已由 `vector_bench_loops_classic_and_fd_both_ways` 自动验收（经典 500 kbit/s + FD 2/5/8 Mbit/s BRS） |
| 6 高级特性执行 | 首个切片 | Multiplexed（ARXML 动态部分 + DBC `mux_when`）、Secured PDU payload 链接、E2E profile 与 SystemSignal/PDU 的结构关联已进模型；Container 展开按短/长头里的 PDU ID 与长度门控 contained PDU 的读取与写入（头未标注 ID 时保持旧行为）；`TRANSFORMATION-PROP-SET` 读取 packing byte order，identity/linear 变换已在解码/编码计划里执行（linear 按 `app = scale·net + offset` 及其逆运算），TextTable/未知类别只报 `UnsupportedTransformation` 并保留原值；导入与计划的 Warning/Error 现在写进 Write 窗口（按消息去重，重建模型不重复刷屏）；ARXML 的 I-SIGNAL/DATA-TRANSFORMATIONS 引用现已链接到模型里的变换对象；SignalPicker 在信号名旁显示 I-SIGNAL-GROUP 徽标，点击徽标选中该组全部信号；UPDATE-INDICATION-BIT-POSITION 进入 SignalRead/SignalWrite，解码只在置位时输出该信号、编码写值后置位；未知链在计划里报 UnsupportedTransformation 并保留原值（没有真实链语料，暂不解释数值）。SecOC 的 `SECURED-AREA-OFFSET/LENGTH` 与引用对象上的 `AUTH-INFO-TX-LENGTH` 已读入模型，计划会检查头+载荷、secured area、authenticator 是否超出 PDU 并报 `LengthConvention`。E2E 做结构校验（保护引用存在、声明了 profile、需要传送 Data ID 的 profile 配了值）；E2E 计数器位置进入 `E2ePlan`：文档声明 `COUNTER-OFFSET/LENGTH` 时按声明，Profile 1/2 缺省按固定布局；核心对 CAN、FlexRay 与 Ethernet 逐帧统计 counter/repeat/gap/jump，并随 Messages 行发布 `e2e` 徽标。认证/CRC 仍为 `Unverified`（不做 CMAC/密钥），其余 profile 的帧级校验与真实语料 golden 未做；Mux/Container/Transformation/E2E/SecOC 的性能已在 §7.5 量化。真实语料方面，cantools `system-4.2.arxml` 已用于验证 SecOC payload 间接引用与组级 E2E（`PROFILE-NAME` + `DATA-ID`）；组级 E2E 没有 counter/CRC 位置，导入时写一条 Unsupported 警告。该语料的 Secured 帧已有位级 golden：无头时内层 CRC 在 byte 0、4 位 SeqCounter 在 bit 8，解码 0xAA/0xB、编码写回 0x55 |
| 7 Buses UI 与工程文件 | 列表与入口已统一 | `.rxproj` 保存 schema v2 并恢复 ClusterId，generator 与 trigger 行分别以 `GeneratorCfg` / `TriggerCfgV2` 的 cluster id 写入，打开时换回 路号（条件与反应发送都走映射，映射缺失的规则丢弃而不是改指别的路）；Buses 顶部统一 Cluster 表（ID/Kind/Name/Bus/Frames/Source，Name/Source/Frames 优先取模型 cluster 与帧数），原 Add bus / Import ARXML / FR / ETH 四个入口收成一个 **Add cluster** 菜单（CAN / ARXML / FlexRay FIBEX / Ethernet ARXML）。每类详情仍按 kind 分卡；窗口/nodes/blocks/sysvars/derived/web 全部是 typed v2 段，总线描述从 `clusters` 恢复；Trace 窗口的 CAN/FR 手选以 cluster id 保存，打开时换回路号 |
| 8 清理与发布 | 清理完成（v0.30.0 已发布，本批待发版） | 生产解码已全部走模型（含 Reference、CLI CSV 与脚本 `set_sig/get_sig`）；描述表的旧编码/解码方法只在测试里保留（生产路径没有回退）；`FrBusCfg`/`EthBusCfg` 已删除，`fr_buses`/`eth_buses` 直接存 `Arc<FrDb>`/`Arc<EthDb>`，描述源路径进核心并写进模型 `Cluster.sources`；CAN 的 `Channel` 也已删除：通道就是模型里的 cluster，名称/位率/描述路径落在 cluster 与 network 参数上，节点角色与 DBC 内容校验和按 `ClusterId` 存在核心映射，`BusCore` 没有并行的总线配置向量，Buses 快照由模型/映射现算；SecOC 与 4.2 组级 E2E 已用真实语料验证并有 Secured 帧位级 golden，带 counter 偏移的 E2E 真实 golden 仍待补；VN1610 在 2026-10-30 首轮 8/8 通过，同日复跑 FD 配置被驱动拒绝（`XL_ERR_WRONG_PARAMETER`，经典仍通过），台架复位后需复验；物理拔插人工项除外 |

验证状态（2026-10-30）：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test --all-targets`（887 通过，12 项台架/性能探针显式 ignored）、前端 71 测试与 `npm run build` 全绿；release 包 0.30.0（2026-10-30 从代码 head `f5c99ab` 重建）在空目录启动 `--web` 返回 200，第二次占用同一端口按预期报错退出（exit 1）且首个进程仍存活后被显式停止；远端 CI（windows-latest）在 `f5c99ab`（真实语料与位级 golden 之后）上 fmt/build/clippy -D warnings/test 全绿；2026-10-30 用 headless Edge 在 1280×800/1024×768、明暗两套主题复核 Trace 主视图，文档宽度与视口一致、无横向溢出。Buses 已在 headless Chromium + CDP 下按 1024×768 与 1280×800、明暗两套主题复核（统一 Cluster 表、Add cluster、各 kind 详情无横向溢出）；2026-10-30 在 VN1610（sn=320097）双口对连上运行 `vector_bench_loops_classic_and_fd_both_ways`：首轮经典 500 kbit/s 与 FD 500/2000、500/5000、500/8000 kbit/s（64 字节、BRS）双向 8/8 通过；同日复跑时 FD 配置被驱动拒绝（`XL_ERR_WRONG_PARAMETER`，经典仍通过），台架/驱动复位后需复验；物理拔插恢复仍需人工复核。

剩余工作按计划顺序：重跑 VN1610 复验（硬件端口按物理路号寻址、附件跟随通道/簇已有测试锁定）；补真实多协议/SecOC/E2E/Transformation 语料与位级 golden，E2E 帧级校验与 CRC/CMAC 仍按设计保持 `Unverified`。旧运行时路径与兼容层（`Channel`/`FrBusCfg`/`EthBusCfg`）已删除；当前 ARXML 文档模型直接进核心，`SigKey` 与 Messages/Statistics 聚合都以 cluster 为身份、bus 只按映射解析；回放映射以 cluster 为目标、硬件端口仍按路号。

