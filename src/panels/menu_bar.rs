//! 顶部菜单栏：文件（信息 / 退出）· 设置（设置窗口 / 主题）· 视图（面板显隐 / 显示模式）·
//! 文本（编码）。
//!
//! **没有「发送」菜单**：它的内容与「发送区」表头那一行完全重复，2026-09-24 按用户
//! 要求删除。发送格式 / 行尾 / 定时发送 / 发送历史都在发送区里，功能一个没少。
//!
//! 菜单栏必须先于其它面板绘制：egui 的面板是"抢占"布局的，先画的先占用整幅宽度，
//! 顶部栏放在最前才能横跨整个窗口。

use crate::app::{ComTool, ThemeChoice};
use crate::receive::TextEncoding;
use crate::receive::TimestampPrefix;
use crate::send::{MAX_SEND_HISTORY, SendFormat, SendLineEnding};
use crate::strings::{Strings, fill};
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
        let s = self.language.strings();

        // 「总在最前端」在这里同步给窗口，而不是在 `ui()` 里：`ui()` 要收
        // `eframe::Frame`，无头测试构造不出来 ⇒ 写在里面等于一次也不会被 `cargo test`
        // 走到。菜单栏每帧必画，挂在它上面与每帧同步等价，同时让"勾选 → 下发命令"
        // 整条链路有接入测试。完整理由见 [`ComTool::apply_always_on_top`]。
        self.apply_always_on_top(ui.ctx());

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(s.menu_file, |ui| {
                    if ui.button(s.menu_info).clicked() {
                        action.info_requested = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(s.menu_quit).clicked() {
                        action.quit_requested = true;
                        ui.close();
                    }
                });

                ui.menu_button(s.settings_title, |ui| {
                    if ui.button(s.menu_open_settings).clicked() {
                        action.settings_requested = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.label(s.theme_label);
                    theme_radios(ui, &mut self.theme, s);
                });

                ui.menu_button(s.menu_view, |ui| {
                    self.view_menu_body(ui);
                });

                ui.menu_button(s.text, |ui| {
                    ui.label(s.encoding_shared_label);
                    text_encoding_radios(ui, &mut self.text_encoding);
                    ui.separator();
                    // GBK 与 GB18030 的**解码器相同**、编码器不同。这句说明放在
                    // 界面上而不是只写进文档：用户看到两档"好像一样"时会犯嘀咕。
                    ui.weak(s.encoding_gbk_note_menu);
                });

                // 这里**故意没有**「发送」菜单（2026-09-24 用户要求删除）。
                //
                // 它原先装的是：发送格式单选、行尾单选、定时发送 + 间隔、发送历史——
                // 与「发送区」表头那一行右侧的控件**逐项重复**（格式 / 行尾 / 定时 /
                // 历史一个不少，连排列顺序都一样）。同一批设置摆两套入口，用户改了一处
                // 会怀疑另一处是否同步，而两处改的其实是**同一份字段**（直接绑 `self.*`），
                // 从不存在不同步——既然如此，多出来的那套就只剩干扰。
                //
                // **被删掉的是什么**：只有菜单栏这个入口。发送格式 / 行尾 / 定时 / 历史
                // 这四项功能**一个没少**，全都在发送区表头那一行里。想改这些设置请去那里。
                //
                // 复活条件：若将来发送区被整体隐藏（`show_send_panel == false`）时仍希望
                // 能改发送格式，再考虑加回来——但那时应先想清楚"没有发送区时改格式给谁用"。
            });
        });

        action
    }

    /// 「视图」菜单的主体：面板显隐 / 显示模式 / 自动滚动 / 总在最前端 / 时间戳前缀。
    ///
    /// 从 `show_menu_bar` 的闭包里抽出来是为了**可测**：菜单主体只在菜单被点开时才画，
    /// 无头测试要模拟"点开菜单"就得先算出菜单按钮的位置、再伪造一串指针事件——
    /// 与其让判据依赖那堆坐标，不如让主体自己可被直接驱动（与
    /// [`ComTool::render_send_presets`] 同一条约定：绘制逻辑要能脱离 `eframe::Frame`
    /// 单独跑一遍）。
    ///
    /// 「总在最前端」放在「自动滚动」之后、单独一行：它管的不是接收区的行为，是
    /// **窗口本身**——和其它接收设置挤在一起会让人以为只影响接收区。
    pub(crate) fn view_menu_body(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        ui.label(s.panel_label);
        self.show_panel_checkboxes(ui);
        ui.separator();
        ui.label(s.display_mode_heading);
        self.display_mode_checkboxes(ui);
        ui.separator();
        ui.checkbox(&mut self.auto_scroll, s.auto_scroll);
        ui.separator();
        ui.checkbox(&mut self.always_on_top, s.always_on_top);
        ui.separator();
        ui.label(s.timestamp_prefix_field);
        timestamp_radios(ui, &mut self.timestamp_prefix, s);
    }

    /// 「颜色」「HEX 接收」「字节查看」三个勾选框，并保证三者的组合**合法**。
    ///
    /// 互斥规则本身只有一处实现（[`ComTool::resolve_display_mode`]），这里只负责把
    /// "本轮哪个刚被勾上"传给它。用局部变量而不是直接绑定字段，是因为本轮改动前后的
    /// 值都要参与判断，且必须一次性写回——边画边改字段会让勾选框在同一帧里互相覆盖。
    ///
    /// **三个必须在这里一起收集、只调一次裁决函数**。「颜色」原先是在调用点单独
    /// `ui.checkbox(&mut self.color_view, …)` 画的，压根没进这个函数 —— 于是菜单这条
    /// 入口可以把它与「字节查看」同时勾上，接收区一片空白（2026-09-24 用户目视发现，
    /// 见 `HANDOVER.md` §3.16）。绘制顺序沿用改动前的外观：颜色 → HEX 接收 → 字节查看。
    fn display_mode_checkboxes(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        let mut color_view = self.color_view;
        let mut hex_view = self.hex_view;
        let mut ascii_view = self.ascii_view;
        let color_just_enabled = ui.checkbox(&mut color_view, s.color).changed() && color_view;
        let hex_just_enabled = ui.checkbox(&mut hex_view, s.hex_receive).changed() && hex_view;
        let ascii_just_enabled = ui.checkbox(&mut ascii_view, s.byte_view).changed() && ascii_view;
        self.color_view = color_view;
        self.hex_view = hex_view;
        self.ascii_view = ascii_view;
        self.resolve_display_mode(hex_just_enabled, ascii_just_enabled, color_just_enabled);
    }

    /// 面板显隐勾选，并保证**至少保留一个可见面板**。
    ///
    /// 规则：当只剩一个面板可见时，禁止把它取消掉（该项置灰）；已经隐藏的面板
    /// 始终允许重新勾上。这样用户永远不可能把界面点成一整片空白。
    fn show_panel_checkboxes(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        let can_hide = self.visible_panel_count() > 1;
        ui.add_enabled(
            can_hide || !self.sidebar_open,
            egui::Checkbox::new(&mut self.sidebar_open, s.nav_sidebar),
        );
        ui.add_enabled(
            can_hide || !self.show_receive_panel,
            egui::Checkbox::new(&mut self.show_receive_panel, s.nav_receive),
        );
        ui.add_enabled(
            can_hide || !self.show_chart_panel,
            egui::Checkbox::new(&mut self.show_chart_panel, s.nav_chart),
        );
        ui.add_enabled(
            can_hide || !self.show_send_panel,
            egui::Checkbox::new(&mut self.show_send_panel, s.nav_send),
        );
        if !can_hide {
            ui.weak(s.at_least_one_panel);
        }
    }
}

