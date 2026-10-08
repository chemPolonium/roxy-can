# 给 agent 的守则

## Dear ImGui：先查绑定，别自己造

依赖是 `dear-imgui-rs 0.18`（绑定 Dear ImGui 1.92，docking 常开）。要任何 ImGui 行为，**先在 crate 文档里搜一遍再动手**：

- 文档首页：<https://docs.rs/dear-imgui-rs/latest/dear_imgui_rs/>
- 直接搜：`https://docs.rs/dear-imgui-rs/latest/dear_imgui_rs/?search=text_wrapped`
  —— `?search=<关键词>` 在该文档任意页面都有效，填英文动词/名词即可（`text`、`table`、`combo`、`drag`、`style`）。
- 版本以 `Cargo.toml` 为准；docs.rs 页面左下角可切版本。

同一个角落的两个例子，都是"绑定里已经有"：

- 日志文本折行 = `ui.text_wrapped(text)`（内部就是 `PushTextWrapPos(0.0)`，按窗口 work rect 右缘折，续行对齐到正文起点）。自己拿 `cursor_screen_pos + content_region_avail` 算 wrap 位置是**错的**：那个参数要的是窗口内坐标，塞屏幕坐标会把一行中段画到窗外，看着像被截断，尾巴却跑到第二行。
- 带颜色的折行文本：`push_style_color(StyleColor::Text, c)` 套住 `ui.text_wrapped(..)`——没有 `text_wrapped_colored` 这种函数。

同理，遇到"内容被裁掉/放不下"的问题，先找控件或列自己的 flag，再考虑手推几何。**但 flag 的作用要读源码确认**，别信名字：表格单元格的文字**不能**靠列级 `TableColumnFlags::NO_CLIP` 跨列——`TableBeginCell` 只在**表级** `TableFlags::NO_CLIP` 时才跳过"把本列矩形设为裁剪矩形"，列级那个 flag 只影响绘制通道的合并（且要求该列总共只有一条绘制命令）。表级 NoClip 在这张表里也不能用：它冻结了列，整表关掉裁剪会让后面的列整片消失。ImGui 没有 colspan——**长内容放进足够宽的列**（一般是最后一列 stretch 列），而不是跟裁剪较劲。

## 控件 ID 不要取自显示文本

表格**不给单元格另加 ID 种子**（`TableSetColumnIndex` 只挪光标和裁剪矩形），所以循环里的控件拿显示文本当 ID 迟早撞车，ImGui 会弹 `2 visible items with conflicting ID` 盖住整个窗口。文本重复是常态，不是边角情况：同一份 DBC 挂在两条总线上，Messages 就有两行 `100  EngineStatus`；两条总线或两路 FlexRay 可以改成同名；`on` / `off` 这种状态词两行之间必然一样。

写法：`let _id = ui.push_id(行的身份);` 套住整行——身份用聚合键那种数字（`(通道, id, 帧类)`、`(路, 槽, 占用帧)`），不要用它派生出的名字；或者给 label 加 `##{i}` 后缀。改了 ID 就要在提交信息里说明：ImGui 按 ID 存的展开状态、列宽这些会重置一次。
