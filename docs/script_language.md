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

on timer 100 {
    let buf = bytes(8);
    buf[0] = 0xAB;
    set_sig(buf, 0x200, "RPM", base + random(0, 50));
    send(0x200, buf);
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
| nil | — | 空值（未初始化变量的默认值） |

整数运算精确（溢出报错），混合运算提升为浮点。`+` 任一侧为 string
时按 print 格式拼接。

## 变量

```c
let x = 10;          // 全局变量（顶层 let）
x = x + 1;           // 赋值

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

## 控制流

```c
if (condition) { ... } else { ... }

while (condition) { ... }

for (let i = 0; i < 10; i = i + 1) { ... }
```

条件必须是 bool。短路求值：`&&` 和 `||`。

`break` 跳出最内层循环；`continue` 跳过本轮循环体剩余部分：
在 `while` 中回到条件判断，在 `for` 中先执行步进语句再判断条件。
两者只能出现在循环体内，否则编译报错。

## 运算符

| 类别 | 运算符 |
|------|--------|
| 算术 | `+` `-` `*` `/` `%` |
| 比较 | `==` `!=` `<` `<=` `>` `>=` |
| 逻辑 | `&&` `||` `!` |
| 一元 | `-` `!` |

`+` 任一侧为 string 时执行拼接（另一个值自动转为文本）。

## 事件处理器

### on start

测量启动时执行一次。

```c
on start {
    print("ready");
}
```

### on message \<id\>

收到指定 ID 的帧时触发。

```c
on message 0x100 {
    // frame_byte(n): 触发帧的第 n 字节
    // frame_dlc():  触发帧的数据长度
    print("got", frame_byte(0), frame_dlc());
}
```

### on timer \<ms\>

周期定时器，每 N 毫秒触发一次。

```c
on timer 100 {
    // 每 100ms 执行
}
```

### 定时器控制内建

| 内建 | 说明 |
|------|------|
| `set_period(ms)` | 修改当前定时器的周期 |
| `stop_timer()` | 停止当前定时器 |

## 总线内建

### send(id, byte0, byte1, ...) / send(id, buffer)

发送经典帧。id ≤ 0x7FF 为标准帧，> 0x7FF 为扩展帧。
载荷为 0..255 的整数字节或一个 bytes 缓冲。

```c
send(0x123, 0x01, 0x02);
send(0x200, buf);
```

### sig(id, "Name")

读取信号 `Name` 在报文 `id` 上的最新物理值。信号尚未出现时产生运行时错误。

```c
let rpm = sig(0x100, "EngineSpeed");
```

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

## 帧数据访问内建

| 内建 | 说明 |
|------|------|
| `frame_byte(n)` | 触发帧的第 n 字节（越界报错） |
| `frame_dlc()` | 触发帧的数据长度 |

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

| 内建 | 说明 |
|------|------|
| `bit_and(a, b)` | 按位与 |
| `bit_or(a, b)` | 按位或 |
| `bit_xor(a, b)` | 按位异或 |
| `bit_not(a)` | 按位取反 |
| `bit_shl(a, n)` / `bit_shr(a, n)` | 左移/右移 |

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
| `now()` | 总线时钟（秒，浮点） |
| `bytes(n)` | 分配 n 字节缓冲（零填充） |
| `len(v)` | 缓冲长度或字符串字符数 |

## 编译错误 vs 运行时错误

- 编译错误在 Apply 时报出，格式 `line N: 消息`
- 运行时错误终止当前回调，格式 `line N: 消息`，节点进入错误状态
- 节点错误状态通过重新 Apply 或重启测量清除
