//! 设置窗口：把 `ComTool` 上的用户偏好接到真正的控件上。
//!
//! 旧版这里是「只有一个语言占位下拉」的空壳；现在每一项都绑到实际生效的字段。
//! 生效范围分三类，界面上如实标注：
//!
//! - **立即生效**：主题、日志长度上限、自动滚动、重绘间隔、发送后清空；
//! - **接线待做**：默认日志文件名（P4 实时文件日志使用）；
//! - **接线待做**：启动时检查更新（P6 版本检查使用，默认关闭）。

use crate::app::{ComTool, Language};
use eframe::egui;
use egui::Ui;

use super::menu_bar::theme_radios;

/// 左侧标签列的宽度，让右侧控件纵向对齐。
const LABEL_WIDTH: f32 = 104.0;

impl ComTool {
    /// 绘制设置窗口（`show_settings` 为 false 时直接返回）。
    pub(crate) fn show_settings_window(&mut self, ui: &mut Ui) {
        if !self.show_settings {
            return;
        }

        // 用局部变量接收窗口开关再回写：与 sidebar.rs 同一套写法，避免在闭包里
        // 同时可变借用 self 与 self.show_settings。
        let mut open = self.show_settings;
        egui::Window::new("设置")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(20.0, 20.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(400.0);

                ui.heading("界面");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("主题"));
                    theme_radios(ui, &mut self.theme);
                });
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("语言"));
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(match self.language {
                            Language::Zh => "中文",
                            Language::En => "English",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.language, Language::Zh, "中文");
                            ui.selectable_value(&mut self.language, Language::En, "English");
                        });
                });
                ui.weak("语言选项尚未接入界面文案，仅作占位");

                ui.separator();
                ui.heading("接收");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("日志长度上限"));
                    ui.add(
                        egui::DragValue::new(&mut self.max_log_chars)
                            .range(0..=1_000_000)
                            .speed(200.0),
                    );
                    ui.label("字符（0 = 不限）");
                });
                ui.weak("超出上限时从最旧的内容开始淘汰，三种接收视图同步裁剪");
                ui.checkbox(&mut self.auto_scroll, "自动滚动到最新");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("重绘间隔"));
                    ui.add(
                        egui::DragValue::new(&mut self.repaint_interval_ms)
                            .range(16..=1000)
                            .speed(1.0),
                    );
                    ui.label("ms（16~1000，越小越跟手也越耗电）");
                });

                ui.separator();
                ui.heading("发送");
                ui.checkbox(&mut self.clear_send_after_send, "发送后清空输入框");

                ui.separator();
                ui.heading("文件日志");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("默认文件名"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.default_log_file_name)
                            .desired_width(240.0),
                    );
                });
                ui.weak("实时文件日志将在 P4 接线；名称支持 %Y-%m-%d_%H-%M-%S 时间格式");

                ui.separator();
                ui.heading("更新");
                ui.checkbox(&mut self.check_update_on_start, "启动时检查更新");
                ui.weak("版本检查将在 P6 接线；离线或内网环境建议保持关闭");

                ui.separator();
                ui.weak("以上均为用户偏好，改动即时生效并在退出时自动保存");
            });
        self.show_settings = open;
    }
}
