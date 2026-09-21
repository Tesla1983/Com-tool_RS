//! 关于窗口：版本、技术栈、存档位置与常用快捷键。
//!
//! 计划里的 P6 会把这里扩展成完整的 Info（含版本检查结果）；当前版本只做
//! 「能一眼看清自己在跑什么」这一件事，且不做任何网络访问。

use crate::APP_TITLE;
use crate::app::ComTool;
use eframe::egui;
use egui::Ui;

/// 应用简介（`Cargo.toml` 暂无 `description` 字段，先写在这里）。
const APP_DESCRIPTION: &str = "基于 eframe/egui 的串口调试上位机";

impl ComTool {
    /// 绘制关于窗口（`show_info` 为 false 时直接返回）。
    pub(crate) fn show_info_window(&mut self, ui: &mut Ui) {
        if !self.show_info {
            return;
        }

        let mut open = self.show_info;
        egui::Window::new("信息")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(340.0);

                ui.heading(APP_TITLE);
                ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
                ui.label(APP_DESCRIPTION);

                ui.separator();
                ui.label("串口后端: serialport");
                ui.label("界面框架: eframe / egui");
                match eframe::storage_dir(APP_TITLE) {
                    Some(dir) => {
                        ui.label(format!("设置存档: {}", dir.display()));
                    }
                    None => {
                        ui.label("设置存档: 当前平台不适用");
                    }
                }

                ui.separator();
                ui.label("快捷键");
                ui.label("Enter 发送 / Shift+Enter 换行");
                ui.label("接收区右键：复制全部日志");
            });
        self.show_info = open;
    }
}
