# 完整 AUTOSAR 化计划

日期：2026-10-29
基线：`roxy-can` @ `21580f6`（805 Rust + 71 frontend tests）

目标：把工程与运行时的通信模型收敛到 AUTOSAR 的
**Cluster → Network → ECU/Port → Frame → PDU → Signal** 结构；ARXML 成为
CAN/FlexRay/Ethernet 的首选描述来源，DBC/FIBEX 只作为导入适配器。分析、
仿真、观测、触发、脚本与硬件能力不因模型切换而缩水。

> 实施状态：第 0、1 阶段已完成；第 2、3 阶段已落地垂直切片，第 4 阶段的
> CAN/FR/ETH 订阅、CAN/FR 触发器与脚本节点输入已切到统一模型，第 5 阶段的
> 三协议 Panel/Generator/脚本发送已切，第 6 阶段 mux/container/secured payload
> 已进计划，第 7 阶段已实现 v1 总线迁移、`.rxproj` schema v2 保存与 ClusterId
> 恢复，generator 与 trigger 行也以 cluster id 存入 v2；ARXML 导入把三协议的统一
> 模型直接送进核心（工程重开时按恢复后的 id 重新绑定）。Messages/Statistics 的帧
> 名与信号数、SignalPicker catalog、Buses 帧数都优先取模型；Container 按头 ID/
> 长度门控，identity/linear Transformation 执行，SecOC 与 E2E 做结构校验并明确
> `Unverified`；E2E Profile 1/2 的计数器连续性逐帧统计并进 Messages。窗口/nodes/
> derived/web 已迁为 typed v2 段。`SigKey` 全链路、窗口 picks 的 cluster 化、
> 硬件/回放 cluster 路由与旧路径清理仍未完成。逐项状态见 [`docs/autosar-model.md`](autosar-model.md) 第 10 节。

## 1. 原则

- **先冻结最终模型，再做垂直切片**：不造用完即弃的过渡模型；允许临时
  importer/adapter，但它们只向最终模型供料。
- **只读参考 roxy-arxml**：借鉴模型、字段与解析语义，不移植编辑、导出、
  写回、RXA 布局代码。
- **旧的按协议分空间只作为输入**：切换期间 `Channel` / `FrBusCfg` /
  `EthBusCfg` 可以继续工作，但不再新增依赖它们的公开接口。
- **每一切片可构建、可测试、可回退**：主分支不长期处于“只有底层没有 UI”
  的状态；大改不采用一次性大爆炸合并。

## 2. 范围与非目标

### 必须支持

- Cluster/Network、Frame、PDU、Signal、CompuMethod/DataType、初始值与量程。
- CAN（含 FD）、FlexRay、Ethernet 的 Frame/PDU/Signal 统一读入。
- ECU/Port 方向、VLAN/MAC、帧触发（CAN ID / FR slot-cycle / ETH flow）。
- 现有能力：Trace、Messages、Statistics、Signals、Data、Monitor、State、
  Triggers、脚本、Panel、Generator、录制/回放、导出、硬件通道。
- 工程保存/恢复、旧工程一次性迁移。

### 高级特性（按切片确认深度）

- Multiplexed PDU、Signal Group、Container PDU。
- Transformation（至少 identity/linear；其余按真实样本）。
- E2E（先只读校验）、SecOC（先解码校验，发送后置）。

### 非目标

- ARXML/DBC 写回、矩阵编辑、RXA 桌面布局。
- SOME/IP、SD、DoIP、TSN、SoAd/socket、交换机拓扑。
- 为了“统一”把 CAN 强行包装成不存在的 PDU 语义；PDU 只在描述确实存在时使用。

## 3. 当前基线

- ARXML 导入已能一次扫描 CAN/FlexRay/Ethernet cluster，并分别落到三种
  内部总线（`src/channel.rs::import_arxml_path`）。
- 运行时仍是三套模型：CAN `Vec<Channel>`，FR/ETH 各自
  `BTreeMap<bus, Cfg>`；`SigKey` 按 `Can {ch}` / `Fr {bus}` / `Eth {bus}`
  分空间（`src/observe.rs`）。
- Buses 的 **Add bus** 只接 CAN（`web/src/components/BusesModal.tsx`）；
  FR/ETH 入口分散在各自的 Load database 区域。
- CAN ARXML 解析器（`src/can_arxml.rs`）目前先转成 DBC 形状的
  `SymbolTable`；FR/ETH 分别有自己的数据库结构。
- 已有审计：`docs/arxml-communication-audit.md`（部分缺口随后续批次已变化，
  以本计划特性表为准）。

## 4. roxy-arxml 参考边界

