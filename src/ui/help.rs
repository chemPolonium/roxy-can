use crate::app::App;
use imgui::Ui;

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

pub(crate) fn popup_is_open(_ui: &Ui, id: &str) -> bool {
    let cstr = std::ffi::CString::new(id).unwrap();
    unsafe { imgui::sys::igIsPopupOpen_Str(cstr.as_ptr(), imgui::sys::ImGuiPopupFlags_None as i32) }
}

pub fn render(app: &mut App, ui: &Ui) {
    if app.show_shortcuts {
        const ID: &str = "Shortcuts##help_shortcuts";
        if !popup_is_open(ui, ID) {
            ui.open_popup(ID);
        }
        let mut open = true;
        let min = ui.push_style_var(imgui::StyleVar::WindowMinSize([380.0, 0.0]));
        ui.modal_popup_config(ID).opened(&mut open).build(|| {
            ui.columns(2, "##shortcut_cols", false);
            for (key, desc) in SHORTCUTS {
                ui.text(key);
                ui.next_column();
                ui.text(desc);
                ui.next_column();
            }
            ui.columns(1, "##shortcut_cols_end", false);
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
        ui.modal_popup_config(ID).opened(&mut open).build(|| {
            ui.text(format!("roxy-can {}", env!("CARGO_PKG_VERSION")));
            ui.text("类 CANoe 的 CAN 总线分析工具");
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