/// 主题三选一。菜单与设置窗口共用同一套写法，避免两处出现不同的选项集合。
pub(crate) fn theme_radios(ui: &mut Ui, theme: &mut ThemeChoice, s: &Strings) {
    ui.radio_value(theme, ThemeChoice::System, s.theme_system);
    ui.radio_value(theme, ThemeChoice::Light, s.theme_light);
    ui.radio_value(theme, ThemeChoice::Dark, s.theme_dark);
}

/// 时间戳前缀三选一。同样由菜单与设置窗口共用——选项集合只有一处定义，
/// 就不会出现"菜单里能选每批、设置里只能选到每行"这种不一致。
pub(crate) fn timestamp_radios(ui: &mut Ui, prefix: &mut TimestampPrefix, s: &Strings) {
    ui.radio_value(prefix, TimestampPrefix::Off, s.ts_off);
    ui.radio_value(prefix, TimestampPrefix::PerLine, s.ts_per_line);
    ui.radio_value(prefix, TimestampPrefix::PerBatch, s.ts_per_batch);
}

/// 文本编码单选（接收解码与发送编码共用同一个偏好）。
///
/// 选项集合由 [`TextEncoding::ALL`] 定义、这里只负责画：加一档编码时不会出现
/// "设置窗口里有、菜单里没有"这种不一致。
///
/// 不需要文案表：`UTF-8` / `GBK` / `GB18030` / `UTF-16LE` 是**编码的正式名字**，
/// 中英文界面下都写同一个词，翻译反而会让人对不上厂商文档。
pub(crate) fn text_encoding_radios(ui: &mut Ui, encoding: &mut TextEncoding) {
    for option in TextEncoding::ALL {
        ui.radio_value(encoding, option, option.label());
    }
}