同机路径：`C:/Users/chemp/source/rust/roxy-arxml`。

| 参考对象 | 用途 | 是否移植 |
| --- | --- | --- |
| `AUTOSAR.md` | Cluster/Network/ECU/Frame/PDU/Signal/关系的字段字典 | 对照裁剪 schema |
| `src/model/mod.rs`、`query.rs` | 统一实体、ID、查询形状 | 参考 |
| `src/arxml/reader.rs` | 路径索引、I-SIGNAL(Container/Multiplexed/Secured) I-PDU 读入 | 读侧语义参考，按 roxy-can 模型重写 |
| `src/arxml/xml.rs`、`signals.rs` | XML 助手、信号/compu 读取 | 参考 |
| `src/dbc_import.rs` | DBC → AUTOSAR 概念映射 | 反向参考，做 DBC importer |
| `export/`、`matrix_write/`、`apply/edit` | 编辑与写回 | **不移植** |

已知缺口：roxy-arxml 对 SecOC/E2E/Transformation 没有真正的读侧实现；
这几项必须基于真实样本新设计，不能假设参考项目已覆盖。

## 5. 最终模型（第 1 阶段冻结）

### 实体

- `Cluster { id: ClusterId, kind, name, aliases, source }`
- `Network { id, cluster, protocol, bitrate/channel params }`
- `Frame { id, cluster, network, name, length, address, timing, pdu_mappings }`
  - CAN：standard/extended/FD、CAN ID、BRS；
  - FR：slot、base cycle、cycle repetition、payload preamble；
  - ETH：VLAN、EtherType（若描述）、帧长约定、flow 匹配键。
- `Pdu { id, kind: Signal|Container|Multiplexed|Secured, length, header, mappings }`
- `Signal { id, name, bit layout, data type, compu, min/max/init, unit, values }`
- `Node/Port { ecu, direction, connector, frame/pdu/signal refs }`
- 高级元数据：mux selectors、container members、signal group、transformation、
  E2E profile、SecOC 参数（按特性表决定是否执行）。

### 标识与 key

- `ClusterId`：工程内持久化、全局唯一；不再用“第几路 CAN / 第几路 FR”做身份。
- `FrameId` / `PduId` / `SignalId`：由模型导入时分配并持久化；重命名不改 id。
- `SigKey` 最终形态：`cluster + frame + pdu? + signal`，编译/保存为稳定文本
  key；脚本与触发器的显示别名保留，但解析以 id 为准。
- 旧 key（`0:256:Name`、`fr:0:25:X`、`eth:0:Frame:X`、`derived:id`）在迁移期
  解析为 v2 id；迁移完成后只写 v2。

### 运行时

- **展开计划缓存**：导入后把 Frame→PDU→Signal 展开为位读取/写入计划，
  每帧解码不遍历模型树；Container/Mux 展开结果带版本。
- **采样与订阅**：沿用现有 `SampleCache` / `SigKey` 订阅模型，只替换 key 与
  帧路由。
- **编码**：统一 Frame/PDU/Signal 编码器；Generator、脚本、Panel、触发器共用。
- **持久化**：工程 schema v2；`clusters` 为唯一总线列表，兼容字段只读迁移。

## 6. 阶段计划

每个阶段的“退出条件”是硬门禁，完成一个阶段再进入下一个；不按日历排期。

### 0. 需求与语料

- 冻结特性表（必须 / 高级 / 不做）；为每个特性准备真实 ARXML 样本。
- 审计 roxy-arxml 模块与字段映射表；输出 `docs/autosar-model.md` 初稿。
- 退出：能逐项回答“该 ARXML 元素在 roxy-can 中对应什么/不支持时如何报错”。

### 1. 冻结最终模型与工程 schema

- 定义实体、ID、`SigKey` 文本形态、工程 schema v2、适配器 trait。
- 建立单元测试与 golden 快照；旧运行时暂不切换。
- 退出：模型/schema 编译通过；新代码不依赖旧三套总线类型的公开契约。

### 2. 新 ARXML 导入器与模型构建

- 以 `reader.rs` 读侧语义为参考，直接构建统一模型（不再先转 DBC 形状）。
- 基础帧/信号、compu/min/max/init、ECU/Port、triggering；高级元素先入模型。
- 退出：真实多协议 ARXML 生成完整模型；现有 ARXML 语料映射成功。

### 3. DBC / FIBEX 适配器

- DBC → CAN Cluster；FIBEX → FR/ETH Cluster；复用现有解析器，只输出最终模型。
- 等价性测试：同一逻辑数据库经 DBC 与 ARXML 导入后，解码结果一致。
- 退出：三种导入方式只产出 Cluster；模型层无 DBC 特有 API。

