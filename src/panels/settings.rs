//! 设置窗口：把 `ComTool` 上的用户偏好接到真正的控件上。
//!
//! 旧版这里是「只有一个语言占位下拉」的空壳；现在每一项都绑到实际生效的字段。
//! 生效范围分三类，界面上如实标注：
//!
//! - **立即生效**：主题、时间戳前缀、日志长度上限、自动滚动、重绘间隔、发送后清空、
//!   文件名模板、图表样本上限、图表自动缩放；
//! - **打开窗口时生效**：日志输出目录（记录过程中改动会在下次开始记录时生效）；
//! - **已移除**：启动时检查更新。P6 决定不做版本检查——内网环境访问不到检查地址，
//!   留一个「开了也没用」的开关比没有更糟，因此连同设置项一并删除。

use crate::app::{ComTool, Language};
use crate::chart::{MAX_SAMPLES, MIN_SAMPLES};
use crate::log_file::render_log_file_name;
use crate::send::{MAX_REPEAT_INTERVAL_MS, MAX_SEND_HISTORY, MIN_REPEAT_INTERVAL_MS};
use crate::strings::fill;
use chrono::Local;
use eframe::egui;
use egui::Ui;

use super::menu_bar::{
    send_format_radios, send_line_ending_radios, text_encoding_radios, theme_radios,
    timestamp_radios,
};

/// 左侧标签列的宽度，让右侧控件纵向对齐。
const LABEL_WIDTH: f32 = 104.0;

impl ComTool {
    /// 绘制设置窗口（`show_settings` 为 false 时直接返回）。
    pub(crate) fn show_settings_window(&mut self, ui: &mut Ui) {
        if !self.show_settings {
            return;
        }

        // 目录选择会弹出模态对话框，不能在绘制过程中直接调用：先记录意图，
        // 等窗口画完、闭包结束之后再派发（与 sidebar 的 action 同一条约定）。
        let mut pick_dir = false;
        let mut reset_dir = false;
        let mut clear_chart = false;
        let mut clear_send_history = false;

        let s = self.language.strings();

        // 用局部变量接收窗口开关再回写：与 sidebar.rs 同一套写法，避免在闭包里
        // 同时可变借用 self 与 self.show_settings。
        let mut open = self.show_settings;
        egui::Window::new(s.settings_title)
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(20.0, 20.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(400.0);

                ui.heading(s.sec_interface);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.theme_label));
                    theme_radios(ui, &mut self.theme, s);
                });
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.language_label));
                    // 「中文」/「English」两档**各自写自己的名字**，不随界面语言变：
                    // 界面已经切成英文时，找中文那一档的人要认得出"中文"两个字。
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
                ui.weak(s.language_applies_immediately);

                ui.separator();
                ui.heading(s.sec_text_encoding);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.encoding_label));
                    text_encoding_radios(ui, &mut self.text_encoding);
                });
                ui.weak(s.encoding_shared_note);
                ui.weak(s.encoding_gbk_note);
                ui.weak(s.encoding_switch_note);

                ui.separator();
                ui.heading(s.sec_receive);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [LABEL_WIDTH, 20.0],
                        egui::Label::new(s.timestamp_prefix_field),
                    );
                    timestamp_radios(ui, &mut self.timestamp_prefix, s);
                });
                ui.weak(s.ts_note);
                ui.weak(s.ts_note_2);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.max_log_field));
                    ui.add(
                        egui::DragValue::new(&mut self.max_log_chars)
                            .range(0..=1_000_000)
                            .speed(200.0),
                    );
                    ui.label(s.chars_unlimited);
                });
                ui.weak(s.max_log_note);
                ui.checkbox(&mut self.auto_scroll, s.auto_scroll_latest);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.repaint_field));
                    ui.add(
                        egui::DragValue::new(&mut self.repaint_interval_ms)
                            .range(16..=1000)
                            .speed(1.0),
                    );
                    ui.label(s.repaint_note);
                });

                ui.separator();
                ui.heading(s.send_title);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.payload_format));
                    send_format_radios(ui, &mut self.send_format, s);
                });
                ui.weak(s.hex_note);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.line_ending));
                    send_line_ending_radios(ui, &mut self.send_line_ending, s);
                });
                ui.weak(s.ending_note);
                ui.checkbox(&mut self.clear_send_after_send, s.clear_after_send);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [LABEL_WIDTH, 20.0],
                        egui::Label::new(s.repeat_interval_field),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.repeat_interval_ms)
                            .range(MIN_REPEAT_INTERVAL_MS..=MAX_REPEAT_INTERVAL_MS)
                            .speed(10.0),
                    );
                    ui.label(fill(
                        s.repeat_interval_range,
                        &[&MIN_REPEAT_INTERVAL_MS, &MAX_REPEAT_INTERVAL_MS],
                    ));
                });
                ui.weak(s.repeat_note);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.send_history_label));
                    ui.label(fill(
                        s.send_history_count_text,
                        &[&self.send_history.len(), &MAX_SEND_HISTORY],
                    ));
                    if ui.button(s.clear_history).clicked() {
                        clear_send_history = true;
                    }
                });
                ui.weak(s.history_note);

                ui.separator();
                ui.heading(s.nav_chart);
                ui.checkbox(&mut self.show_chart_panel, s.chart_panel_checkbox);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.max_samples_field));
                    ui.add(
                        egui::DragValue::new(&mut self.chart_max_samples)
                            .range(MIN_SAMPLES..=MAX_SAMPLES)
                            .speed(100.0),
                    );
                    ui.label(fill(s.samples_range, &[&MIN_SAMPLES, &MAX_SAMPLES]));
                });
                ui.weak(s.samples_note);
                ui.checkbox(&mut self.chart_auto_bounds, s.auto_bounds_checkbox);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.current_data));
                    if ui.button(s.clear_chart).clicked() {
                        clear_chart = true;
                    }
                    ui.label(fill(
                        s.chart_count_text,
                        &[&self.chart.channel_count(), &self.chart.sample_count()],
                    ));
                });
                ui.weak(s.chart_format_note);

                ui.separator();
                ui.heading(s.file_log);
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.file_name_template));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.default_log_file_name)
                            .desired_width(240.0),
                    );
                });
                // 把模板实时展开给用户看：占位符写错（例如 %Q）时会退回默认模板，
                // 与其等开始记录时才发现，不如当场看见将要生成的名字。
                ui.weak(fill(
                    s.next_file_name,
                    &[&render_log_file_name(
                        &self.default_log_file_name,
                        Local::now(),
                    )],
                ));

                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.output_dir_field));
                    ui.label(self.effective_log_dir().display().to_string());
                });
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(""));
                    if ui.button(s.pick_dir).clicked() {
                        pick_dir = true;
                    }
                    if ui.button(s.restore_default).clicked() {
                        reset_dir = true;
                    }
                    if self.is_logging() {
                        ui.label(fill(s.logging_written, &[&self.log_written_bytes()]));
                    }
                });
                ui.weak(s.log_note);

                ui.separator();
                ui.weak(s.prefs_note);
            });
        self.show_settings = open;

        if pick_dir {
            self.pick_log_dir();
        }
        if reset_dir {
            self.reset_log_dir();
        }
        if clear_chart {
            self.clear_chart();
        }
        if clear_send_history {
            self.send_history.clear();
            // 历史没了，浏览位置也必须一起复位——否则 ↑ 会基于旧下标去取，
            // 表现为"按了没反应"。
            self.send_history_nav.reset();
        }
    }
}
