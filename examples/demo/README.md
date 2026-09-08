# demo 工程包

一个最小可跑的工程（单总线 Powertrain，挂 `assets/sample.dbc`）加两个
Profile 档案，用来快速体验"一套工程 + 多环境角色覆盖"：

```bash
# bench 台架：EngineECU 由本工具模拟
roxy-can --project examples/demo/demo.rxproj --profile bench --duration 2 --stats smoke.csv

# CI 冒烟：同上（内容见 profiles/ci.toml，可按需改成全静默）
roxy-can --project examples/demo/demo.rxproj --profile ci --duration 2

# 不带 --profile：按工程原样打开，不发车
roxy-can --project examples/demo/demo.rxproj --duration 2
```

DBC 路径相对工程目录解析，工程文件夹整体挪动不失效；角色词错配时
Profile 整份拒绝并在标准错误给出原因。
