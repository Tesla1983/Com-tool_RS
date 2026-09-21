//! 顶部菜单栏：文件（信息 / 退出）· 设置（设置窗口 / 主题）· 视图（面板显隐 / 显示模式）。
//!
//! 菜单栏必须先于其它面板绘制：egui 的面板是"抢占"布局的，先画的先占用整幅宽度，
//! 顶部栏放在最前才能横跨整个窗口。

use crate::app::{ComTool, ThemeChoice};
use eframe::egui;
use egui::Ui;

/// 菜单栏本轮产生的**副作用**请求。
///
/// 只承载"要打开窗口 / 要退出程序"这类动作。勾选框、单选按钮直接绑定 `ComTool`
/// 的字段——它们改的是状态值，不需要经过派发（与 [`crate::sidebar::SidebarAction`]
/// 同一条约定）。
#[derive(Default)]
pub(crate) struct MenuAction {
    /// 用户点了「文件 → 信息」。
    pub info_requested: bool,
    /// 用户点了「设置 → 打开设置窗口」。
    pub settings_requested: bool,
    /// 用户点了「文件 → 开始/停止记录日志」。
    pub log_toggle_requested: bool,
    /// 用户点了「文件 → 退出」。
    pub quit_requested: bool,
}

impl ComTool {
    /// 绘制顶部菜单栏并返回本轮的操作请求。
    pub(crate) fn show_menu_bar(&mut self, ui: &mut Ui) -> MenuAction {
        let mut action = MenuAction::default();

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("文件", |ui| {
                    if ui.button("信息").clicked() {
                        action.info_requested = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("退出").clicked() {
                        action.quit_requested = true;
                        ui.close();
                    }
                });

                ui.menu_button("设置", |ui| {
                    if ui.button("打开设置窗口").clicked() {
                        action.settings_requested = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.label("主题");
                    theme_radios(ui, &mut self.theme);
                });

                ui.menu_button("视图", |ui| {
                    ui.label("面板");
                    self.show_panel_checkboxes(ui);
                    ui.separator();
                    ui.label("显示模式");
                    ui.checkbox(&mut self.color_view, "颜色");
                    ui.checkbox(&mut self.ascii_view, "字节查看");
                    ui.checkbox(&mut self.hex_view, "HEX 接收");
                    ui.separator();
                    ui.checkbox(&mut self.auto_scroll, "自动滚动");
                });
            });
        });

        action
    }

    /// 面板显隐勾选，并保证**至少保留一个可见面板**。
    ///
    /// 规则：当只剩一个面板可见时，禁止把它取消掉（该项置灰）；已经隐藏的面板
    /// 始终允许重新勾上。这样用户永远不可能把界面点成一整片空白。
    fn show_panel_checkboxes(&mut self, ui: &mut Ui) {
        let can_hide = self.visible_panel_count() > 1;
        ui.add_enabled(
            can_hide || !self.sidebar_open,
            egui::Checkbox::new(&mut self.sidebar_open, "侧边栏"),
        );
        ui.add_enabled(
            can_hide || !self.show_receive_panel,
            egui::Checkbox::new(&mut self.show_receive_panel, "接收区"),
        );
        ui.add_enabled(
            can_hide || !self.show_chart_panel,
            egui::Checkbox::new(&mut self.show_chart_panel, "实时图表"),
        );
        ui.add_enabled(
            can_hide || !self.show_send_panel,
            egui::Checkbox::new(&mut self.show_send_panel, "发送区"),
        );
        if !can_hide {
            ui.weak("至少保留一个面板");
        }
    }
}

/// 主题三选一。菜单与设置窗口共用同一套写法，避免两处出现不同的选项集合。
pub(crate) fn theme_radios(ui: &mut Ui, theme: &mut ThemeChoice) {
    ui.radio_value(theme, ThemeChoice::System, "跟随系统");
    ui.radio_value(theme, ThemeChoice::Light, "浅色");
    ui.radio_value(theme, ThemeChoice::Dark, "深色");
}
