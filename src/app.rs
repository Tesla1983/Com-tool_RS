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
//! 菜单栏与各类窗口在 [`crate::panels`]，实时文件日志在 [`crate::log_file`]。

use crate::chart::ChartData;
use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::log_file::LogSink;
use crate::panels::{send_format_radios, send_line_ending_radios};
use crate::receive::{AnsiDecoder, AnsiStyle, ColoredSpan, TimestampPrefix, show_plain_receive};
use crate::send::{
    DEFAULT_REPEAT_INTERVAL_MS, HistoryNav, SendFormat, SendHistory, SendLineEnding, build_payload,
    clamp_repeat_interval, on_first_line, on_last_line,
};
use crate::serial_impl::{DEFAULT_BAUD_RATE, SerialChannel, SerialConfig, scan_ports};
use eframe::egui;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

/// 图表面板是否默认显示。
///
/// 默认不显示：它会在中央区占掉一块高度，而"有没有曲线"取决于设备是否发数值流，
/// 让新用户一上来就对着一个空图表没有意义。从「视图 → 面板 → 实时图表」打开。
pub const DEFAULT_SHOW_CHART_PANEL: bool = false;

/// 图表每条曲线默认保留的样本数。
///
/// 2000 点足以看清一段趋势，单帧转换成本也在毫秒级；上限可在设置里调。
pub const DEFAULT_CHART_MAX_SAMPLES: usize = 2_000;

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
    /// 发送负载的解析方式（文本 / HEX）。注意它与接收侧的显示模式**无关**。
    pub send_format: SendFormat,
    /// 发送时追加的行尾（无 / LF / CRLF），在**字节层**追加。
    pub send_line_ending: SendLineEnding,
    /// 发送历史（最多 [`crate::send::MAX_SEND_HISTORY`] 条，落盘）
    pub send_history: SendHistory,
    /// 定时发送的间隔（毫秒，实际使用时夹到 10~600000）。
    ///
    /// 只存间隔不存"是否开启"：开关是 [`ComTool::repeat_send`]，它是运行时状态。
    pub repeat_interval_ms: u64,

    // 串口通信管理器（基于 ByteChannel trait，句柄不可序列化）
    #[serde(skip)]
    pub serial: Option<Box<dyn ByteChannel>>,

    // 串口事件通道（Bytes / Failed / Closed）
    #[serde(skip)]
    pub event_rx: Option<Receiver<ChannelEvent>>,

    // 上次自动重连尝试时间（用于节流，避免高频重试）
    #[serde(skip)]
    pub last_reconnect_attempt: Option<Instant>,

    // —— 发送侧的运行时状态（不落盘）——
    /// 是否正在定时发送。
    ///
    /// **不落盘**，而且断开连接时会被自动关掉（见 [`ComTool::stop_repeat_send`]）：
    /// "程序一启动就自动往外发数据"和"重连成功后突然开始喷数据"都不是用户要的行为，
    /// 与「实时文件日志不自动开始」是同一条原则（见 `crate::config` 模块注释）。
    #[serde(skip)]
    pub repeat_send: bool,
    /// 上一次定时发送的时刻。用 `Instant` 差值计时，**不是每帧累加**——后者在帧率
    /// 波动时会漂移（`repaint_interval_ms` 是可配的，帧率本来就不固定）。
    #[serde(skip)]
    pub last_repeat_send: Option<Instant>,
    /// ↑ / ↓ 在历史里停在哪儿（纯运行时状态，不落盘）
    #[serde(skip)]
    pub send_history_nav: HistoryNav,
    /// 上一帧发送输入框的光标位置（**字符**下标），用于判断 ↑ / ↓ 是否在首行 / 末行。
    ///
    /// 必须用上一帧的值：本帧的光标可能已经被 ↑ 自己挪走，用它判断会让"在第二行按
    /// ↑"立刻变成"取历史"。
    #[serde(skip)]
    pub send_cursor_index: Option<usize>,

    // 上次成功打开的串口（自动重连时优先用它，避免跳到 COM1）
    pub last_opened_port: Option<String>,

    /// HEX 视图：本批字节直接转十六进制文本。它与"文本"是同一个开关的两面
    /// （`hex_view == false` 即文本模式），并与 [`ComTool::ascii_view`] 互斥。
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
    /// 字节查看视图：渲染 `ascii_spans`。
    ///
    /// **与 [`ComTool::hex_view`] 互斥**——HEX 分支只写 `display_spans`，两个同时为真时
    /// 这个视图必然是空的。互斥由 [`ComTool::resolve_display_mode`] 统一裁决，界面（菜单
    /// 与接收设置行）与启动修复都走它，不要在别处另写一份判断。
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
    /// 收到数据时是否加时间戳前缀，以及按行还是按批（见 [`crate::receive::TimestampPrefix`]）
    pub timestamp_prefix: TimestampPrefix,
    /// 界面重绘间隔（毫秒），实际使用时会被夹到 16~1000
    pub repaint_interval_ms: u64,
    /// 发送后是否清空输入框
    pub clear_send_after_send: bool,
    /// 默认日志文件名模板（实时文件日志使用，`chrono` 负责展开时间占位）
    pub default_log_file_name: String,
    /// 实时文件日志的输出目录（空 = 用 [`crate::log_file::default_log_dir`]）
    pub log_dir: String,
    /// 接收区是否显示
    pub show_receive_panel: bool,
    /// 发送区是否显示
    pub show_send_panel: bool,
    /// 实时图表面板是否显示
    pub show_chart_panel: bool,
    /// 实时图表每条曲线保留的样本数上限
    pub chart_max_samples: usize,
    /// 实时图表是否自动跟随数据缩放（关掉后保留用户拖动 / 缩放后的视图）
    pub chart_auto_bounds: bool,

    // —— 运行时状态（不落盘，见 crate::config）——
    /// 正在运行的实时文件日志。`Some` 即表示「记录中」——不另设 bool 标志位，
    /// 两个字段表达同一件事迟早会不一致。
    #[serde(skip)]
    pub log: Option<LogSink>,
    /// 实时图表的样本缓冲。每次收到数据都会推进，且与接收日志一样是纯运行时
    /// 内容——落盘只会让退出时的保存变慢。
    #[serde(skip)]
    pub chart: ChartData,
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

        let now = Instant::now();
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
            self.push_receive_note("没有可导出的数据\n".to_string());
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
                self.push_receive_note(format!("已导出到: {}\n", path.display()));
            }
            Err(e) => {
                self.push_receive_note(format!("导出接收数据失败: {e}\n"));
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

        // 失败原因先攒在局部变量里：下面 `serial` 持有 `self.serial` 的可变借用，
        // 而落笔用的 `push_receive_note` 需要 `&mut self`，两个借用不能重叠。
        let mut failure: Option<String> = None;

        {
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
                failure = Some(format!("设置串口参数失败: {e}\n"));
            } else {
                let (event_tx, event_rx) = mpsc::channel::<ChannelEvent>();
                match serial.open(event_tx) {
                    Ok(()) => self.event_rx = Some(event_rx),
                    Err(e) => failure = Some(format!("打开串口失败: {e}\n")),
                }
            }
        }

        if let Some(msg) = failure {
            self.push_receive_note(msg);
            return;
        }

        self.connected = true;
        self.last_opened_port = Some(self.selected_port.clone());

        let open_msg = format!("已打开 {}\n", self.selected_port);
        self.display_spans.push(ColoredSpan {
            text: open_msg.clone(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
        self.push_receive_note(open_msg);
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

        // 主动关闭时也要停掉定时发送：否则重连之后它会自己接着发。
        self.stop_repeat_send();

        // 一次性追加一条彩色提示到彩色日志（不随渲染每帧重复）
        self.display_spans.push(ColoredSpan {
            text: "串口已关闭\n".to_string(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(205, 49, 49)), // 红
                bold: false,
            },
        });
        self.push_receive_note("串口已关闭\n".to_string());
    }

    /// 发送一次。UI 的「发送」按钮、Enter 键、定时发送都走这里——**只有这一条路**。
    ///
    /// 返回是否真的把字节交给了通道。UI 的调用点**不需要**看返回值（失败原因已经
    /// 写进接收区了），它存在的意义是让测试能断言"解析失败时确实什么都没发出去"。
    pub fn send_data(&mut self) -> bool {
        if !self.connected {
            return false;
        }

        let data = match build_payload(&self.send_text, self.send_format, self.send_line_ending) {
            Ok(bytes) => bytes,
            Err(reason) => {
                // 关键：**解析失败时不清空输入框**。这条提示的意思是"你输入的十六进制
                // 文本有问题"，用户必须能接着改它；顺手清空等于把用户刚写的内容删掉。
                // 与下面那条"发送失败"不同——那条是通道错误，输入本身是好的。
                self.push_receive_note(format!("发送失败: {reason}\n"));
                return false;
            }
        };

        // 输入为空且没有行尾时确实无字节可发：不算失败，也不该进历史。
        if data.is_empty() {
            return false;
        }

        // 历史记的是"**你尝试发过什么**"，所以写失败也记——用户下一步多半是
        // 连好线再发一遍，而不是重新敲一遍。
        self.send_history.record(&self.send_text);
        self.send_history_nav.reset();

        // 同 `open_port`：`serial` 的可变借用还活着时不能调 `push_receive_note`。
        let mut failure: Option<String> = None;
        if let Some(serial) = &mut self.serial
            && let Err(e) = serial.write(data)
        {
            failure = Some(format!("发送失败: {e}\n"));
        }
        let write_failed = failure.is_some();
        if let Some(msg) = failure {
            self.push_receive_note(msg);
        }

        // 「发送后清空」的语义是"**确实发出去了**才清空"。写失败（通道已断开、
        // 设备长时间不接受数据）时数据根本没出去，清掉等于让用户重打一遍——
        // 与上面 HEX 解析失败时不清空是同一条原则。
        if self.clear_send_after_send && !write_failed {
            self.send_text.clear();
        }

        !write_failed
    }

    /// 停掉定时发送。
    ///
    /// 断开连接（正常关闭、对端断开、通道报错）时都会调用它。**"自动开始往外发数据"
    /// 是必须避免的一类行为**：重连成功后突然开始喷数据，用户根本不知道谁在发。
    pub fn stop_repeat_send(&mut self) {
        self.repeat_send = false;
        self.last_repeat_send = None;
    }

    /// 定时发送的节拍，每帧调用一次。
    ///
    /// 计时基准是 [`Instant`] 差值而不是"每帧累加一个间隔"——后者在帧率波动时会
    /// 漂移，而重绘间隔本来就是可配的（16~1000ms）。
    ///
    /// 另外一条：**只在开启时才请求重绘**。若在这里无条件
    /// `request_repaint_after(10ms)`，设置里的「重绘间隔」就被架空了——用户把它调到
    /// 500ms 省电，界面却仍在 10ms 空转。
    pub fn tick_repeat_send(&mut self, ctx: &egui::Context) {
        if !self.repeat_send || !self.connected {
            return;
        }

        let interval = Duration::from_millis(clamp_repeat_interval(self.repeat_interval_ms));
        let now = Instant::now();

        let Some(last) = self.last_repeat_send else {
            // 刚勾上：这一帧只负责起步计时，**不立刻发一包**。用户在按下勾选框的
            // 那一刻不该有任何数据出去。
            self.last_repeat_send = Some(now);
            ctx.request_repaint_after(interval);
            return;
        };

        let elapsed = now.duration_since(last);
        if elapsed >= interval {
            self.last_repeat_send = Some(now);
            self.send_data();
            ctx.request_repaint_after(interval);
        } else {
            ctx.request_repaint_after(interval - elapsed);
        }
    }
}

