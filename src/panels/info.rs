//! 关于窗口：版本、技术栈、存档位置与常用快捷键。
//!
//! 计划里的 P6 会把这里扩展成完整的 Info（含版本检查结果）；当前版本只做
//! 「能一眼看清自己在跑什么」这一件事，且不做任何网络访问。

use crate::APP_TITLE;
use crate::app::ComTool;
use crate::strings::fill;
use eframe::egui;
use egui::Ui;

impl ComTool {
    /// 绘制关于窗口（`show_info` 为 false 时直接返回）。
    pub(crate) fn show_info_window(&mut self, ui: &mut Ui) {
        if !self.show_info {
            return;
        }

        let s = self.language.strings();
        let mut open = self.show_info;
        egui::Window::new(s.info_window_title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(340.0);

                ui.heading(APP_TITLE);
                ui.label(fill(s.version_label, &[&env!("CARGO_PKG_VERSION")]));
                ui.label(s.app_description);

                ui.separator();
                ui.label(s.serial_backend);
                ui.label(s.ui_framework);
                match eframe::storage_dir(APP_TITLE) {
                    Some(dir) => {
                        ui.label(fill(s.settings_dir, &[&dir.display()]));
                    }
                    None => {
                        ui.label(s.settings_dir_unavailable);
                    }
                }

                ui.separator();
                ui.label(s.shortcuts);
                ui.label(s.shortcut_send);
                ui.label(s.shortcut_copy);
            });
        self.show_info = open;
    }
}
