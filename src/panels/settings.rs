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
use crate::send::{
    MAX_REPEAT_INTERVAL_MS, MAX_SEND_HISTORY, MAX_SEND_PRESETS, MIN_REPEAT_INTERVAL_MS, SendFormat,
};
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

/// 设置窗口内容区的高度上限。
///
/// **为什么必须给窗口一个上限**：窗口默认按内容自动撑高（`Resize` 的 `auto_sized` 语义），
/// 内容一长，窗口就会一直长到超出屏幕——底部的「恢复默认」按钮被顶到屏幕外，且**没有滚动条**
/// 够得着（egui 的 `Window` 默认是不滚动的，`ScrollArea::neither()`）。给了上限之后超出部分
/// 才落进下面 [`SCROLL_ID`] 那个滚动区。
///
/// 720 这个取值的取舍：设置项目前约 900px 高，在 1080p 屏幕上窗口约 1.6 屏——刚好"看得见
/// 有东西在下面"，滚动条也会因为内容溢出而现身。取太小（比如 480）会让每一项都显得局促，
/// 明明屏幕还有空间却要一直滚；取太大则退化成"没有上限"的老问题。
/// 窗口本身仍可手动拉高（`resizable(true)`），那条路径不受这个默认值限制。
const SETTINGS_MAX_HEIGHT: f32 = 720.0;

/// 滚动区的 id。
///
/// 用显式 id 而不是 `ScrollArea` 的默认 id：加载期修复（见 `config::restore_runtime_state`）
/// 或将来别的调用点若也要复位滚动位置，得有一个稳定的名字可指。默认 id 由调用栈位置推导，
/// 位置一变就换了个"身份"，之前存的滚动位置会神秘丢失。
const SCROLL_ID: &str = "settings_scroll";

impl ComTool {
    /// 绘制设置窗口（`show_settings` 为 false 时直接返回）。
    ///
    /// 这里只负责**窗口外壳**（标题栏、尺寸上限、开关、滚动区）；内容在
    /// [`Self::settings_body`]，会弹对话框的那几个动作也在那里就地派发。
    pub(crate) fn show_settings_window(&mut self, ui: &mut Ui) {
        if !self.show_settings {
            return;
        }

        let s = self.language.strings();

        // 用局部变量接收窗口开关再回写：与 sidebar.rs 同一套写法，避免在闭包里
        // 同时可变借用 self 与 self.show_settings。
        let mut open = self.show_settings;
        egui::Window::new(s.settings_title)
            .collapsible(false)
            .resizable(true)
            // 没有这行，窗口会随内容无限长高、底部控件被顶出屏幕且无处可滚。
            .max_height(SETTINGS_MAX_HEIGHT)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(20.0, 20.0))
            .show(ui.ctx(), |ui| {
                ui.set_min_width(400.0);

                // 设置项已经比一屏还长，全部塞进一个纵向滚动区。
                //
                // 为什么是 `ScrollArea` 而不是把窗口本身设成可滚：`Window` 确实也有 `.vscroll()`
                // 开关，但那样滚动条会**长在标题栏下面、跟着整个窗口框走**，几项视觉细节
                // （圆角/内边距/滚动条与内容的贴合）都由 `Window` 的 frame 决定。这里只需要
                // "内容可滚"，用内容级滚动区更直接，也不受窗口外观实现变动影响。
                //
                // `auto_shrink([false, true])`：**宽度不要收缩**——纵向滚动条默认会挤占宽度，
                // 若让滚动区按内容宽度收缩，里面的 `ui.separator()` 等填满宽度的控件会跟着变窄，
                // 纵向滚动条一出现/消失，整排控件就左右抖一下。高度保持收缩（`true`），
                // 内容没超上限时不留空白。
                egui::ScrollArea::vertical()
                    .id_salt(SCROLL_ID)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        self.settings_body(ui, s);
                    });
            });
        self.show_settings = open;
    }

    /// 设置窗口的**内容**部分。
    ///
    /// 从 [`Self::show_settings_window`] 里抽出来，理由与 `render_send_presets` 相同
    /// （见 `AGENTS.md`）：只有 `&mut Ui` 的绘制入口才可能被无头测试画到，而
    /// `show_settings_window` 里套着 `Window::show` 的闭包，很难单独驱动。
    ///
    /// 用 `&mut ComTool` 上的方法而不是自由函数，是因为它几乎逐项都在读写 `self` 的字段；
    /// 抽成自由函数得传进去二十来个 `&mut`，反而更难读。
    ///
    /// `pick_dir` / `reset_dir` 这几个"意图"仍走"先记录、后派发"：闭包（这里是本函数体）
    /// 里只置位局部变量，真正会弹对话框的调用放在**所有控件画完之后**——与 sidebar 的
    /// action 同一条约定，绘制过程中不弹模态框。
    ///
    /// `pub(crate)` 是为了让 `panels::tests` 能单独画它（与 `render_send_presets` 同样的理由，
    /// 见约定 43）。
    pub(crate) fn settings_body(&mut self, ui: &mut Ui, s: &crate::strings::Strings) {
        let mut pick_dir = false;
        let mut reset_dir = false;
        let mut clear_chart = false;
        let mut clear_send_history = false;
        let mut clear_send_presets = false;

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
        // 只显示**当前那一档**的说明。三档的解析规则互不相容（HEX 把输入当
        // 十六进制文本、转义把 `\n` 当字节、文本什么都不解释），把三份规则
        // 一起摆出来，用户得自己挑哪一段才适用于自己选的档，反而更容易看错。
        match self.send_format {
            // 文本档没有额外规则可讲，不占地方。
            SendFormat::Text => {}
            SendFormat::Hex => {
                ui.weak(s.hex_note);
            }
            SendFormat::Escape => {
                ui.weak(s.escape_note);
            }
        }
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
        // 常用数据（N2）在这里只放"还剩多少位置"和"一键清空"：内容本身在
        // 发送区编辑（那里才看得见发送效果）。**计数必须有**——列表满了会
        // 拒绝新增，用户得能提前知道还剩几条。
        ui.horizontal(|ui| {
            ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(s.presets_header));
            ui.label(fill(
                s.preset_count_text,
                &[&self.send_presets.len(), &MAX_SEND_PRESETS],
            ));
            if ui.button(s.clear_presets).clicked() {
                clear_send_presets = true;
            }
        });

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
                egui::TextEdit::singleline(&mut self.default_log_file_name).desired_width(240.0),
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
        if clear_send_presets {
            self.send_presets.clear();
        }
    }
}
