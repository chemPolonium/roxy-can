# ARXML 通信层支持审计

日期：2026-10-21
基线：`roxy-can` @ `2c907d0`，对照 `roxy-arxml` 的 AUTOSAR 通信矩阵实现。

这份报告只回答“通信层面缺什么、哪些是以太网解码必需的”。编辑/写回能力不在本次范围内；除以太网必需项外，缺口记录到 `TODO.md`。

## 1. 现有 roxy-can 的 ARXML 覆盖

唯一入口是 `src/fr_db.rs::parse`，按根节点 `FIBEX`/`AUTOSAR` 或标签启发式选择解析器；ARXML 分支只认 FlexRay。

已读取：

- `I-SIGNAL` / `SYSTEM-SIGNAL`（按短名合并）、`LENGTH`、符号约定、`COMPU-METHOD-REF`。
- `COMPU-METHOD` 的线性系数与文本值表，`UNIT` 的显示名。
- `I-SIGNAL-I-PDU` / `FLEXRAY-I-PDU`、`LENGTH`、`PDU-TYPE` 动态标志。
- `I-SIGNAL-TO-PDU-MAPPING` / `I-SIGNAL-TO-I-PDU-MAPPING` 的起始位、长度、`PACKING-BYTE-ORDER`。
- `FLEXRAY-FRAME` 的帧长、`PAYLOAD-PREAMBLE`、帧→PDU 映射及起始位。
- `FLEXRAY-PHYSICAL-CHANNEL` → `FLEXRAY-FRAME-TRIGGERING` 的槽、基周期、重复、启动帧；A/B 由通道名推断。
- 第一个 `FLEXRAY-CLUSTER` 的约 50 个参数。
- `ECU-INSTANCE` 名称；通过 `I-SIGNAL-I-PDU-GROUP` + `COMMUNICATION-DIRECTION=OUT` 推导 PDU 发送方。

## 2. 已确认的缺口

| 缺口 | 现状 | 以太网必需 | 说明 |
| --- | --- | --- | --- |
| `ETHERNET-CLUSTER` / `ETHERNET-PHYSICAL-CHANNEL` | 完全不解析 | 是 | 网络、VLAN、PHY 的入口 |
| `ETHERNET-FRAME` 及 Generic/UserDefined/IEEE1722 变体 | 完全不解析 | 是 | 帧长、VLAN、触发关系 |
| `ETHERNET-FRAME-TRIGGERING` | 完全不解析 | 是 | VLAN 来自祖先 physical channel |
| `PDU-TRIGGERING` 周期 | 未读取（FlexRay 也缺） | 否 | 解码不需要周期；统计可后续 |
| ECU 端口/连接器（FRAME-PORT / PDU-PORT / I-SIGNAL-PORT） | 未读取 | 否 | 发送方/接收方判定可先用 PDU group |
| `SYSTEM-SIGNAL` → 多个 `I-SIGNAL` 扇出 | 按名称合并 | 否 | 以太网单 PDU/信号场景可先按名字 |
| `I-SIGNAL-GROUP` | 未读取 | 否 | 接口/矩阵编辑需要 |
| `UPDATE-INDICATION-BIT-POSITION` | 已读取 | 是 | 解码仅在置位时输出该信号；编码写值后置位 |
| 信号 `min`/`max` | 未读取 | 否 | Signals/DBC 范围可后续 |
| 多 cluster 引用 | 未解析，取第一个 | 是* | 同一文件多 Ethernet cluster 时需按引用选择 |
| `CAN-CLUSTER` / `CAN-FRAME` | 已实现（`src/can_arxml.rs`）：CAN 地址/帧类、帧→PDU、I-Signal、compu/单位、PDU 周期、ECU 端口发送方 | — | 多 cluster 文件按 cluster 分别导入 |
| `CONTAINER-I-PDU` | 未读取 | 否 | SOME/IP / 容器解码超出本次范围 |
| `MULTIPLEXED-I-PDU` | 未读取 | 否 | 同上 |
| `SECURED-I-PDU` | 未读取 | 否 | 认证信息不作为应用信号 |
| SoAd / socket address / SOME/IP / SD / TSN | 不支持 | 否 | 明确超出“L2 + PDU/信号”范围 |

