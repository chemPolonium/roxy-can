# TODO

只记**还没做**的事。已完成的能力见 `README.md` 与 `docs/usage.md`；批次明细在 git log；设计决策的理由在 `docs/architecture.md`。

## FlexRay

- **FlexRay 硬件发送**（`xlFrTransmit`）：脚本与生成器发的帧目前只进入本会话，被监听的端口是仅接收的。CH1 已能在真机上配置成功，缺的是发送对象的布局——台架现在可用，布局按实测取，别照记忆写。
- **真实帧进入 Trace**：FRLoop 跑着时能否被动收到没测过；`ab` 通道偏移也还没坐实。
- **订阅键含帧名**：同一槽的两个占用帧若声明同名信号，两条曲线会并成一条。手上两份描述实测 0 处，守卫测试常驻；真实描述让它红了再动持久格式。

## 硬件

- **Profile 里的通道号会随硬件变**：`profiles/*.toml` 的 `[[hw]]` 存的是裸索引，插上第二台设备就指错口——而 Profile 的卖点正是"同一套工程在仿真 / 台架 / CI 间零修改切换"。改成按设备名匹配（或名字 + 编号双校验、错配整份拒绝）。
- **Vector CAN FD 真机验收**：VN1610 在启动前插入时已通过 500 kbit/s / 2 Mbit/s 的 FD 配置；2026-10-26 两路 VN1610 对连实测：500 kbit/s 仲裁段下，经典帧双向、FD 64 字节、带 BRS 的 2/5/8 Mbit/s 数据段均能对向正确收到（`txFail` 为空，Trace 的 FD·B/长度/载荷一致）；Generator 已有 FD/BRS 选项。Windows PnP 监听已实现（拔出时关闭 Vector 连接、重插后重开 XL 驱动并刷新列表），但**物理拔插的台架复核仍未做**；其他 Vector 型号的 PnP 映射、同型号多台设备的逐台离线判定也待补。
- **屏幕验收**：FlexRay 行的下拉回显；`发送被拒 ×N` 在真机上亮一次（两设备对发通着时得让对端停掉才会出现）；`断开` 之后端口是不是真关；Vector 设备拔出后刷新是否仍列出。

## 零散

- 外部仿真元件动态库加载：目前只有 per-node `HostExternFn` 宿主钩子；动态库加载、插件约定与沙箱边界尚未设计。
- 同值双类报文在列表里只列一条（标准优先）：等真实双类库需求。
- 系统变量：字符串型、枚举型。
- 脚本发送尚不能选择 BRS（Generator 的 FD/BRS 已可用）；需要时加显式发送选项，不要用长度猜。
- CSV 口径细化（State Tracker 等）：等明确诉求。
- 独立的时间范围双输入框：现有跳转已覆盖，低优先级。

## 以太网（进行中）

- **范围**：ARXML + 离线 Vector BLF；L2 + AUTOSAR PDU/信号；不做实时硬件、ASC/PCAP、SOME/IP/SD/DoIP/TSN/SoAd、Container/Multiplexed/Secured 解码、ARXML 写回。
- **真实样本**：目前没有真实 CANoe 以太网 BLF，导入先按规范 + 合成 fixture，标记 experimental；拿到样本后确认对象布局。
- **帧匹配**：AUTOSAR 的 `ETHERNET-FRAME` 不含 MAC/EtherType，捕获帧只能按 flow key + 打分匹配；歧义必须由项目覆盖解决，不能猜。
- **FRAME-LENGTH**：项目级开关已加（Buses「FRAME-LENGTH 含 L2 头」，随工程保存），声明长度含 14 字节头时按「-14」匹配。
- **回放通道映射**：CAN / FlexRay / 以太网三套日志编号都能在 Buses 映射到本工程的路；以太网帧先映射再解码，静音与计数按文件编号。
- **Trace/作用域**：CAN/FlexRay/Ethernet 已合并在同一张按时间排序的表；作用域提供 `Ethernet: ETHn`，Manual 以 L2 flow 为选择键并随工程保存。Trace 导出按扩展名选格式：`.asc` 写 CAN/FlexRay，`.pcap` 写以太网；Messages/Statistics CSV 同样包含以太网 flow。
- **导出保真**：PCAP 为经典格式，不写 FCS，也不带方向位（Rx/Tx 都按线上帧导出）；S-VLAN/C-VLAN 的 TPID 从 BLF 保留。
- **只收不发**：以太网帧与 flow 在 Trace / Messages / Statistics 中可见并可解码；没有以太网硬件发送路径/生成器（与 FlexRay 的只收端口同一边界）。Panel Setpoint 可会话内注入一帧（矩阵 VLAN、EtherType 或 `0x88B5`、固定本机管理 MAC），不录制、不出端口。
- **后续**：Container/Multiplexed/Secured 以太网解码；SoAd/socket/SOME-IP；TSN/交换机拓扑；ARXML 写回与完整矩阵往返。

## ARXML 通信层（审计见 `docs/arxml-communication-audit.md`）

- FlexRay 现有缺口的代码修复：`PDU-TRIGGERING` 周期、ECU 端口/连接器、Signal Group、update bit、SystemSignal 扇出、min/max（CompuMethod 限值已读，`DATA-CONSTR` 未读）、cluster 引用。
- CAN ARXML 的 Container/Multiplexed/Secured PDU、Signal Group、update bit、PDU 组/端口方向（目前只认 `FRAME-PORT-REF`）；当前只读普通 `I-SIGNAL-I-PDU`。
- 以太网必需的子集随以太网功能实现；其余只记录，不在本次补。

## Panel 操作页

- **Panel 组写已完成**：Button 的 Signal writes 可一次写一组信号（逐项继续、汇总失败），随页面保存与 `.rxpanel` 导入导出。
- **动作返回值**：`on panel_action` 目前只有输入参数，没有把脚本结果送回按钮/提示的通道；要不要加、怎么与“效果集可静态推导”共存，先决策再加。
- 页面模板库（内置示例页）价值低，低优先级。

## 脚本调试器

- 条件/命中次数断点、监视表达式、step over/out 尚未做；目前只有行断点与行级
  step。
- 暂停期间 core 线程被挡住，采集/回放同步停住（有意的最简模型）；若要求暂停时
  不丢总线事件，需要把节点 VM 搬到独立线程并排队事件。

## 远期候选

- 测试单元的 CI 结论：GUI 已有 `on stop` + `test()` 与通过/失败列表；无头跑一段仿真、输出 JSON/退出码尚未做。
- 游标 Peek（零配置）：对当前筛选出的信号在游标时刻按需查询，不建立常驻订阅；显式 Read 集已可用，Peek 作为后续补充。
- 脚本编辑器的自定义关键字着色：走上游，不再 fork。

## 边界外（定位变了才重启，不许逐步混入）

面板设计器 / 通用 HMI（任意控件、动态事件模型、Docking）；固定画幅的操作 Panel 已在范围内，不属于这里。换脚本底座（Lua / JS / Python）；工业级测试报告（HTML / XML / PDF）；多机分布式与 PTP / GPS 时间同步；用户与权限、国际化；网关与跨总线路由；诊断层（UDS / CDD / ODX / 刷写）；MF4 读取（缺真实样本，无从验证）；LIN。（以太网已单独立项，见上。）

## CI 的规矩

计时类测试一律同机对照，不许退回绝对阈值；缺测试数据必须失败、不许跳过。详见 `.github/workflows/ci.yml` 顶部。
