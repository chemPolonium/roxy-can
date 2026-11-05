# TODO

只记**还没做**的事。已完成的能力见 `README.md` 与 `docs/usage.md`；定位与取舍见 `docs/product.md`；实现理由见 `docs/architecture.md`。

## 能力缺口（按当前定位）

- **Ethernet 实时口台架验证**：Npcap 收发已接；多口/时序、VLAN offload、驱动缺失与断线恢复仍需台架验证。
- **FlexRay 硬件发送**（`xlFrTransmit`）：脚本与生成器发的帧只进入本会话，被监听的端口仅接收。发送对象布局按台架实测。
- **SOME/IP 服务模拟**：Offer/StopOffer、SubscribeAck、订阅表、event 转发、method 自动应答（UDP/TCP，`someip_reply`）、Trigger TCP 已做。
- **PDU-only ARXML、UDP NM 信号与 NM 状态机**已接；CP NM 计时可用脚本 `nm_timing` 覆盖，也会读 ARXML 的 `NM-*-TIME`。
- **真实语料回归**：仍缺真实 CANoe 的以太网 BLF/PCAP 与 SOME/IP ARXML 语料；现有 BLF Ethernet 布局按 `vector_blf` 参考实现并用合成对象验证。
- **ARXML XSD 校验**：`scripts/validate-arxml.ps1` 已用 `target/refs` 的 R23-11 schema；CP 4.2.2 精确 schema 仍待补。
- **测试单元进 CI**：`--test --test-json` 已能输出 JSON/退出码；CI 尚未接。

## 体验债

- Panel 校验提示统一到字段附近并保留到修正。
- SOME/IP 发送的字段输入从 `name=value` 文本改成按 service/method 列字段。
- service instance / eventgroup 已有 Database 过滤与 endpoint 展示。
- 新增解析规则前先补真实样本，不靠合成 fixture 自证。

## 硬件与台架

- **Profile 通道号**：`profiles/*.toml` 的 `[[hw]]` 仍是裸索引，插第二台设备会指错口；改成按设备名匹配，或名字 + 编号双校验、错配整份拒绝。
- **Vector CAN FD 真机验收**：VN1610 已通过 500 kbit/s / 2 Mbit/s FD 配置与双机对连；物理拔插后的 PnP 复核仍未做，其他型号与同型号多台的离线判定待补。
- **FlexRay 真实帧**：FRLoop 运行时能否被动收到、`ab` 通道偏移尚未坐实。
- **屏幕验收**：FlexRay 下拉回显、`发送被拒 ×N`、断开后端口是否真正关闭、Vector 设备拔出后的列表刷新。

## 零散

- 脚本发送选择 BRS（Generator 的 FD/BRS 已可用）；需要时加显式选项，不用长度猜。
- 系统变量的字符串型、枚举型。
- 同一槽两个占用帧声明同名信号时曲线会合并；守卫测试常驻，真实描述触发后再改持久格式。
- 外部仿真元件的动态库加载与沙箱边界尚未设计；当前只有 per-node `HostExternFn`。
- CSV 口径细化（State Tracker 等）等明确诉求。

## 边界外（定位变了才重启）

诊断层（UDS / ODX / CDD / 刷写）；TSN / DoIP / SoAd 协议栈；通用 HMI / 任意控件 / Docking；多机分布式与 PTP / GPS；用户与权限、国际化；工业级测试报告；MF4、LIN；网关与跨总线路由。

## CI 的规矩

计时类测试一律同机对照，不退回绝对阈值；缺测试数据必须失败、不许跳过。详见 `.github/workflows/ci.yml` 顶部。




