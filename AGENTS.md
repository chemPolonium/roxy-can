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

同理，遇到"内容被裁掉/放不下"的问题，先找控件或列自己的 flag，再考虑手推几何（例：表格单元格跨列用 `TableColumnFlags::NO_CLIP`，手推裁剪矩形会吃掉冻结表头的列）。
