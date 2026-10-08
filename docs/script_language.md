# 节点脚本语言参考手册

类 CAPL 的总线仿真脚本语言，编译后由内置虚拟机执行。每个回调限 10 万条指令、256 层递归，超限即熔断停跑，坏脚本卡不死总线。

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

整数运算精确（溢出报错），混合运算提升为浮点。`+` 任一侧为 string 时按 print 格式拼接。

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

复合赋值全家：`+=` `-=` `*=` `/=` `%=` `&=` `|=` `^=` `<<=` `>>=`，缓冲元素同样适用（`buf[0] |= 0x80`）。

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

`switch` 对主体的每个候选值依次比较（`==` 语义），**命中即执行对应分支，各分支互斥**——没有 C 式贯穿，也不需要 `break`。`default` 命中不了任何 case 时执行，可省略。主体只在比较开始时求值一次；分支内声明的局部变量只在该分支可见。

`break` 跳出最内层循环；`continue` 跳过本轮循环体剩余部分：在 `while` 中回到条件判断，在 `for` 中先执行步进语句再判断条件。两者只能出现在循环体内，否则编译报错。

## 运算符

| 类别 | 运算符 |
|------|--------|
| 算术 | `+` `-` `*` `/` `%` |
| 移位 | `<<` `>>` |
| 比较 | `==` `!=` `<` `<=` `>` `>=` |
| 位运算 | `&` `^` `\|` |
| 逻辑 | `&&` `||` `!` |
| 一元 | `-` `!` |

`+` 任一侧为 string 时执行拼接（另一个值自动转为文本）。位运算只接受整数：浮点不提升（直接报错），移位量在 0..64 之外报错。
优先级从高到低：一元 → 乘除取模 → 加减 → 移位 → 比较 → `&` → `^` → `\|` → `&&` → `||`。

## 事件处理器

### on start

测量启动时执行一次。

```c
on start {
    print("ready");
}
```

### on message \<id\>

收到指定 ID 的帧时触发。id ≤ 0x7FF 只匹配标准帧；更大的 id 匹配 29 位扩展帧（与 `send` 的判断规则一致，同值的标准/扩展帧互不串扰）。

```c
on message 0x100 {
    print("got", frame_byte(0), frame_dlc());
}

on message 0x1C3D1E5 {
    // 扩展帧
}
```

### on message \*

收到本信道**任何数据帧**时触发（错误帧只走 `on errorFrame`），配合 `frame_byte` / `frame_id` 可写网关、记录器、协议嗅探节点。

节点也会收到自己 `send` 出去的帧——在通配处理器里无条件发帧会自激成环。

```c
on message * {
    if (frame_id() != 0x700) {   // 排除自己的转发帧
        print("seen 0x", frame_id());
    }
}
```

### on extended message \<id\>

显式匹配**扩展帧**：数值 ≤ 0x7FF 的扩展帧只有这种写法能寻址，普通 `on message 0x50` 只匹配标准帧。配套发送用 `send_ext`。

```c
on extended message 0x50 {
    send_ext(0x51, frame_byte(0));   // 应答也走扩展帧
}
```

### on fr slot \<槽号\> / on fr \<路\> slot \<槽号\>

FlexRay 帧到达时触发。**寻址用槽而不是帧名**：一个静态槽按周期相位可以轮流排好几帧，"到达"这件事发生在槽上，所以 `on fr slot 13` 会为该槽实际到达的**每一帧**跑一次，并按到达那一帧自己的布局解码。

路号可选——槽号在两路 cluster 上会重复，所以 `on fr 0 slot 13` 只看 FR0，`on fr slot 13` 看所有已配置的路。节点本身绑在一条 CAN 通道上，而 FlexRay 帧没有 CAN 通道可比，所以**路号是唯一的过滤条件**。

处理器里的帧上下文就是这次到达：

| 内建 | 在 `on fr slot` 里的含义 |
|------|--------------------------|
| `frame_id()` | 槽号（**不是** CAN id） |
| `frame_dlc()` | 载荷字节数 |
| `frame_byte(i)` | 载荷第 i 字节 |
| `fr_cycle()` | 该帧到达时的通信周期号（0..63） |

```c
on fr 0 slot 13 {
    let speed = fr_sig(0, 13, "CarSpeed");   // 刚到达那一帧的值
    print("slot", frame_id(), "cycle", fr_cycle(), "speed", speed);
    send(0x320, speed * 0.075);              // 应答走节点的 CAN 通道
}
```