impl ComTool {
    /// 当前可见的面板数量（侧边栏 / 接收区 / 图表面板 / 发送区）。
    ///
    /// 「至少保留一个可见」这条不变量在两个地方用到：视图菜单里决定勾选框是否可点，
    /// 以及 [`crate::config::restore_runtime_state`] 里修正被手改坏的存档。
    /// **新增面板时必须同步这里、视图菜单、以及那处兜底修正**——三处缺一，
    /// 不变量就有一个入口可以绕过。
    pub fn visible_panel_count(&self) -> usize {
        usize::from(self.sidebar_open)
            + usize::from(self.show_receive_panel)
            + usize::from(self.show_chart_panel)
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
            send_format: SendFormat::default(),
            send_line_ending: SendLineEnding::default(),
            send_history: SendHistory::default(),
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            serial: None,
            event_rx: None,
            last_reconnect_attempt: None,
            repeat_send: false,
            last_repeat_send: None,
            send_history_nav: HistoryNav::default(),
            send_cursor_index: None,
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
            timestamp_prefix: TimestampPrefix::default(),
            repaint_interval_ms: DEFAULT_REPAINT_INTERVAL_MS,
            clear_send_after_send: true,
            default_log_file_name: DEFAULT_LOG_FILE_NAME.to_string(),
            log_dir: String::new(),
            show_receive_panel: true,
            show_send_panel: true,
            show_chart_panel: DEFAULT_SHOW_CHART_PANEL,
            chart_max_samples: DEFAULT_CHART_MAX_SAMPLES,
            chart_auto_bounds: true,
            log: None,
            chart: ChartData::default(),
        }
    }
}

