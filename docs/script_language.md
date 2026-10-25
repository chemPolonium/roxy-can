# 节点脚本语言参考手册

类 CAPL 的总线仿真脚本语言，编译后由内置虚拟机执行。每个回调限 10 万条指令、256 层递归，超限即停止运行，有缺陷的脚本不会卡住总线。

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

on message EngineStatus {            // 按 DBC 报文名
    print(EngineStatus.RPM);
}

on extended message 0x50 { }    // 显式扩展帧（数值可 ≤ 0x7FF）

on message * { }        // 任意帧（嗅探/网关）

on fr slot 13 { }       // FlexRay 槽 13 有帧到达（所有路）

on fr 0 slot 13 { }     // 只有 FR0 的槽 13

on errorFrame { }       // 错误帧

on timer 100 { }        // 周期定时器（也可写 100ms / 1s）

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

### `.`、`:` 与 `::` 的分工

Rust 用 `::` 走类型/模块路径、用 `.` 访问值成员，这是两种不同性质的名字。本语言没有用户可定义的对象/结构体，所以只保留一种读取路径，但把边界写死：

- `.` 是**读取路径**：左边要么是保留上下文（`this`、`node`），要么是数据库/总线名（DBC 报文、`FR0`、`FR.PowerTrain`、`ETH0`、`ETH.Body`）；右边是信号、字段或方法式读取。`Message["Signal Name"]` 用下标处理非标识符名字。
- `:` 只用于 `switch` 的 `case` / `default` 标签，不参与名字解析（Rust 的 `:` 是类型/字段标注，这里没有类型标注语法）。
- `::` 只出现在系统变量键的字符串里（`"ns::name"`），**不是**语言运算符；写成 `Message::Signal` 会编译报错。
- `this`、`node`、`FR`、`ETH`、`FR<n>`、`ETH<n>` 是保留名，不能声明为变量、函数或参数——否则 `FR0.Signal` 这类读取的归属就无法从源码判断。
- `.` 左边永远按数据库名或上述上下文解析；脚本变量没有成员访问，同名变量不会参与点号读取。

因此 `EngineStatus.RPM`（数据库名）、`this.signal("RPM")`（触发帧）和 `node.bus`（脚本自身）共用 `.`，但左手边永远只有一个确定解释；需要显式 id 时用 `sig(id, "Name")`。

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

### on message \<名称\>

按 DBC 报文名注册处理器，启动时用节点所在通道的库解析出 id 和帧类型（标准/扩展）；名字不存在会在启动日志报 `[check]`：

```c
on message EngineStatus {
    print("rpm", EngineStatus.RPM);
}

on message "Engine Cmd" {   // 名字含空格时用字符串
}
```

- 重载 DBC 后名称会自动跟随新的 id/帧类型，日志里有一条 `[reload] handler ...`。
- 同一报文名只能有一个处理器；与数字 id 处理器撞上同一个报文会在启动时报重复。
- `frame_id()` 在命名处理器里仍返回解析后的数字 id，可用于原样转发。

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

路号可选——槽号在两个 cluster 之间会重复，所以 `on fr 0 slot 13` 只看 FR0，`on fr slot 13` 看所有已配置的路。节点本身绑在一条 CAN 通道上，而 FlexRay 帧没有 CAN 通道可比，所以**路号是唯一的过滤条件**。

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

### on timer \<period\>

周期定时器。周期可以写裸数字（毫秒），也可以带 `ms` / `s` 单位：

```c
on timer 100 { }       // 100 ms
on timer 250ms { }     // 同样 250 ms
on timer 1s { }        // 1000 ms
```

带单位的周期必须是整毫秒；`1.5s` 可以，`0.5ms` 会报错。

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
| `set_period(100ms)` | 修改当前定时器的周期（一次性定时器中等价于重新武装）；也接受裸毫秒数 |
| `stop_timer()` | 停止当前定时器 |
| `set_timer("名称", 200ms)` | 武装命名一次性定时器，延迟后触发一次；也接受裸毫秒数 |
| `cancel_timer("名称")` | 取消已武装的一次性定时器 |

`set_timer` 指向不存在的处理器时记一条日志告警，不影响运行。

## 触发帧上下文：this / node

在帧处理器里，`this` 指向**这次触发的那一帧**，不用再记住 id、槽号或总线号：

| 写法 | 含义 |
|------|------|
| `this.bus` | 触发帧所在的总线：CAN 是通道号，FlexRay 是 cluster 号 |
| `this.id` | CAN 帧 id（在 FlexRay 处理器里使用会报错） |
| `this.slot` | FlexRay 槽号（在 CAN 处理器里使用会报错） |
| `this.cycle` | FlexRay 通信周期号（在 CAN 处理器里使用会报错） |
| `this.len` | 触发帧的载荷字节数 |
| `this[i]` | 触发帧载荷的第 i 字节 |
| `this.signal("Name")` / `this["Name"]` | 触发帧自己的信号物理值，按这一帧的布局解码 |