### 4. 运行时切换：解码/订阅/Trace/Messages/Statistics

- BusCore 按 Cluster 路由；解码走展开计划；`SigKey` 全链路迁移。
- 性能门槛：50k trace、8×12k 曲线、10Hz 发布不低于当前基线。
- 退出：三协议流量在新模型下可观察；旧测试迁移后全绿。

### 5. 发送/仿真/硬件/回放

- Generator、脚本 `send/frame`、Panel Setpoint/Send、触发器动作统一编码。
- 硬件通道、FR 端口、Ethernet 会话帧、VN1610 验收重跑。
- 回放映射改为文件通道 → `ClusterId`；录制/导出口径回归。
- 退出：硬件收发、回放、录制、脚本发送验收通过；旧工程迁移后不回退。

### 6. 高级 AUTOSAR 特性切片

每个切片都包含“导入 → 解码/编码 → 观测/发送 → 测试”：

1. Multiplexed PDU / Signal Group
2. Container PDU
3. Transformation（identity/linear 优先）
4. E2E（只读校验 + 统计）
5. SecOC（解码校验优先，发送后置）

- 退出：每项有真实样本、位级 golden 测试、明确的不支持报错与性能数据。

### 7. Buses UI 与工程迁移

- 统一 Cluster 列表（Kind 列）、**Add cluster**、按扩展名自动选 importer；
  每类 cluster 的详情仍按 kind 展示。
- 工程 v1 → v2 迁移器与迁移报告；旧字段只读后不再写回。
- 明暗主题、1024×768 / 1280×800、弹窗与主标签页复核。
- 退出：旧工程打开/保存/再打开一致；Buses 不再有三套并列入口。

### 8. 收尾与发布

- 删除旧 `Channel`/`FrBusCfg`/`EthBusCfg` 运行时路径与兼容层。
- 全量测试、clippy、CI、release smoke（空目录、端口冲突、硬件）。
- 更新 README/architecture/usage/script/roadmap，发布新 minor。

## 7. 里程碑

| 里程碑 | 内容 |
| --- | --- |
| M0 | 特性表、语料、模型字段映射 |
| M1 | 最终模型 + schema v2 冻结 |
| M2 | ARXML → 统一模型 |
| M3 | DBC/FIBEX 适配器 + 等价性 |
| M4 | 运行时切换 + 性能达标 |
| M5 | 发送/硬件/回放切换 |
| M6 | 高级特性切片 |
| M7 | UI 与迁移 |
| M8 | 清理与发布 |

M4–M5 可与 M7 的 Buses UI 部分并行，但模型与 key 迁移必须先行；高级特性做到 SecOC/E2E 深水区时，M6 会拉长。

## 8. 测试与验收

- **golden 语料**：多 cluster ARXML、Container/Mux、SecOC/E2E、真实 OEM 矩阵；
  导入快照 + 位级解码 + 编码回读。
- **等价性**：DBC/FIBEX/ARXML 对同一逻辑库的帧、信号、量程、初始值一致。
- **性能**：解码 p50/p99、50k trace 重建、发布 JSON 大小、订阅内存。
- **回归**：现有 805 Rust + 71 前端测试按新契约迁移，断言不降级；硬件与
  回放用例保留。
- **UI**：1024×768 与 1280×800、明暗主题、所有 Buses 弹窗/选择器无横向溢出。
- **迁移**：旧工程 × 若干真实样本；迁移后保存再打开逐字段对比。

## 9. 风险与决策点

1. **ClusterId 策略**：全局持久化 id（推荐）还是 kind+序号；决定持久化与硬件映射。
2. **SigKey 文本**：是否保留旧别名解析；决定脚本/触发器/工程迁移成本。
3. **高级特性深度**：SecOC/E2E/Transformation 做到“只读校验”还是“可发送”。
4. **旧工程兼容**：按现有规则可不兼容旧版，但必须提供一次性迁移与报告。
5. **性能**：PDU/Container 展开缓存必须从第 4 阶段起就有基准，否则后期难补。

## 10. 起始批次

1. 建 `docs/autosar-model.md`：实体字段与 roxy-arxml `AUTOSAR.md` 对照表。
2. 收集 3 份 ARXML（多协议、Container/Mux、SecOC/E2E），记录来源与期望。
3. 画出现有 `SigKey`/`Channel`/`FrBusCfg`/`EthBusCfg` 的引用图，标出第 4 阶段迁移面。
4. 输出 schema v2 草案（JSON 示例 + 迁移映射：旧字段 → 新字段）。
5. 评审并冻结决策点 1–3，再进入第 1 阶段编码。
