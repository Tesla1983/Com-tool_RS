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
use chrono::Local;
use eframe::egui;
use egui::Ui;

use super::menu_bar::{
    send_format_radios, send_line_ending_radios, theme_radios, timestamp_radios,
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
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("时间戳前缀"));
                    timestamp_radios(ui, &mut self.timestamp_prefix);
                });
                ui.weak("形如 [12:00:00.123] 的本地时间前缀；每行 = 每个换行后重新标注，每批 = 每次收到数据标注一次");
                ui.weak("前缀是显示注解而不是收到的数据：它会随接收区一起进导出文件与实时日志，但不会进入图表解析");
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
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("负载格式"));
                    send_format_radios(ui, &mut self.send_format);
                });
                ui.weak("HEX 时输入按十六进制解析：允许空格 / 逗号分隔与 0x 前缀；解析失败不会发送，也不会清空输入框");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("行尾"));
                    send_line_ending_radios(ui, &mut self.send_line_ending);
                });
                ui.weak("追加在字节层：HEX 模式下发出的是 0x0A / 0x0D 0x0A 两个字节，不是字符");
                ui.checkbox(&mut self.clear_send_after_send, "发送后清空输入框");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("定时发送间隔"));
                    ui.add(
                        egui::DragValue::new(&mut self.repeat_interval_ms)
                            .range(MIN_REPEAT_INTERVAL_MS..=MAX_REPEAT_INTERVAL_MS)
                            .speed(10.0),
                    );
                    ui.label(format!("ms（{MIN_REPEAT_INTERVAL_MS}~{MAX_REPEAT_INTERVAL_MS}）"));
                });
                ui.weak("开关在发送区；断开连接会自动关掉，且不跨进程延续（重启后一定是关的）");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("发送历史"));
                    ui.label(format!("{} / {MAX_SEND_HISTORY} 条", self.send_history.len()));
                    if ui.button("清空历史").clicked() {
                        clear_send_history = true;
                    }
                });
                ui.weak("↑ / ↓ 在输入框首行 / 末行时取历史（首行按 ↑、末行按 ↓）；连续发送相同内容只记一条");

                ui.separator();
                ui.heading("实时图表");
                ui.checkbox(
                    &mut self.show_chart_panel,
                    "显示图表面板（也可从「视图 → 面板」切换）",
                );
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("样本上限"));
                    ui.add(
                        egui::DragValue::new(&mut self.chart_max_samples)
                            .range(MIN_SAMPLES..=MAX_SAMPLES)
                            .speed(100.0),
                    );
                    ui.label(format!("点/曲线（{MIN_SAMPLES}~{MAX_SAMPLES}）"));
                });
                ui.weak("超出上限时从最旧的样本开始淘汰；绘制时每条曲线最多取 4000 点");
                ui.checkbox(
                    &mut self.chart_auto_bounds,
                    "自动缩放（关掉后保留手动拖动/缩放的视图）",
                );
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("当前数据"));
                    if ui.button("清空图表").clicked() {
                        clear_chart = true;
                    }
                    ui.label(format!(
                        "{} 通道 · {} 样本",
                        self.chart.channel_count(),
                        self.chart.sample_count()
                    ));
                });
                ui.weak("格式：一行一组样本，字段用逗号分隔（如 1.0,2.0,3.0）；与接收显示模式无关");

                ui.separator();
                ui.heading("文件日志");
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("文件名模板"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.default_log_file_name)
                            .desired_width(240.0),
                    );
                });
                // 把模板实时展开给用户看：占位符写错（例如 %Q）时会退回默认模板，
                // 与其等开始记录时才发现，不如当场看见将要生成的名字。
                ui.weak(format!(
                    "下次开始记录时生成: {}",
                    render_log_file_name(&self.default_log_file_name, Local::now())
                ));

                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new("输出目录"));
                    ui.label(self.effective_log_dir().display().to_string());
                });
                ui.horizontal(|ui| {
                    ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(""));
                    if ui.button("选择目录").clicked() {
                        pick_dir = true;
                    }
                    if ui.button("恢复默认").clicked() {
                        reset_dir = true;
                    }
                    if self.is_logging() {
                        ui.label(format!("记录中，已写 {} 字节", self.log_written_bytes()));
                    }
                });
                ui.weak("接收区的内容会同步写入该文件；日志文件不受上面的长度上限约束");

                ui.separator();
                ui.weak("以上均为用户偏好，改动即时生效并在退出时自动保存");
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