`node.bus` 是**脚本节点自己**的 CAN 通道号，与触发帧无关，因此在 `on start` 和定时器里也能读。绑定脚本的总线是它绑定的 DBC 节点所在通道；未绑定脚本是创建时选的通道。

```c
on message * {
    // 原样转发，不需要先知道 id
    send(frame_id(), this[0]);
}

on fr slot 13 {
    // 听 FlexRay、答到节点自己的 CAN 通道
    send(0x320, this.signal("CarSpeed") * 0.075, this.cycle);
}

on timer 100 {
    // 定时器没有触发帧：this.* 会报错，node.bus 仍然可用
    print("node bus", node.bus);
}
```

`this.signal` 读的是触发帧自己的解码表：CAN 用 `(id, name)`，FlexRay 用 `(cluster, slot, name)`，所以同一槽按周期轮流承载不同帧时，读到的仍是实际到达那一帧。信号不在这一帧里是运行时错误，不会静默给 0。`this` 和 `node` 是保留前缀。

原有的 `frame_id()` / `frame_byte()` / `fr_cycle()` 等函数保留不变，`this` 只是更清楚的写法。

## 总线内建

### send(id, byte0, byte1, ...) / send(id, buffer)

发送帧。id ≤ 0x7FF 为标准帧，> 0x7FF 为扩展帧；需要数值小的扩展帧用 `send_ext`（参数相同，永远按扩展帧发送）。
载荷为 0..255 的整数字节或一个 bytes 缓冲；浮点值自动向零截断，所以波形内建可以直接写入载荷。缓冲超过 8 字节自动按 CAN FD 发送（长度取整到合法 FD 长度，上限 64）。

```c
send(0x123, 0x01, 0x02);
send(0x200, buf);
```

节点发出的帧会在同一个 step 内继续分发；如果一个处理器发送的就是它收到的帧，会形成自激循环。每一步的反应帧上限为 1024，超出后丢弃并在 Write 记一条警告，不会卡住 core。

**id 必须可静态推导**（编译期规则）：字面量（`0x123`）、字面量的常量算术（`0x100 + 0x20`）、或 `frame_id()`。写变量 id 会**编译报错**——工具需要静态知道"这个节点可能发哪些报文"，才能与 DBC 声明对账、进 CI 门禁。动态转发场景写成 `on message *` + `send(frame_id(), ...)`。

### sig(id, "Name")

读取信号 `Name` 在报文 `id` 上的最新物理值。信号尚未出现时产生运行时错误，不会静默给 0。

```c
let rpm = sig(0x100, "EngineSpeed");
```

### Message.Signal / Message["Signal Name"]

按 DBC 报文名和信号名读取最新物理值，不需要记 id：

```c
let rpm = EngineStatus.EngineSpeed;
let temp = EngineStatus["Engine Temp"];   // 名字不是标识符时用下标
```

- 节点启动时用它所在通道的 DBC 把 `Message` 解析成 id；报文名或信号名拼错会在启动日志里以 `[check]` 报出，不会静默读到 0。
- 只读。写信号仍走 `set_sig(buffer, id, "Name", value)`，因为写入需要先有一帧的原始字节。
- 点号要求报文名、信号名都是标识符（字母/数字/下划线）；下标里的字符串可以是任意 DBC 名字。
- `FR` / `ETH` 开头、后接数字或点号的形式（如 `FR2`、`ETH0`、`FR.PowerTrain`）是保留的总线选择器，不能作为报文名或变量名。

### FRn.Signal / FRn["Signal Name"]

FlexRay 的按名读取，`n` 是 Buses 表格行末的 `#n`（cluster 索引）：

```c
on fr slot 13 {
    let speed = FR0.CarSpeed;
    let torque = FR1["Drive Torque"];
}
```

- 启动时在该路的集群描述里把信号名解析成**唯一槽号**；名字不存在、或这一路没有描述，会在日志里报 `[check]`。
- 同一个信号名声明在**多个不同槽**时无法判断该读哪一个，会报歧义；改用落槽号的 `fr_sig(bus, slot, "Name")`。同一槽被多帧按周期相位轮流占用不算歧义——它们读的是同一个线上槽。
- 换集群描述（含测量中重载）会就地重解析，日志写 `[reload] FR0.CarSpeed slot 13 -> 20`。
- 读到的值属于该槽**实际到达的那一帧**；信号尚未出现仍是运行时错误。
- 不想记编号时，可以用集群描述里的名字选中该路：`FR.PowerTrain.CarSpeed`，非标识符名字用 `FR["Power Train"]["Car Speed"]`。名字在节点启动时解析；找不到或同名多路会报 `[check]`，此时回退编号写法。`fr_sig_bus("PowerTrain", "CarSpeed")` 是等价函数形式。

### ETHn.Frame.Signal / ETHn["Frame"]["Signal Name"]

Ethernet 的按名读取：`n` 是 Ethernet 路号，之后是 ARXML 的帧名与信号名：

```c
on message 0x100 {
    let speed = ETH0.SpeedFrame.Speed;
    let temp = ETH1["Body"]["Cabin Temp"];   // 名字不是标识符时用下标
}
```

