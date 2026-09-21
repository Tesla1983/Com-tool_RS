//! 顶部菜单栏：文件（信息 / 退出）· 设置（设置窗口 / 主题）· 视图（面板显隐 / 显示模式）。
//!
//! 菜单栏必须先于其它面板绘制：egui 的面板是"抢占"布局的，先画的先占用整幅宽度，
//! 顶部栏放在最前才能横跨整个窗口。

use crate::app::{ComTool, ThemeChoice};
use crate::receive::TimestampPrefix;
use crate::send::{MAX_SEND_HISTORY, SendFormat, SendLineEnding};
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
                    self.display_mode_checkboxes(ui);
                    ui.separator();
                    ui.checkbox(&mut self.auto_scroll, "自动滚动");
                    ui.separator();
                    ui.label("时间戳前缀");
                    timestamp_radios(ui, &mut self.timestamp_prefix);
                });

                ui.menu_button("发送", |ui| {
                    ui.label("格式");
                    send_format_radios(ui, &mut self.send_format);
                    ui.separator();
                    ui.label("行尾");
                    send_line_ending_radios(ui, &mut self.send_line_ending);
                    ui.separator();
                    if ui.checkbox(&mut self.repeat_send, "定时发送").changed() && self.repeat_send
                    {
                        // 勾上的那一刻只起步计时，不立刻发一包（见 `ComTool::tick_repeat_send`）
                        self.last_repeat_send = None;
                    }
                    if self.repeat_send {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.repeat_interval_ms)
                                    .range(
                                        crate::send::MIN_REPEAT_INTERVAL_MS
                                            ..=crate::send::MAX_REPEAT_INTERVAL_MS,
                                    )
                                    .speed(10.0)
                                    .suffix(" ms"),
                            );
                        });
                    }
                    ui.separator();
                    self.send_history_menu(ui);
                });
            });
        });

        action
    }

    /// 「HEX 接收」与「字节查看」勾选，并保证两者**互斥**。
    ///
    /// 互斥规则本身只有一处实现（[`ComTool::resolve_display_mode`]），这里只负责把
    /// "本轮哪个刚被勾上"传给它。用局部变量而不是直接绑定字段，是因为本轮改动前后的
    /// 值都要参与判断，且必须一次性写回——边画边改字段会让两个勾选框在同一帧里互相覆盖。
    fn display_mode_checkboxes(&mut self, ui: &mut Ui) {
        let mut hex_view = self.hex_view;
        let mut ascii_view = self.ascii_view;
        let hex_just_enabled = ui.checkbox(&mut hex_view, "HEX 接收").changed() && hex_view;
        let ascii_just_enabled = ui.checkbox(&mut ascii_view, "字节查看").changed() && ascii_view;
        self.hex_view = hex_view;
        self.ascii_view = ascii_view;
        self.resolve_display_mode(hex_just_enabled, ascii_just_enabled);
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

/// 时间戳前缀三选一。同样由菜单与设置窗口共用——选项集合只有一处定义，
/// 就不会出现"菜单里能选每批、设置里只能选到每行"这种不一致。
pub(crate) fn timestamp_radios(ui: &mut Ui, prefix: &mut TimestampPrefix) {
    ui.radio_value(prefix, TimestampPrefix::Off, "关闭");
    ui.radio_value(prefix, TimestampPrefix::PerLine, "每行");
    ui.radio_value(prefix, TimestampPrefix::PerBatch, "每批");
}

/// 发送格式二选一（菜单 / 设置窗口 / 发送区三处共用一处定义）。
pub(crate) fn send_format_radios(ui: &mut Ui, format: &mut SendFormat) {
    ui.radio_value(format, SendFormat::Text, "文本");
    ui.radio_value(format, SendFormat::Hex, "HEX");
}

/// 发送行尾三选一。三值枚举而不是"追加 CR / 追加 LF"两个勾选框：那四个组合里
/// 只有三个有意义，用勾选框表达会让"只加 CR"这种状态无法描述也无法校验。
pub(crate) fn send_line_ending_radios(ui: &mut Ui, ending: &mut SendLineEnding) {
    ui.radio_value(ending, SendLineEnding::None, "无");
    ui.radio_value(ending, SendLineEnding::Lf, "LF");
    ui.radio_value(ending, SendLineEnding::Crlf, "CRLF");
}

impl ComTool {
    /// 「发送历史」下拉：点一条就把它填回输入框。
    ///
    /// 给鼠标用户用；键盘走 ↑ / ↓（判定规则见 [`crate::send::HistoryNav`]）。
    /// 两个入口改的是同一份 `send_history`，不存在"两个历史"。
    pub(crate) fn send_history_menu(&mut self, ui: &mut Ui) {
        if self.send_history.is_empty() {
            ui.add_enabled(false, egui::Button::new("发送历史"));
            ui.weak("还没有发送记录");
            return;
        }

        ui.menu_button(
            format!(
                "发送历史（{} / {MAX_SEND_HISTORY}）",
                self.send_history.len()
            ),
            |ui| {
                // 从最新往旧列，符合"想找回刚发过的那条"的实际用法。
                let mut picked: Option<String> = None;
                for text in self.send_history.entries().iter().rev() {
                    // 多行内容在菜单里只显示第一行，否则会把菜单撑爆
                    let label = text.lines().next().unwrap_or("").to_string();
                    let label = if label.is_empty() {
                        "<空行>".to_string()
                    } else {
                        label
                    };
                    if ui.button(label).on_hover_text(text.as_str()).clicked() {
                        picked = Some(text.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("清空历史").clicked() {
                    self.send_history.clear();
                    self.send_history_nav.reset();
                    ui.close();
                }
                if let Some(text) = picked {
                    self.send_text = text;
                    // 填回来的内容与"正在浏览的历史位置"无关，回到编辑态
                    self.send_history_nav.reset();
                }
            },
        );
    }
}