impl eframe::App for ComTool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 检查串口接收数据
        self.receive_data();

        // 写日志线程可能已经报错退出：每帧检查一次，不能让界面继续显示「记录中」
        self.poll_log_events();

        // 串口意外断开时自动重连（节流 500ms）
        self.auto_reconnect();

        // 定时发送的节拍（只在自己请求重绘的前提下工作，见该方法注释）
        self.tick_repeat_send(ui.ctx());

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
        if menu.log_toggle_requested {
            self.toggle_logging();
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
        if action.log_toggle_requested {
            self.toggle_logging();
        }
        if action.pick_log_dir_requested {
            self.pick_log_dir();
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
                // 「字节查看」与「HEX 接收」也互斥（见 `resolve_display_mode`）。这里必须
                // 分别记下"哪个是刚被勾上的"，交给那一个函数统一裁决，而不是就地写
                // `if hex { ascii = false; }`——规则两处各写一遍，迟早有一处忘了改。
                let mut hex_just_enabled = false;
                let mut ascii_just_enabled = false;
                ui.horizontal(|ui| {
                    ui.label("接收设置:");
                    if ui.checkbox(&mut show_hex, "HEX").changed() && show_hex {
                        show_text = false; // 勾 HEX → 取消文本
                        hex_just_enabled = true;
                    }
                    if ui.checkbox(&mut show_text, "文本").changed() && show_text {
                        show_hex = false; // 勾文本 → 取消 HEX
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.color_view, "颜色");
                        if ui.checkbox(&mut self.ascii_view, "字节查看").changed()
                            && self.ascii_view
                        {
                            ascii_just_enabled = true;
                        }
                    });
                });
                self.hex_view = show_hex; // 文本 = !hex_view；两个都取消时退回文本
                self.resolve_display_mode(hex_just_enabled, ascii_just_enabled);

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

            // 中央区里这几块是纵向堆叠的，分隔线只在"上面有东西、下面也有东西"
            // 时才有意义；图表夹在接收区与发送区之间。
            if self.show_receive_panel && (self.show_chart_panel || self.show_send_panel) {
                ui.separator();
            }

            self.render_chart_panel(ui);

            if self.show_chart_panel && self.show_send_panel {
                ui.separator();
            }

            if self.show_send_panel {
                // 发送区
                ui.horizontal(|ui| {
                    ui.label("发送区:");
                    ui.separator();
                    ui.label("格式");
                    send_format_radios(ui, &mut self.send_format);
                    ui.separator();
                    ui.label("行尾");
                    send_line_ending_radios(ui, &mut self.send_line_ending);
                    ui.separator();
                    if ui.checkbox(&mut self.repeat_send, "定时发送").changed() && self.repeat_send
                    {
                        // 勾上的那一刻只起步计时，不立刻发一包（见 `tick_repeat_send`）
                        self.last_repeat_send = None;
                    }
                    if self.repeat_send {
                        ui.add(
                            egui::DragValue::new(&mut self.repeat_interval_ms)
                                .range(
                                    crate::send::MIN_REPEAT_INTERVAL_MS
                                        ..=crate::send::MAX_REPEAT_INTERVAL_MS,
                                )
                                .speed(10.0)
                                .suffix(" ms"),
                        );
                    }
                    ui.separator();
                    self.send_history_menu(ui);
                });

                let send_edit_id = egui::Id::new("send_input");

                // 焦点与箭头键都必须在绘制 `TextEdit` **之前**读：TextEdit 会消费
                // 导航键事件，等它画完再读 `key_pressed` 有可能已经拿不到了。
                let had_focus = ui.ctx().memory(|m| m.has_focus(send_edit_id));
                let (up_pressed, down_pressed) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp) && !i.modifiers.shift,
                        i.key_pressed(egui::Key::ArrowDown) && !i.modifiers.shift,
                    )
                });

                let out = egui::TextEdit::multiline(&mut self.send_text)
                    .id(send_edit_id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(4)
                    .font(egui::FontId::monospace(14.0))
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        egui::Key::Enter,
                    ))
                    .show(ui);
                let send_has_focus = out.response.response.has_focus();

                // 用**上一帧**的光标位置判断首行 / 末行：本帧的位置可能已经被 ↑ 自己
                // 挪走，用它判断会让"在第二行按 ↑"当场变成"取历史"。
                let cursor = self.send_cursor_index.unwrap_or(0);
                let mut recalled = false;
                if had_focus {
                    let next = if up_pressed && on_first_line(&self.send_text, cursor) {
                        // 注意 `send_history_nav` 与 `send_history` / `send_text` 是三个
                        // 互不重叠的字段，借用检查器允许这样同时借。
                        self.send_history_nav
                            .older(&self.send_history, &self.send_text)
                    } else if down_pressed && on_last_line(&self.send_text, cursor) {
                        self.send_history_nav.newer(&self.send_history)
                    } else {
                        None
                    };

                    if let Some(text) = next {
                        self.send_text = text;
                        recalled = true;
                        // 光标移到末尾：否则下一次 ↑ 会因为光标还停在旧位置而重复命中
                        // 同一条历史（shell 里的 ↑ 也是这个行为）。
                        let mut state = out.state.clone();
                        let end = egui::text::CCursor::new(self.send_text.chars().count());
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), send_edit_id);
                    }
                }

                // 用户自己改了内容 → 退出浏览态，否则下一次 ↑ 会接着半截历史往下走。
                // `recalled` 那一帧例外：本次改动是我们自己写进去的。
                if out.response.response.changed() && !recalled {
                    self.send_history_nav.reset();
                }

                // 记录本帧光标位置供下一帧使用；刚取过历史就按"光标已到末尾"记，
                // 免得下一帧拿一个越界下标去判断。
                // `CCursor::index` 是 `CharIndex` 这个 newtype（不是裸 usize），
                // 取 `.0` 拿字符下标；`send.rs` 里的首行/末行判定也是按字符算的。
                self.send_cursor_index = if recalled {
                    Some(self.send_text.chars().count())
                } else {
                    out.cursor_range.map(|r| r.primary.index.0)
                };

                // 实时校验：HEX 写错了要当场看见，而不是点了发送才发现。
                // 超长输入跳过——发送框里正常不会有这么大的文本，而每帧解析一遍
                // 几十 KB 的十六进制字符串是白费。
                // 全部挤在同一行里：这一行的高度固定，浏览历史时不会让下面的按钮跳位。
                const PREVIEW_MAX_LEN: usize = 16 * 1024;
                ui.horizontal(|ui| {
                    if self.send_text.len() > PREVIEW_MAX_LEN {
                        ui.weak("输入过长，已停止实时校验");
                    } else {
                        match build_payload(
                            &self.send_text,
                            self.send_format,
                            self.send_line_ending,
                        ) {
                            Ok(bytes) => {
                                ui.weak(format!("将发送 {} 字节", bytes.len()));
                            }
                            Err(reason) => {
                                let error_color = ui.visuals().error_fg_color;
                                ui.colored_label(error_color, format!("HEX 解析失败: {reason}"));
                            }
                        }
                    }
                    // 让用户知道现在按 ↑ / ↓ 是在翻历史而不是在移光标
                    if self.send_history_nav.is_active() {
                        ui.separator();
                        ui.weak("历史浏览中：↑ 更旧 / ↓ 更新");
                    }
                });

                let enter_to_send = send_has_focus
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
                    if self.repeat_send && !self.connected {
                        ui.weak("未连接：连上之后才会开始定时发送");
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
                    self.send_history_nav.reset();
                }
            }
        });
    }

    /// `eframe` 在退出前以及周期性自动保存时调用。
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        crate::config::save(storage, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communicationtrait::fake::{FakeChannel, FakeHandle};

    /// 一台"已连接、通道是替身"的应用。
    ///
    /// 走 `open_port()` 而不是手工拼状态：这样 `configure` → `open` → 建立事件通道
    /// 这一段真实路径也被覆盖，而它是不需要串口硬件就能跑的。
    fn connected_app() -> (ComTool, FakeHandle) {
        let (channel, handle) = FakeChannel::new();
        let mut app = ComTool {
            serial: Some(Box::new(channel)),
            selected_port: "COM_TEST".to_string(),
            ..ComTool::default()
        };
        app.open_port();
        assert!(app.connected, "替身通道应当能打开成功");
        assert!(app.event_rx.is_some(), "打开后必须有事件通道");
        (app, handle)
    }

    #[test]
    fn text_send_writes_utf8_and_appends_line_ending() {
        let (mut app, handle) = connected_app();
        app.send_text = "你好".to_string();
        app.send_line_ending = SendLineEnding::Crlf;

        assert!(app.send_data());

        assert_eq!(handle.written_bytes(), "你好\r\n".as_bytes());
    }

    #[test]
    fn hex_send_parses_input_before_writing() {
        let (mut app, handle) = connected_app();
        app.send_format = SendFormat::Hex;
        app.send_text = "41 42,43".to_string();

        assert!(app.send_data());

        assert_eq!(handle.written(), vec![vec![0x41, 0x42, 0x43]]);
    }

    /// 这条是 F1 的核心约定：HEX 写错时**不发、也不清空输入框**。
    ///
    /// 静默丢弃半个字节会让用户去怀疑设备；顺手清空则等于把用户刚敲的内容删掉。
    #[test]
    fn invalid_hex_neither_writes_nor_clears_the_input() {
        let (mut app, handle) = connected_app();
        app.send_format = SendFormat::Hex;
        app.send_text = "4".to_string();
        app.clear_send_after_send = true;

        assert!(!app.send_data(), "解析失败不能报告为已发送");

        assert_eq!(handle.write_count(), 0, "解析失败时一个字节都不该出去");
        assert_eq!(app.send_text, "4", "输入框必须原样保留，用户还要改它");
        assert!(
            app.receive_text.contains("发送失败"),
            "必须给出可见的原因，而不是静默无反应：{}",
            app.receive_text
        );
        assert!(app.send_history.is_empty(), "没发出去的东西不该进历史");
    }

    #[test]
    fn empty_input_without_line_ending_sends_nothing() {
        let (mut app, handle) = connected_app();
        app.send_text = String::new();

        assert!(!app.send_data());
        assert_eq!(handle.write_count(), 0);

        // 但"只发一个换行"是合法用法
        app.send_line_ending = SendLineEnding::Lf;
        assert!(app.send_data());
        assert_eq!(handle.written_bytes(), b"\n");
    }

    #[test]
    fn disconnected_app_writes_nothing() {
        let (mut app, handle) = connected_app();
        app.send_text = "ping".to_string();
        app.connected = false;

        assert!(!app.send_data());
        assert_eq!(handle.write_count(), 0);
    }

    #[test]
    fn successful_send_is_recorded_in_history() {
        let (mut app, _handle) = connected_app();
        app.send_text = "AT+RST".to_string();
        app.send_data();

        assert_eq!(app.send_history.entries(), ["AT+RST"]);

        // 定时发送同一句不会把历史塞满（连续重复只记一条）
        app.send_data();
        app.send_data();
        assert_eq!(app.send_history.len(), 1);
    }

    #[test]
    fn clear_after_send_is_honoured_and_resets_history_browsing() {
        let (mut app, _handle) = connected_app();
        app.send_text = "AT".to_string();
        app.send_history.record("旧的");
        app.send_history_nav.older(&app.send_history, "");
        assert!(app.send_history_nav.is_active());

        app.clear_send_after_send = true;
        app.send_data();

        assert!(app.send_text.is_empty());
        assert!(
            !app.send_history_nav.is_active(),
            "发出去之后历史浏览位置必须复位"
        );
    }

    #[test]
    fn write_failure_is_reported_and_does_not_clear_input() {
        let (mut app, handle) = connected_app();
        handle.reject_writes(true);
        app.send_text = "AT".to_string();
        app.clear_send_after_send = true;

        app.send_data();

        assert!(
            app.receive_text.contains("发送失败"),
            "通道错误也要有可见提示：{}",
            app.receive_text
        );
        assert_eq!(app.send_text, "AT", "写失败时不该把输入清掉");
    }

    // ---------- 定时发送 ----------

    /// 勾上定时发送的那一刻**不能立刻发包**；到点之后每过一个间隔发一次。
    #[test]
    fn repeat_send_waits_one_interval_before_the_first_send() {
        let (mut app, handle) = connected_app();
        app.send_text = "tick".to_string();
        app.repeat_interval_ms = 10;
        app.repeat_send = true;

        let ctx = egui::Context::default();

        // 第一帧只起步计时
        app.tick_repeat_send(&ctx);
        assert_eq!(handle.write_count(), 0, "勾上的瞬间不该有数据出去");

        // 还没到点
        app.tick_repeat_send(&ctx);
        assert_eq!(handle.write_count(), 0);

        std::thread::sleep(Duration::from_millis(15));
        app.tick_repeat_send(&ctx);
        assert_eq!(handle.write_count(), 1, "到点之后应当发一条");
        assert_eq!(handle.written_bytes(), b"tick");
    }

    #[test]
    fn repeat_send_does_nothing_when_disconnected_or_disabled() {
        let (mut app, handle) = connected_app();
        app.send_text = "tick".to_string();
        app.repeat_interval_ms = 10;
        let ctx = egui::Context::default();

        // 没勾：一帧都不该有动作
        app.tick_repeat_send(&ctx);
        assert_eq!(handle.write_count(), 0);

        // 勾了但断开：同样不动
        app.repeat_send = true;
        app.connected = false;
        std::thread::sleep(Duration::from_millis(15));
        app.tick_repeat_send(&ctx);
        app.tick_repeat_send(&ctx);
        assert_eq!(handle.write_count(), 0);
    }

    /// 通道报错 → 自动停掉定时发送。
    ///
    /// "重连成功后突然开始往外喷数据"是必须避免的一类行为：用户根本不知道是谁在发。
    #[test]
    fn channel_failure_stops_repeat_send() {
        let (mut app, handle) = connected_app();
        app.send_text = "tick".to_string();
        app.repeat_send = true;
        app.last_repeat_send = Some(Instant::now());

        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();

        assert!(!app.connected, "报错后必须回到未连接");
        assert!(!app.repeat_send, "定时发送必须一起停掉");
        assert!(app.last_repeat_send.is_none(), "计时基准也要清掉");
        assert!(
            app.receive_text.contains("设备已拔出"),
            "错误原因要能被看见：{}",
            app.receive_text
        );
    }

    #[test]
    fn closed_event_stops_repeat_send_too() {
        let (mut app, handle) = connected_app();
        app.repeat_send = true;

        handle.emit(ChannelEvent::Closed);
        app.receive_data();

        assert!(!app.connected);
        assert!(!app.repeat_send);
        assert!(app.receive_text.contains("连接已关闭"));
    }

    /// 主动关闭同样要停掉，否则重连之后它会自己接着发。
    #[test]
    fn closing_the_port_stops_repeat_send() {
        let (mut app, _handle) = connected_app();
        app.repeat_send = true;

        app.close_port();

        assert!(!app.connected);
        assert!(!app.repeat_send);
    }
}