- 启动时用该路的 ARXML 核对帧名和信号名；帧不存在或该帧不声明这个信号，都会报 `[check]`。
- 值来自与曲线 / Data 相同的按流解码表，只对 ARXML 能**唯一匹配**的流有效；没匹配上的流读值是运行时错误，不会猜成某个帧。
- 目前只读：Ethernet 发送仍走生成器条目。
- 命名形式：`ETH.Body.FrameA.Speed` / `ETH["Body"]["Frame A"]["Speed X"]`，`Body` 是 ARXML 的 cluster 名；函数形式 `eth_sig_bus("Body", "FrameA", "Speed")`。名字找不到或多路同名同样报 `[check]`，编号形式始终可用。

### fr_sig(cluster, slot, "Name")

FlexRay 侧的同一种读数：该路（Buses 表里行末的 `#n`）上，槽 `slot` 最近一帧解出的信号 `Name` 物理值。信号尚未出现时同样是运行时错误。

```c
let speed = fr_sig(0, 13, "CarSpeed");
emit_value("SpeedKmh", speed * 0.075);
```

用槽而不是帧名寻址，是因为槽才是"到达"与"调度"共同说得清的东西；读到的值属于**实际到达的那一帧**。集群编号必须显式写：两个 cluster 上的同号槽是两个不同的读数，而脚本节点属于某条 CAN 通道，没有"本节点所在的那一路"可推断。

### derived("Name")

读取 core 采样的**派生通道**（Signals 页 `Derived…` 创建、随工程保存的数学通道）的最新值，按定义名寻址。派生通道没有 CAN id 或总线编号，所以不复用 `sig()`，也不存在"本节点所在的那一路"的歧义。

```c
let error = derived("Speed error");
emit_value("ErrorAlarm", error > 5);
```

派生通道在每一步结尾采样，因此脚本在这一步读到的是**上一步结束时的值**。尚未采到第一点（`on start`、首个帧处理器，或名字还没有定义）时读 **0**，与 `sys_get` 读取未定义变量一致——脚本不会因此停下。条件没有采样（源信号未选中或历史为空）时，触发器的 `+ Derived` 规则保持原电平。

### fr_send(cluster, slot, b0, ...) / fr_send(cluster, slot, buffer)

为该路的这个槽发**一帧**：`(路, 槽)` 之后给整数字节（最多 9 个）或一个字节缓冲（最长 254 字节）。

- 这一帧与生成器条目走同一个入口，所以 Trace、Messages、统计、负载与规格、触发器、`on fr slot` 处理器、录制文件全都看得见它。
- **目前不发到总线上**：FlexRay 端口仅接收，所以这是本会话的流量，不是总线上的帧。
- 载荷宽度归集群描述：那一槽声明几个字节就发几个字节，给短了补 0；**给得比声明长则整帧丢弃**并在节点日志写原因（静默截断会发出谁都没要过的帧）。这一路没描述、或描述没在这个槽排帧，就照脚本给的长度发。
- `(路, 槽)` 里的**路**必须是已配置的路（加载了集群描述或已连接监听）。写一个不存在的路号，整帧丢弃并在日志里写原因。槽没配置不受此限：这一路存在，就按输入的长度发送。
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

读写**系统变量**——在 View > System Variables 里定义的命名空间值（namespace + name + 初值 + 可选界限 + 单位/备注，定义随工程保存）。每次测量开始所有变量复位为初值；每个变量同时发布为一条可观测流，Data / Graphics 按 namespace 分组选用。写入被夹到定义的界限内；引用未定义的变量在节点启动时报出来，运行中写未定义变量被丢弃并记日志（节点继续运行）。

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

## 信号与系统变量的访问

信号按“报文.信号”读取，名字进静态检查（拼错是显式失败）：

```c
let rpm = EngineStatus.RPM;                   // 报文名由所在通道的 DBC 解析
let temp = EngineStatus["Engine Temp"];       // 非标识符名字
let target = sys_get("Demo::Setpoint");       // 系统变量仍用函数
sys_set("Demo::Setpoint", target * 0.9);      // 写入受定义的界限钳位
```

信号**没有**赋值速记：写信号走 `set_sig(buffer, id, "Name", value)` 的缓冲语义（FlexRay 用 `set_fr_sig`），`Message.Signal`、`FRn.Signal`、`FR.Cluster.Signal`、`ETHn.Frame.Signal`、`ETH.Cluster.Frame.Signal` 及下标形式都只读。启动检查（编辑器收发 tab、节点日志、`--check-script`）会核对 CAN 报文名/信号名；FlexRay 与 Ethernet 的名字在节点启动时核对对应描述。

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

## 编译错误 vs 运行时错误

- 编译错误在 Apply 时报出，格式 `line N: 消息` 或 `line N:M: 消息`（N:M 为行:列）
- 运行时错误终止当前回调，格式 `line N: 消息`，节点进入错误状态
- 节点错误状态通过重新 Apply 或重启测量清除
