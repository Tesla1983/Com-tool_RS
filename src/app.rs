//! 应用状态与顶层编排。
//!
//! 本模块只保留三类内容：
//!
//! 1. [`ComTool`] 的状态定义与默认值；
//! 2. 串口生命周期操作（打开 / 关闭 / 发送 / 自动重连 / 导出）；
//! 3. [`eframe::App`] 的顶层编排——按顺序画出菜单栏、侧边栏与中央区，并把用户操作
//!    派发到对应的方法上。
//!
//! 接收链路的解码与裁剪在 [`crate::receive`]，右侧边栏在 [`crate::sidebar`]，
//! 菜单栏与各类窗口在 [`crate::panels`]。

use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::receive::{AnsiDecoder, AnsiStyle, ColoredSpan, show_plain_receive};
use crate::serial_impl::{DEFAULT_BAUD_RATE, SerialChannel, SerialConfig, scan_ports};
use eframe::egui;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// 界面语言（方案 A：目前仅作为设置项占位，尚未接入文案切换）
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Zh,
    En,
}

/// 主题偏好。
///
/// 不直接存 `egui::ThemePreference`：那是外部类型，取值一旦被上游调整就会让旧存档
/// 读不出来。这里用自有枚举做持久化，转换只发生在应用主题的那一刻。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeChoice {
    /// 跟随系统（与 egui 自身默认值一致）
    #[default]
    System,
    Light,
    Dark,
}

impl From<ThemeChoice> for egui::ThemePreference {
    fn from(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::System => Self::System,
            ThemeChoice::Light => Self::Light,
            ThemeChoice::Dark => Self::Dark,
        }
    }
}

/// 日志长度上限的默认值（字符数），与参考实现的默认档位一致。
pub const DEFAULT_MAX_LOG_CHARS: usize = 30_000;

/// 默认重绘间隔（毫秒）。16 ms ≈ 60 FPS，是设置项允许的下限。
pub const DEFAULT_REPAINT_INTERVAL_MS: u64 = 30;

/// 默认日志文件名模板（P4 实时文件日志使用，`chrono` 负责展开时间占位）。
pub const DEFAULT_LOG_FILE_NAME: &str = "LogFile_%Y-%m-%d_%H-%M-%S.txt";

/// 应用状态。
///
/// 序列化策略见 [`crate::config`]：带 `#[serde(skip)]` 的字段是运行时状态，
/// 不落盘，启动时由 `restore_runtime_state` 重建。容器级 `#[serde(default)]`
/// 保证以后新增字段时，旧存档依然读得出来。
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ComTool {
    // 串口列表（每次启动按当前系统重新枚举，不沿用存档快照）
    #[serde(skip)]
    pub ports: Vec<String>,

    // 当前选择的串口
    pub selected_port: String,

    // 当前波特率
    pub baud_rate: u32,

    // 当前串口参数
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,

    // 是否已经打开串口（活不过进程边界）
    #[serde(skip)]
    pub connected: bool,

    // 接收区（无长度上限，不落盘；见 crate::config 的模块说明）
    #[serde(skip)]
    pub receive_text: String,

    // 发送区
    pub send_text: String,

    // 串口通信管理器（基于 ByteChannel trait，句柄不可序列化）
    #[serde(skip)]
    pub serial: Option<Box<dyn ByteChannel>>,

    // 串口事件通道（Bytes / Failed / Closed）
    #[serde(skip)]
    pub event_rx: Option<Receiver<ChannelEvent>>,

    // 上次自动重连尝试时间（用于节流，避免高频重试）
    #[serde(skip)]
    pub last_reconnect_attempt: Option<std::time::Instant>,

    // 上次成功打开的串口（自动重连时优先用它，避免跳到 COM1）
    pub last_opened_port: Option<String>,

    pub hex_view: bool,
    #[serde(skip)]
    pub display_spans: Vec<ColoredSpan>,
    #[serde(skip)]
    pub decoder: AnsiDecoder,
    pub color_view: bool,
    pub sidebar_open: bool,
    pub language: Language,
    // 模态窗口开关，不该在启动时自动弹出
    #[serde(skip)]
    pub show_settings: bool,
    #[serde(skip)]
    pub show_info: bool,
    pub ascii_view: bool,

    #[serde(skip)]
    pub ascii_text: String,
    #[serde(skip)]
    pub ascii_spans: Vec<ColoredSpan>,

    // —— 界面与接收行为的用户偏好（全部落盘）——
    /// 主题偏好（浅色 / 深色 / 跟随系统）
    pub theme: ThemeChoice,
    /// 接收区是否自动滚动到最新
    pub auto_scroll: bool,
    /// 日志长度上限（字符数，0 = 不限）；超出后从最旧的内容开始淘汰
    pub max_log_chars: usize,
    /// 界面重绘间隔（毫秒），实际使用时会被夹到 16~1000
    pub repaint_interval_ms: u64,
    /// 发送后是否清空输入框
    pub clear_send_after_send: bool,
    /// 默认日志文件名模板（P4 使用）
    pub default_log_file_name: String,
    /// 启动时是否检查更新（P6 使用，默认关闭）
    pub check_update_on_start: bool,
    /// 接收区是否显示
    pub show_receive_panel: bool,
    /// 发送区是否显示
    pub show_send_panel: bool,
}

