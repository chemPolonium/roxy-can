use crate::app::App;
use dear_imgui_rs::{StyleVar, TableColumnSetup, Ui};

const SHORTCUTS: [(&str, &str); 11] = [
    ("F9", "启动 / 停止测量"),
    ("Space", "播放 / 暂停"),
    ("- / +", "回放减速 / 加速一档"),
    ("Home", "图形窗口回到实时边缘"),
    ("Ctrl+N", "新建工程"),
    ("Ctrl+O", "打开 DBC"),
    ("Ctrl+Shift+O", "打开工程"),
    ("Ctrl+R", "切换日志录制 (ASC)"),
    ("Ctrl+E", "导出 Trace 为 ASC"),
    ("Ctrl+S", "保存工程"),
    ("Ctrl+Shift+S", "工程另存为"),
];

pub(crate) fn popup_is_open(ui: &Ui, id: &str) -> bool {
    ui.is_popup_open(id)
}

pub fn render(app: &mut App, ui: &Ui) {
    if app.show_shortcuts {
        const ID: &str = "Shortcuts##help_shortcuts";
        if !popup_is_open(ui, ID) {
            ui.open_popup(ID);
        }
        let mut open = true;
        let min = ui.push_style_var(StyleVar::WindowMinSize([380.0, 0.0]));
        ui.modal_popup_with_opened(ID, &mut open, || {
            ui.table("##shortcut_cols")
                .add_column(TableColumnSetup::new("键"))
                .add_column(TableColumnSetup::new("功能"))
                .build(|ui| {
                    for (key, desc) in SHORTCUTS {
                        ui.table_next_row();
                        ui.table_next_column();
                        ui.text(key);
                        ui.table_next_column();
                        ui.text(desc);
                    }
                });
        });
        min.pop();
        app.show_shortcuts = open;
    }
    if app.show_about {
        const ID: &str = "About##help_about";
        if !popup_is_open(ui, ID) {
            ui.open_popup(ID);
        }
        let mut open = true;
        let mut close = false;
        ui.modal_popup_with_opened(ID, &mut open, || {
            ui.text(format!("roxy-can {}", env!("CARGO_PKG_VERSION")));
            ui.text("CAN 总线仿真与分析工具");
            ui.text("虚拟仿真与 ASC/BLF 回放 · DBC 解码 · 多窗口观测 · 多桌面");
            ui.separator();
            if ui.button("Close") {
                ui.close_current_popup();
                close = true;
            }
        });
        app.show_about = open && !close;
    }
}
