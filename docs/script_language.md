# roxy-can 节点脚本语言参考手册

类 CAPL 的 CAN 总线仿真脚本语言。编译成字节码由内置栈式虚拟机执行，
每个回调限 10 万条指令 + 256 层递归深度，防止死循环卡死总线线程。

## 语法概览

```c
// 全局变量（int / float / bool / string）
let base = 800;
let name = "node";

// 用户函数（支持递归）
fn limit(v) {
    if (v > 5000) { return 5000; }
    return v;
}

// 事件处理器
on start {
    print("node up");
}

on message 0x100 {
    let rpm = sig(0x100, "RPM");
    print("rpm:", rpm);
}

on extended message 0x50 { }    // 显式扩展帧（数值可 ≤ 0x7FF）

on message * { }        // 任意帧（嗅探/网关）

on fr slot 13 { }       // FlexRay 槽 13 有帧到达（所有路）

on fr 0 slot 13 { }     // 只有 FR0 的槽 13

on errorFrame { }       // 错误帧

on timer 100 { }        // 周期定时器

on timer "name" { }     // 命名一次性定时器（配合 set_timer）

on start {
    set_timer("name", 500);
}

on message 0x300 {
    // 按值分发：各分支互斥，无贯穿，不需要 break
    switch (frame_byte(0)) {
        case 1: { send(0x301, 1); }
        case 2: { send(0x302, 2); }
        default: { send(0x3FF, 0); }
    }
}
```

## 数据类型

| 类型 | 字面量示例 | 说明 |
|------|-----------|------|
| int | `42`, `0x1F` | 64 位有符号整数 |
| float | `3.14` | 64 位 IEEE 754 |
| bool | `true`, `false` | |
| string | `"hello"` | UTF-8，支持 `\n` `\t` `\"` `\\` |
| bytes | `bytes(8)` | 引用语义字节缓冲，`buf[i]` 读写 |
| array | `array(3)` | 引用语义定长数组，元素为任意值，`a[i]` 读写 |
| nil | — | 空值（未初始化变量的默认值） |

整数运算精确（溢出报错），混合运算提升为浮点。`+` 任一侧为 string
时按 print 格式拼接。

## 变量

```c
let x = 10;          // 全局变量（顶层 let）
x = x + 1;           // 赋值
x += 5;              // 复合赋值：等价于 x = x + 5

fn f() {
    let y = 5;       // 局部变量（函数作用域）
    y = y + 1;
    if (y > 3) {
        let z = y * 2;  // 块作用域
        print(z);
    }
    // z 在此处已出作用域
}
```

复合赋值全家：`+=` `-=` `*=` `/=` `%=` `&=` `|=` `^=` `<<=` `>>=`，
缓冲元素同样适用（`buf[0] |= 0x80`）。

## 控制流

```c
if (condition) { ... } else { ... }

while (condition) { ... }

for (let i = 0; i < 10; i = i + 1) { ... }

switch (value) {
    case 1: { ... }
    case 2: { ... }
    default: { ... }    // 可选
}
```

条件必须是 bool。短路求值：`&&` 和 `||`。

`switch` 对主体的每个候选值依次比较（`==` 语义），**命中即执行对应
分支，各分支互斥**——没有 C 式贯穿（fallthrough），也不需要 `break`。
`default` 命中不了任何 case 时执行，可省略。主体只在比较开始时求值
一次；case 分支内声明的局部变量只在该分支可见。

`break` 跳出最内层循环；`continue` 跳过本轮循环体剩余部分：
在 `while` 中回到条件判断，在 `for` 中先执行步进语句再判断条件。
两者只能出现在循环体内，否则编译报错。

## 运算符

| 类别 | 运算符 |
|------|--------|
| 算术 | `+` `-` `*` `/` `%` |
| 移位 | `<<` `>>` |
| 比较 | `==` `!=` `<` `<=` `>` `>=` |
| 位运算 | `&` `^` `\|`（优先级介于比较与逻辑之间，同 C） |
| 逻辑 | `&&` `||` `!` |
| 一元 | `-` `!` |

`+` 任一侧为 string 时执行拼接（另一个值自动转为文本）。
位运算只接受整数：浮点不提升（直接报错），移位量在 0..64 之外报错。
优先级从高到低：一元 → 乘除取模 → 加减 → 移位 → 比较 → `&` →
`^` → `\|` → `&&` → `||`。

## 事件处理器

### on start

测量启动时执行一次。

```c
on start {
    print("ready");
}
```

### on message \<id\>

收到指定 ID 的帧时触发。id ≤ 0x7FF 只匹配标准帧；更大的 id 匹配
29 位扩展帧（与 `send` 的判断规则一致，同值的标准/扩展帧互不串扰）。