impl ComTool {
    /// 从 `eframe` 存储恢复上次会话。
    ///
    /// 存档只提供用户偏好；串口句柄、事件通道、端口列表等运行时状态由
    /// [`crate::config::load`] 统一重建。
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::config::load(cc.storage.as_deref())
    }
}

impl ComTool {
    pub fn refresh_ports(&mut self) {
        self.ports = scan_ports();

        if !self.ports.contains(&self.selected_port) {
            self.selected_port = self.ports.first().cloned().unwrap_or_default();
        }
    }

    pub fn auto_reconnect(&mut self) {
        // 只有串口曾经打开过、现在断开时才尝试自动重连
        if self.connected || self.serial.is_none() {
            return;
        }

        let now = std::time::Instant::now();
        let need_retry = self
            .last_reconnect_attempt
            .map_or(true, |t| now.duration_since(t).as_millis() >= 500);
        if !need_retry {
            return;
        }
        self.last_reconnect_attempt = Some(now);

        let target = self
            .last_opened_port
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.selected_port.clone());
        if target.is_empty() {
            return;
        }
        self.selected_port = target;

        // 只刷新端口列表供 UI 显示，不改变所选端口
        self.ports = scan_ports();

        self.open_port();
    }

    pub fn export_receive_text(&mut self) {
        if self.receive_text.is_empty() {
            self.receive_text.push_str("没有可导出的数据\n");
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .set_title("导出接收数据.txt")
            .add_filter("文本文件", &["txt"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &self.receive_text.as_bytes()) {
            Ok(_) => {
                self.receive_text
                    .push_str(&format!("已导出到: {}\n", path.display()));
            }
            Err(e) => {
                self.receive_text
                    .push_str(&format!("导出接收数据失败: {}\n", e));
            }
        }
    }

    pub fn open_port(&mut self) {
        if self.selected_port.is_empty() {
            return;
        }

        // 首次打开时创建串口通信管理器
        if self.serial.is_none() {
            self.serial = Some(Box::new(SerialChannel::new()));
        }
        let serial = self.serial.as_mut().unwrap();

        // 把当前 UI 选择的参数应用到串口
        let settings = SerialConfig {
            port: self.selected_port.clone(),
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            stop_bits: self.stop_bits,
            parity: self.parity,
            flow_control: self.flow_control,
        };
        if let Err(e) = serial.configure(&settings) {
            self.receive_text
                .push_str(&format!("设置串口参数失败: {}\n", e));
            return;
        }

        let (event_tx, event_rx) = mpsc::channel::<ChannelEvent>();
        if let Err(e) = serial.open(event_tx) {
            self.receive_text
                .push_str(&format!("打开串口失败: {}\n", e));
            return;
        }
        self.event_rx = Some(event_rx);
        self.connected = true;

        self.last_opened_port = Some(self.selected_port.clone());

        let open_msg = format!("已打开 {}\n", self.selected_port);
        self.receive_text.push_str(&open_msg);

        self.display_spans.push(ColoredSpan {
            text: open_msg,
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
    }

    pub fn close_port(&mut self) {
        if let Some(serial) = &mut self.serial {
            let _ = serial.close();
        }

        self.serial = None;
        self.event_rx = None;
        self.last_opened_port = None;
        self.last_reconnect_attempt = None;
        self.connected = false;

        self.receive_text.push_str("串口已关闭\n");
        // 一次性追加一条彩色提示到彩色日志（不随渲染每帧重复）
        self.display_spans.push(ColoredSpan {
            text: "串口已关闭\n".to_string(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(205, 49, 49)), // 红
                bold: false,
            },
        });
    }

    pub fn send_data(&mut self) {
        if !self.connected {
            return;
        }

        let data = self.send_text.as_bytes().to_vec();

        if let Some(serial) = &mut self.serial {
            if let Err(e) = serial.write(data) {
                self.receive_text.push_str(&format!("发送失败: {}\n", e));
            }
        }

        if self.clear_send_after_send {
            self.send_text.clear();
        }
    }
}

impl ComTool {
    /// 当前可见的面板数量（侧边栏 / 接收区 / 发送区）。
    ///
    /// 「至少保留一个可见」这条不变量在两个地方用到：视图菜单里决定勾选框是否可点，
    /// 以及 [`crate::config::restore_runtime_state`] 里修正被手改坏的存档。
    pub fn visible_panel_count(&self) -> usize {
        usize::from(self.sidebar_open)
            + usize::from(self.show_receive_panel)
            + usize::from(self.show_send_panel)
    }

    /// 把主题偏好同步给 egui——仅在与当前生效值不同时调用，避免每帧都触发
    /// `options_mut` 造成的无谓重绘。
    pub fn apply_theme(&self, ctx: &egui::Context) {
        let want: egui::ThemePreference = self.theme.into();
        if ctx.options(|opt| opt.theme_preference) != want {
            ctx.set_theme(want);
        }
    }
}

impl Default for ComTool {
    fn default() -> Self {
        let ports = scan_ports();

        let selected_port = ports.first().cloned().unwrap_or_default();

        Self {
            ports,
            selected_port,
            baud_rate: DEFAULT_BAUD_RATE,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            connected: false,
            receive_text: String::new(),
            send_text: String::new(),
            serial: None,
            event_rx: None,
            last_reconnect_attempt: None,
            last_opened_port: None,
            hex_view: false,
            display_spans: Vec::new(),
            decoder: AnsiDecoder::default(),
            color_view: false,
            sidebar_open: true,
            language: Language::Zh,
            show_settings: false,
            show_info: false,
            ascii_view: false,
            ascii_text: String::new(),
            ascii_spans: Vec::new(),
            theme: ThemeChoice::default(),
            auto_scroll: true,
            max_log_chars: DEFAULT_MAX_LOG_CHARS,
            repaint_interval_ms: DEFAULT_REPAINT_INTERVAL_MS,
            clear_send_after_send: true,
            default_log_file_name: DEFAULT_LOG_FILE_NAME.to_string(),
            check_update_on_start: false,
            show_receive_panel: true,
            show_send_panel: true,
        }
    }
}

impl eframe::App for ComTool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 检查串口接收数据
        self.receive_data();

        // 串口意外断开时自动重连（节流 500ms）
        self.auto_reconnect();

        // 主题偏好只在与当前生效值不同时下发一次
        self.apply_theme(ui.ctx());

        // 不断刷新 GUI；间隔可配，越界值（手改存档）在这里夹回合法区间
        let repaint_ms = self.repaint_interval_ms.clamp(16, 1000);
        ui.ctx()
            .request_repaint_after(Duration::from_millis(repaint_ms));

        // ① 顶部菜单栏必须最先画：egui 的面板先画先占位，先画才能横跨整幅宽度
        let menu = self.show_menu_bar(ui);
        if menu.info_requested {
            self.show_info = true;
        }
        if menu.settings_requested {
            self.show_settings = true;
        }
        if menu.quit_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // ② 右侧边栏
        let mut sidebar_open = self.sidebar_open;
        let action = self.show_sidebar(ui, &mut sidebar_open);

        // 回写状态 + 执行延迟动作（放在绘制之后，避免与 &mut sidebar_open 借用冲突）
        self.sidebar_open = if action.close_requested {
            false
        } else {
            sidebar_open
        };
        if action.clear_requested {
            self.clear_receive(ui);
        }
        if action.export_requested {
            self.export_receive_text();
        }

        // ③ 中央区
        egui::CentralPanel::default().show(ui, |ui| {
            // 设置 / 信息两个窗口跟着中央区一起绘制（主界面的常驻部分）
            self.show_settings_window(ui);
            self.show_info_window(ui);

            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.sidebar_open, "☰ 侧边栏");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ 设置").clicked() {
                        self.show_settings = true;
                    }
                });
            });

            ui.separator();

            if self.show_receive_panel {
                // 接收设置：文本/HEX 用 checkbox 表达（底层互斥，因为收到数据只能按一种格式解析）
                let mut show_hex = self.hex_view;
                let mut show_text = !self.hex_view;
                ui.horizontal(|ui| {
                    ui.label("接收设置:");
                    if ui.checkbox(&mut show_hex, "HEX").changed() && show_hex {
                        show_text = false; // 勾 HEX → 取消文本
                    }
                    if ui.checkbox(&mut show_text, "文本").changed() && show_text {
                        show_hex = false; // 勾文本 → 取消 HEX
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.color_view, "颜色");
                        ui.checkbox(&mut self.ascii_view, "字节查看");
                    });
                });
                self.hex_view = show_hex; // 文本 = !hex_view；两个都取消时退回文本

                // 接收区
                if self.color_view {
                    self.render_color_receive(ui);
                } else if self.ascii_view {
                    // ASCII 模式：逐字节解码，控制字符/高位字节转义为可见形式
                    self.render_ascii_receive(ui);
                } else {
                    // 普通文本模式：不折行 + 纵/横双向滚动 + 可选中 + 右键复制全部
                    show_plain_receive(ui, &self.receive_text, self.auto_scroll);
                }
            }

            if self.show_receive_panel && self.show_send_panel {
                ui.separator();
            }

            if self.show_send_panel {
                // 发送区
                ui.label("发送区:");

                let send_edit_response = ui.add(
                    egui::TextEdit::multiline(&mut self.send_text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4)
                        .font(egui::FontId::monospace(14.0))
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        )),
                );

                let enter_to_send = send_edit_response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

                let mut do_send = false;
                let mut do_clear = false;
                ui.horizontal(|ui| {
                    if ui.button("发送").clicked() {
                        do_send = true;
                    }
                    if ui.button("清空发送").clicked() {
                        do_clear = true;
                    }
                });
                if enter_to_send {
                    do_send = true;
                }
                if do_send {
                    self.send_data();
                }
                if do_clear {
                    self.send_text.clear();
                }
            }
        });
    }

    /// `eframe` 在退出前以及周期性自动保存时调用。
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        crate::config::save(storage, self);
    }
}