这种处理器能"听 FlexRay、答 CAN"（网关形状），也能用 `fr_send` / `set_fr_sig` 直接动 FlexRay。

`send(frame_id())` 在这种处理器里是**编译错误**——槽号和 CAN id 是两套编号，把槽号发上 CAN 是最容易犯的错，静态检查直接拦下。

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

### on timer "\<名称\>"

命名一次性定时器：平时不运行，被 `set_timer` 武装后延迟触发**一次**。适合"收到某报文后 200ms 再应答"这类延迟反应。

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

发送帧。id ≤ 0x7FF 为标准帧，> 0x7FF 为扩展帧；需要数值小的扩展帧用 `send_ext`（参数相同，永远按扩展帧发送）。
载荷为 0..255 的整数字节或一个 bytes 缓冲；浮点值自动向零截断，所以波形内建可以直接喂进载荷。缓冲超过 8 字节自动按 CAN FD 发送（长度取整到合法 FD 长度，上限 64）。

```c
send(0x123, 0x01, 0x02);
send(0x200, buf);
```

**id 必须可静态推导**（编译期规则）：字面量（`0x123`）、字面量的常量算术（`0x100 + 0x20`）、或 `frame_id()`。写变量 id 会**编译报错**——工具需要静态知道"这个节点可能发哪些报文"，才能与 DBC 声明对账、进 CI 门禁。动态转发场景写成 `on message *` + `send(frame_id(), ...)`。

### sig(id, "Name")

读取信号 `Name` 在报文 `id` 上的最新物理值。信号尚未出现时产生运行时错误，不会静默给 0。

```c
let rpm = sig(0x100, "EngineSpeed");
```

### fr_sig(cluster, slot, "Name")

FlexRay 侧的同一种读数：该路（Buses 表里行末的 `#n`）上，槽 `slot` 最近一帧解出的信号 `Name` 物理值。信号尚未出现时同样是运行时错误。

```c
let speed = fr_sig(0, 13, "CarSpeed");
emit_value("SpeedKmh", speed * 0.075);
```

用槽而不是帧名寻址，是因为槽才是"到达"与"调度"共同说得清的东西；读到的值属于**实际到达的那一帧**。集群编号必须显式写：两路 cluster 上的同号槽是两个不同的读数，而脚本节点属于某条 CAN 通道，没有"本节点所在的那一路"可推断。

### fr_send(cluster, slot, b0, ...) / fr_send(cluster, slot, buffer)

为该路的这个槽发**一帧**：`(路, 槽)` 之后给整数字节（最多 9 个）或一个字节缓冲（最长 254 字节）。

- 这一帧与生成器条目走同一个入口，所以 Trace、Messages、统计、负载与规格、触发器、`on fr slot` 处理器、录制文件全都看得见它。
- **目前不出线缆**：FlexRay 端口只收，所以这是本会话的流量，不是总线上的帧。
- 载荷宽度归集群描述：那一槽声明几个字节就发几个字节，给短了补 0；**给得比声明长则整帧丢弃**并在节点日志写原因（静默截断会发出谁都没要过的帧）。这一路没描述、或描述没在这个槽排帧，就照脚本给的长度发。
- `(路, 槽)` 里的**路**必须是已配置的路（加载了集群描述或挂了监听）。写一个不存在的路号，整帧丢弃并在日志里写原因。槽没配置不受此限：路在，就照打的长度发。
- 这一帧落在该步的生成块里：FlexRay 到达唤醒的处理器在同一步就能看到它，CAN 帧或定时器唤醒的要等下一步。每一步最多一轮，所以"收到自己发的帧就再发一次"的脚本每步只多一帧。

```c
on fr slot 13 {
    // 收到 FR0 槽 13 就替槽 20 发一帧：载荷两个字节，其余按声明补 0
    fr_send(0, 20, frame_byte(0), frame_byte(1));
}
```

### set_fr_sig(cluster, slot, "Name", value)

把物理值写进**这一路这个槽的生成器条目**的基载荷，编码由该路的集群描述完成——就是 Network 里节点面板上改一个信号值所做的同一件事，从脚本做。它改的是"这个槽从现在起带什么"，不是立刻发一帧：条目关着就仍然什么都不发。

需要那一槽已经有发送条目（条目归描述声明的发送节点）。没有条目、这一路没有描述、或这一帧不声明这个名字的信号，写入**整条丢弃**并在日志里写原因，不会留下半个值。

```c
on timer 1000 {
    set_fr_sig(0, 13, "CarSpeed", ramp(0, 240, 4_000_000));
}
```