```c
on message 0x100 {
    // frame_byte(n): 触发帧的第 n 字节
    // frame_dlc():  触发帧的数据长度
    print("got", frame_byte(0), frame_dlc());
}

on message 0x1C3D1E5 {
    // 扩展帧
}
```

### on message \*

收到本信道**任何数据帧**时触发（错误帧只走 `on errorFrame`），
配合 `frame_byte` / `frame_id` 可写网关、记录器、协议嗅探节点。
注意节点也会收到自己 `send` 出去的帧——在通配处理器里无条件发帧会自激成环。

```c
on message * {
    if (frame_id() != 0x700) {   // 排除自己的转发帧
        print("seen 0x", frame_id());
    }
}
```

### on extended message \<id\>

显式匹配**扩展帧**：数值 ≤ 0x7FF 的扩展帧（合法但不常见）只有这种
写法能寻址，普通 `on message 0x50` 只匹配标准帧。

```c
on extended message 0x50 { }   // 只匹配扩展帧 0x50
```

配套发送用 `send_ext`：与 `send` 参数相同，但**永远按扩展帧发送**，
即使 id ≤ 0x7FF。

```c
on extended message 0x50 {
    send_ext(0x51, frame_byte(0));   // 应答也走扩展帧
}
```

### on fr slot \<槽号\> / on fr \<路\> slot \<槽号\>

FlexRay 帧到达时触发。**寻址用槽而不是帧名**：一个静态槽按周期相位可以轮流排好几帧，
"到达"这件事发生在槽上，所以 `on fr slot 13` 会为该槽实际到达的**每一帧**跑一次
（哪一帧在跑，就按哪一帧的布局解码）。路号可选——槽号在两路 cluster 上会重复，
所以 `on fr 0 slot 13` 只看 FR0，`on fr slot 13` 看所有已配置的路。节点本身绑在
一条 CAN 通道上，FlexRay 帧没有 CAN 通道可匹配，因此**路号是唯一的过滤条件**
（不写就是全部路）。

处理器里的帧上下文是这次到达本身：

| 内建 | 在 `on fr slot` 里的含义 |
|------|--------------------------|
| `frame_id()` | 槽号（**不是** CAN id） |
| `frame_dlc()` | 载荷字节数 |
| `frame_byte(i)` | 载荷第 i 字节 |
| `fr_cycle()` | 该帧到达时的通信周期号（0..63，非 FlexRay 事件里为 0） |

```c
on fr 0 slot 13 {
    // 读到这一帧刚带回的信号值（ingest 先于派发，所以不会读到上一拍）
    let speed = fr_sig(0, 13, "CarSpeed");
    print("slot", frame_id(), "cycle", fr_cycle(), "speed", speed);
    send(0x320, speed * 0.075);   // 应答走节点的 CAN 通道
}
```

写方向仍然只有 CAN：FlexRay 发车（静态/动态槽发送）尚未实现，所以这个处理器能
"听 FlexRay、答 CAN"（网关形状），但没有 `fr_send`/`set_fr_sig`。
`send(frame_id())` 在这种处理器里是**编译错误**——把槽号当 CAN id 发出去正是
两套编号混用最容易犯的错，静态检查直接拦下。

### on errorFrame

收到错误帧时触发。此时 `frame_dlc()` 为 0，`frame_byte(n)` 会越界报错。

```c
let errors = 0;

on errorFrame {
    errors = errors + 1;
    print("error frame seen, total", errors);
}
```

### on timer \<ms\>

周期定时器，每 N 毫秒触发一次。

```c
on timer 100 {
    // 每 100ms 执行
}
```

### on timer "\<名称\>"

命名一次性定时器：平时不运行，被 `set_timer` 武装后延迟触发**一次**。
适合"收到某报文后 200ms 再应答"这类延迟反应。

```c
on message 0x100 {
    set_timer("resp", 200);   // 每次收到 0x100 重新武装
}

on timer "resp" {
    send(0x101, frame_byte(0));
}
```

### 定时器控制内建

| 内建 | 说明 |
|------|------|
| `set_period(ms)` | 修改当前定时器的周期（一次性定时器中等价于重新武装） |
| `stop_timer()` | 停止当前定时器 |
| `set_timer("名称", ms)` | 武装命名一次性定时器，ms 毫秒后触发一次；可在任意回调中调用 |
| `cancel_timer("名称")` | 取消已武装的一次性定时器 |

`set_timer` 指向不存在的处理器时记一条日志告警，不影响运行。

## 总线内建

### send(id, byte0, byte1, ...) / send(id, buffer)