/// 发送格式二选一（菜单 / 设置窗口 / 发送区三处共用一处定义）。
///
/// `HEX` 是专有名词，中英两列取值相同——这是事实，不是漏翻译
/// （`strings.rs` 的 `identical_entries_are_language_neutral_only` 认这一点）。
/// 发送格式三选一（N3 起是三档，不是两档）。
///
/// **只在这里定义一次**：菜单栏、发送区、设置窗口三处都调它。三处各写一遍的
/// 后果是"某天加了第四档，只改了两处"——界面上就出现了"设置窗口里能选、发送区
/// 里选不到"的格式。
pub(crate) fn send_format_radios(ui: &mut Ui, format: &mut SendFormat, s: &Strings) {
    ui.radio_value(format, SendFormat::Text, s.text);
    ui.radio_value(format, SendFormat::Hex, s.hex);
    ui.radio_value(format, SendFormat::Escape, s.escape);
}

/// 发送行尾三选一。三值枚举而不是"追加 CR / 追加 LF"两个勾选框：那四个组合里
/// 只有三个有意义，用勾选框表达会让"只加 CR"这种状态无法描述也无法校验。
pub(crate) fn send_line_ending_radios(ui: &mut Ui, ending: &mut SendLineEnding, s: &Strings) {
    ui.radio_value(ending, SendLineEnding::None, s.ending_none);
    ui.radio_value(ending, SendLineEnding::Lf, s.ending_lf);
    ui.radio_value(ending, SendLineEnding::Crlf, s.ending_crlf);
}

impl ComTool {
    /// 「发送历史」下拉：点一条就把它填回输入框。
    ///
    /// 给鼠标用户用；键盘走 ↑ / ↓（判定规则见 [`crate::send::HistoryNav`]）。
    /// 两个入口改的是同一份 `send_history`，不存在"两个历史"。
    pub(crate) fn send_history_menu(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        if self.send_history.is_empty() {
            // 历史为空时这颗按钮**留在原地但置灰**（而不是整个消失）：位置稳定，
            // 用户不会看到表头那一行的控件左右跳动。禁用态用 `add_enabled_ui` 下发，
            // 自绘按钮读 `ui.is_enabled()` 取禁用配色、且灰着就点不动。
            ui.add_enabled_ui(false, |ui| {
                crate::button::standard(ui, s.send_history_label);
            });
            ui.weak(s.no_send_history);
            return;
        }

        ui.menu_button(
            fill(
                s.send_history_count,
                &[&self.send_history.len(), &MAX_SEND_HISTORY],
            ),
            |ui| {
                // 从最新往旧列，符合"想找回刚发过的那条"的实际用法。
                let mut picked: Option<String> = None;
                for text in self.send_history.entries().iter().rev() {
                    // 多行内容在菜单里只显示第一行，否则会把菜单撑爆
                    let label = text.lines().next().unwrap_or("").to_string();
                    let label = if label.is_empty() {
                        s.empty_line.to_string()
                    } else {
                        label
                    };
                    if ui.button(label).on_hover_text(text.as_str()).clicked() {
                        picked = Some(text.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(s.clear_history).clicked() {
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