### set_sig(buffer, id, "Name", value)

将物理值编码到字节缓冲中。buffer 不足 8 字节自动补齐。

```c
let buf = bytes(8);
set_sig(buf, 0x200, "RPM", 3000);
send(0x200, buf);
```

### get_sig(buffer, id, "Name")

从字节缓冲中按数据库定义解码信号物理值。

```c
let v = get_sig(buf, 0x200, "RPM");
```

### emit_value("Name", 表达式)

发布**派生信号**：名字 + 本节点算出的一个值。表达式的求值就是派生逻辑本身——`sig()` 读数、数学与波形内建、任意脚本逻辑都能参与。派生信号可在 Graphics / Data 的信号选择树"派生信号"分组里勾选，像数据库信号一样画曲线、看数值。空名字或非数值是运行时错误。整个会话最多 256 个不同名字，超出后新名字的发布被忽略。

```c
on timer 10 {
    emit_value("SpeedKmh", sig(0x100, "EngineSpeed") * 0.075);
}
```

### sys_get("ns::name") / sys_set("ns::name", value)

读写**系统变量**——在 View > System Variables 里定义的命名空间值（namespace + name + 初值 + 可选界限 + 单位/备注，定义随工程保存）。每次测量开始所有变量复位为初值；每个变量同时发布为一条可观测流，Data / Graphics 按 namespace 分组选用。写入被夹到定义的界限内；引用未定义的变量在节点启动时报出来，运行中写未定义变量被丢弃并记日志（节点不停跑）。

```c
on timer 50 {
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

运算符 `&` `|` `^` `<<` `>>` 之外的等价写法：

| 内建 | 说明 |
|------|------|
| `bit_and(a, b)` | 按位与 |
| `bit_or(a, b)` | 按位或 |
| `bit_xor(a, b)` | 按位异或 |
| `bit_not(a)` | 按位取反 |
| `bit_shl(a, n)` / `bit_shr(a, n)` | 左移/右移 |

## 信号与系统变量的速记

读信号、读系统变量除函数形式外还有两种等价速记，名字都进静态检查（拼错是显式失败）：

```c
let rpm = $EngineStatus::RPM;                 // 等价 sig(id, "RPM")，id 由库里的报文名解析
let target = @sysvar::Demo::Setpoint;         // 等价 sys_get("Demo::Setpoint")
@sysvar::Demo::Setpoint = target * 0.9;       // 等价 sys_set(..)，写入仍受界限钳位
```

信号**没有**赋值速记：写信号走 `set_sig(buffer, id, "Name", value)` 的缓冲语义，`$…` 只读。启动检查（编辑器收发 tab、节点日志、`--check-script`）会核对这两种速记的名字。

## 波形内建

周期波形与发送发生器共用同一套求值器，同参数下逐采样一致。均以总线时钟为时基；`hi < lo` 时 ramp / triangle / counter 反向。

| 内建 | 说明 |
|------|------|
| `ramp(lo, hi, period_s)` | 锯齿波，从 lo 线性升到 hi，周期 period_s 秒 |
| `triangle(lo, hi, period_s)` | 三角波，半周期处到达峰值 hi |
| `square(lo, hi, period_s)` | 方波，前半周期 lo，后半周期 hi |
| `counter(lo, hi, period_s)` | 滚动计数器，从 lo 到 hi 整步递增 |
| `sine_wave(offset, amplitude, period_s)` | 正弦波，中心 offset，振幅 amplitude，起点在中心 |

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

外部仿真元件（电池模型、诊断栈、被控对象仿真等）在进程内注册函数，任何脚本都可以直接调用，与内建写法相同：

```c
let soc = battery_soc();          // 外部注册的无参函数
send(0x321, pack_soc(soc));       // 与内建混用
```

- 注册方为 Rust 侧代码：`roxy_can::script::register_extern("名称", 函数)`，与内建同名注册会被拒绝
- 未注册的函数名可以编译，调用时报运行时错误，便于分阶段接线
- 每个节点还可以挂私有钩子（如 `set_sig` / `get_sig` 走信道数据库），解析顺序：注册表 → 节点钩子

## 编译错误 vs 运行时错误

- 编译错误在 Apply 时报出，格式 `line N: 消息` 或 `line N:M: 消息`（N:M 为行:列）
- 运行时错误终止当前回调，格式 `line N: 消息`，节点进入错误状态
- 节点错误状态通过重新 Apply 或重启测量清除