发送经典帧。id ≤ 0x7FF 为标准帧，> 0x7FF 为扩展帧。
载荷为 0..255 的整数字节或一个 bytes 缓冲；浮点值自动向零截断，
所以波形内建可以直接喂进载荷（`send(0x100, ramp(0, 255, 1))`）。
缓冲超过 8 字节自动按 CAN FD 发送（长度取整到合法 FD 长度，上限 64）。
需要显式扩展帧时用 `send_ext`（参数相同，永远扩展）。

```c
send(0x123, 0x01, 0x02);
send(0x200, buf);
```

**id 必须可静态推导**（编译期规则）：字面量（如 `0x123`）、对字面量的
常量算术（如 `0x100 + 0x20`）、或 `frame_id()`。写 `send(n, ...)` 这类
变量 id 会**编译报错**——工具需要静态知道"这个节点可能发哪些报文"
（与 DBC 声明对账、CI 门禁）。动态 id 的转发场景用 `on message *` +
`send(frame_id(), ...)` 表达。

### sig(id, "Name")

读取信号 `Name` 在报文 `id` 上的最新物理值。信号尚未出现时产生运行时错误。

```c
let rpm = sig(0x100, "EngineSpeed");
```

### fr_sig(cluster, slot, "Name")

FlexRay 侧的同一种读数：该路集群（Trace/统计里显示为 `FR{n}` 的那个编号）上，
槽 `slot` 最近一帧解出的信号 `Name` 物理值。信号尚未出现时同样产生运行时错误，
不会静默给 0。

```c
let speed = fr_sig(0, 13, "CarSpeed");
emit_value("SpeedKmh", speed * 0.075);
```

用槽而不是帧名寻址，是因为槽才是"到达"与"调度"共同说得清的东西：一个静态槽按
周期相位可以轮流属于好几个帧，脚本读到的是**实际到达那一帧**按自己的布局解出的
值。集群编号是显式的——两路 cluster 上的同号槽是两个不同的读数，而脚本节点属于某条
CAN 通道，没有"本节点所在的那一路"可推断。

写方向目前没有：FlexRay 发车（静态/动态槽发送）尚未实现，所以没有
`set_fr_sig`/`fr_send`。读的方向除了随手查值，也能被到达唤醒：见
`on fr slot`，处理器里 `fr_sig(路, 槽, "Name")` 读到的就是刚到的那一帧。

### set_sig(buffer, id, "Name", value)

将物理值编码到字节缓冲中。buffer 不足 8 字节自动补齐。

```c
let buf = bytes(8);
set_sig(buf, 0x200, "RPM", 3000);
send(0x200, buf);
```

### get_sig(buffer, id, "Name")

从字节缓冲中按 DBC 定义解码信号物理值。

```c
let v = get_sig(buf, 0x200, "RPM");
```

### emit_value("Name", 表达式)

发布**派生信号**：名字 + 本节点算出的一个值。表达式的求值就是
派生逻辑本身——`sig()` 读数、数学/波形内建、任意脚本逻辑都能参与。
派生信号作为合成订阅流进入程序，可在 Graphics / Data 的信号选择树
"派生信号"分组里勾选，像数据库信号一样画曲线、看数值。空名字或
非数值是运行时错误。整个会话最多 256 个不同名字（防脚本循环造名），
超出后新名字的发布被忽略。

```c
// 换算 + 组合：扭矩 = 转速 × 透析系数，单位换算等
on timer 10 {
    emit_value("SpeedKmh", sig(0x100, "EngineSpeed") * 0.075);
}
```

### sys_get("ns::name") / sys_set("ns::name", value)

读写**系统变量**——在 View > System Variables 管理器里定义的
命名空间值（namespace + name + 初值 + 可选界限 + 单位/备注，定义随
工程保存）。每次测量开始所有变量复位为声明的初值；每个变量同时发布
为一条可观测流（信号选择树里按 namespace 分组），Data / Graphics
像数据库信号一样选用。写入会被夹到定义的界限内；引用未定义的变量
在节点启动时报出来，运行中写未定义变量被丢弃并记日志（节点不停跑）。

```c
on timer 50 {
    // 读控制器写的目标值，向下发布实际值
    let target = sys_get("Demo::Setpoint");
    sys_set("Demo::Actual", target * 0.9);
}
```

## 帧数据访问内建

| 内建 | 说明 |
|------|------|
| `frame_byte(n)` | 触发帧的第 n 字节（越界报错） |
| `frame_dlc()` | 触发帧的数据长度 |
| `frame_id()` | 触发帧的 ID（通配处理器里可据此过滤/转发/防自环）；在 `on fr slot` 里是**槽号** |
| `fr_cycle()` | `on fr slot` 处理器里这次到达的通信周期号；其他处理器里为 0 |