\* 单 cluster 文件可先取第一个；多 cluster 在加载时给出诊断。

## 3. roxy-arxml 的对应实现（语义参考）

- 入口 `src/arxml/mod.rs`，读取器 `src/arxml/reader.rs`，信号 `src/arxml/signals.rs`，XML 助手 `src/arxml/xml.rs`。
- Ethernet cluster：`ETHERNET-CLUSTER` 的 `SPEED`/`BAUDRATE`、PHY、每个 physical channel 的 `VLAN-CONFIG/VLAN-IDENTIFIER`。
- Ethernet 帧：`ETHERNET-FRAME`、`GENERIC-ETHERNET-FRAME`、`USER-DEFINED-ETHERNET-FRAME`、`IEEE-1722-TP-ETHERNET-FRAME`；`FRAME-LENGTH`，无 CAN ID。
- 触发：`ETHERNET-FRAME-TRIGGERING`，VLAN 从祖先 physical channel 推导；同一帧多 VLAN 只保留第一个（`RXA-1143`）。
- ECU：`ETHERNET-COMMUNICATION-CONTROLLER/MAC-UNICAST-ADDRESS`、`COUPLING-PORT` 的 VLAN membership/default VLAN、connector。
- PDU：`I-SIGNAL-I-PDU`、`PDU-TO-FRAME-MAPPING`（`START-POSITION`、byte order）；Container/Multiplexed/Secured 有模型但本次不解码。
- 明确不支持：SOME/IP、SD、TSN、IP/SoAd/socket、交换机拓扑、untagged membership、ARXML 中不存在 MAC/EtherType。

可移植的部分是**语义**与 `src/arxml/xml.rs` 的路径索引/取值助手；`Project`/`Comm`/Newtype ID/撤销模型不适合实时分析器。

## 4. 以太网解码的额外事实

1. AUTOSAR `ETHERNET-FRAME` **不携带目的/源 MAC 和 EtherType**。仅凭 ARXML 无法把捕获帧唯一映射到帧定义，必须使用捕获头 + 可选项目覆盖。
2. `FRAME-LENGTH` 在不同 OEM 文件里可能表示 payload 长度，也可能包含 14/18 字节 L2 头。需要项目级开关，并在匹配失败时提示。
3. 同一 VLAN/长度下可能有多条帧定义；歧义时不解码，显示候选数，由用户绑定。
4. Vector BLF 的以太网对象布局尚未用真实 CANoe 样本验证；本次先按规范实现并只用合成 fixture，导入标记 experimental。

## 5. 本次实现边界

实现：

- Ethernet cluster/physical channel/VLAN。
- Ethernet 帧（四种标签）、帧触发、帧→PDU、I-Signal→PDU、compu/unit。
- ECU MAC + coupling port membership（仅用于匹配打分）。
- 帧匹配：flow key + 唯一打分 + 项目覆盖。
- L2 + 单 `I-SIGNAL-I-PDU` 信号解码。

不实现：

- Container/Multiplexed/Secured 解码。
- SOME/IP/SD/DoIP/TSN/SoAd/switch。
- ARXML 写回/矩阵编辑。
- FlexRay 现有缺口的代码修复（记入 TODO）。
- 实时硬件抓包、ASC 以太网写回、以太网录制。

## 6. 后续 TODO

- FlexRay：`PDU-TRIGGERING` 周期、ECU 端口、Signal Group、update bit、SystemSignal 扇出、min/max、cluster 引用。
- Container/Multiplexed/Secured 的以太网解码。
- SoAd/socket/SOME-IP、TSN、交换机拓扑。
- 真实 CANoe 以太网 BLF 的回归样本与对象布局确认。
- ARXML 写回与完整通信矩阵往返。
