# TODO

只记**还没做**的事。已完成的能力见 `README.md` 与 `docs/usage.md`；定位与取舍见 `docs/product.md`；实现理由见 `docs/architecture.md`。

## 能力缺口

- **Ethernet 实时口台架验证**：Npcap 收发已接；多口/时序、VLAN offload、驱动缺失与断线恢复仍需台架验证。
- **FlexRay 硬件回环验收**：`xlFrTransmit` 已接，生成器/脚本/触发器/Panel 帧可写端口；单口不能自收，需第二个 FR 节点/接口验证真实收帧、槽/cycle 解码与 A/B 偏移。
- **真实语料回归**：仍缺真实 CANoe 的以太网 BLF/PCAP 与 SOME/IP ARXML 语料；现有 BLF Ethernet 布局按 `vector_blf` 参考实现并用合成对象验证。
- **ARXML XSD 校验**：CP 4.2.2 精确 schema 仍待补；R23-11 已可用。
- **测试单元进 CI**：`--test --test-json` 已能输出 JSON/退出码；CI 尚未接。

## 硬件与台架

- **Profile 通道号**：`profiles/*.toml` 的 `[[hw]]` 仍是裸索引，插第二台设备会指错口；改成按设备名匹配，或名字 + 编号双校验、错配整份拒绝。
- **Vector 拔插验收**：VN1610 的 500 kbit/s / 2 Mbit/s FD 配置与双机对连已通过；物理拔插后的 PnP 复核、其他型号与同型号多台的离线判定待补。
- **FlexRay 真实帧回环**：VN7640 FlexRay RX 口已完成枚举、FIBEX/ARXML 配置打开、TX 调用和 App attach / start / stop 自检；单口实际收帧、槽/cycle 解码与 `ab` 通道偏移仍未验收。
- **屏幕验收**：FlexRay 下拉回显、`发送被拒 ×N`、断开后端口是否真正关闭、Vector 设备拔出后的列表刷新。

## 零散

- 系统变量的真正字符串值；Bands 已支持数值区段，数值仍是运行时类型。
- 同一槽两个占用帧声明同名信号时曲线会合并；守卫测试常驻，真实描述触发后再改持久格式。
- 外部仿真元件的动态库加载与沙箱边界尚未设计；当前只有 per-node `HostExternFn`。
- CSV 口径细化（State Tracker 等）等明确诉求。