帧访问内建在 FlexRay 处理器里同样可用，只是含义换成"这一帧"的（见
"事件处理器"一节的 `on fr slot`）；两套编号共用 `frame_id()` 这个数字，所以
`send(frame_id())` 在 `on fr slot` 里被编译期拒绝。

## 数学内建

| 内建 | 说明 |
|------|------|
| `abs(x)` | 绝对值 |
| `floor(x)` / `ceil(x)` / `round(x)` | 取整 |
| `sin(x)` / `cos(x)` | 三角函数（弧度） |
| `min(a, b)` / `max(a, b)` | 最小/最大 |
| `clamp(v, lo, hi)` | 限幅 |
| `random(lo, hi)` | 均匀随机浮点 |
| `srand(seed)` | 重置随机种子 |

## 位运算内建

操作符 `&` `|` `^` `<<` `>>` 之外，这些内建等价可用（历史原因保留）：

| 内建 | 说明 |
|------|------|
| `bit_and(a, b)` | 按位与 |
| `bit_or(a, b)` | 按位或 |
| `bit_xor(a, b)` | 按位异或 |
| `bit_not(a)` | 按位取反 |
| `bit_shl(a, n)` / `bit_shr(a, n)` | 左移/右移 |

## 信号与系统变量的速记（CANoe 风格）

读信号、读系统变量除了函数形式，还有两种**等价的速记**——名字都是
字面量，编译期进静态事实表，装配时对照 DBC / 系统变量表校验（拼错
是显式失败）：

```c
// 读信号：$报文名::信号名 —— 等价 sig(id, "信号名")，id 由库名字解析
let rpm = $EngineStatus::RPM;

// 读系统变量：@sysvar::命名空间::变量名 —— 等价 sys_get("ns::name")
let target = @sysvar::Demo::Setpoint;

// 写系统变量：赋值形式 —— 等价 sys_set("ns::name", v)（写入仍受界限钳位）
@sysvar::Demo::Setpoint = target * 0.9;
```

信号**没有**赋值速记：写信号走 `set_sig(buffer, id, "Name", value)`
的缓冲语义，`$...` 只读。启动检查（编辑器收发 tab / 节点日志 /
`--check-script --dbc`）会对两种速记的名字做 DBC 核对。

## 波形内建

周期波形与 TX 发生器共用同一套求值器，同参数下逐采样一致。

| 内建 | 说明 |
|------|------|
| `ramp(lo, hi, period_s)` | 锯齿波，从 lo 线性升到 hi，周期 period_s 秒 |
| `triangle(lo, hi, period_s)` | 三角波，半周期处到达峰值 hi |
| `square(lo, hi, period_s)` | 方波，前半周期 lo，后半周期 hi |
| `counter(lo, hi, period_s)` | 滚动计数器，从 lo 到 hi 整步递增 |
| `sine_wave(offset, amplitude, period_s)` | 正弦波，中心 offset，振幅 amplitude（独立参数化，起点在中心） |

均以总线时钟为时基。`hi < lo` 时 ramp/triangle/counter 反向。

## 其他内建

| 内建 | 说明 |
|------|------|
| `print(v0, v1, ...)` | 输出到节点日志 |
| `format(fmt, v0, ...)` | printf 风格格式化为字符串：`%d` `%u` `%x` `%X` `%f`（默认 6 位小数）、`%.Nf`、`%e` `%g`、`%s`，宽度和 `-`/`0` 标志可用（`%05d`、`%-8s`），`%%` 为字面百分号 |
| `now()` | 总线时钟（秒，浮点） |
| `bytes(n)` | 分配 n 字节缓冲（零填充） |
| `array(n)` | 分配 n 元定长数组（nil 填充，元素任意值，引用语义） |
| `len(v)` | 缓冲 / 数组长度或字符串字符数 |

## 外部函数（扩展缝）

外部仿真元件（电池模型、诊断栈、被控对象仿真等）在进程内注册函数，
任何脚本都可以直接调用，与内建写法相同：

```c
let soc = battery_soc();          // 外部注册的无参函数
send(0x321, pack_soc(soc));       // 与内建混用
```

- 注册方为 Rust 侧代码：`roxy_can::script::register_extern("名称", 函数)`，
  与内建同名注册会被拒绝
- 未注册的函数名**可以编译**，但调用时报运行时错误，便于分阶段接线
- 每个节点还可以挂私有钩子（如 `set_sig`/`get_sig` 走信道数据库），
  解析顺序：注册表 → 节点钩子

## 编译错误 vs 运行时错误

- 编译错误在 Apply 时报出，格式 `line N: 消息` 或 `line N:M: 消息`（N:M 为行:列）
- 运行时错误终止当前回调，格式 `line N: 消息`，节点进入错误状态
- 节点错误状态通过重新 Apply 或重启测量清除
