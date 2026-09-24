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
use crate::net_impl::{
    DEFAULT_REMOTE_HOST, DEFAULT_REMOTE_PORT, NetConfig, TcpChannel, UdpChannel,
};
use crate::panels::{send_format_radios, send_line_ending_radios};
use crate::receive::{
    AnsiDecoder, AnsiStyle, ColoredSpan, TextEncoding, TimestampPrefix, show_plain_receive,
};
use crate::send::{
    DEFAULT_REPEAT_INTERVAL_MS, HistoryNav, Payload, SendFormat, SendHistory, SendLineEnding,
    SendPresets, build_payload, clamp_repeat_interval, on_first_line, on_last_line,
};
use crate::serial_impl::{DEFAULT_BAUD_RATE, SerialChannel, SerialConfig, clamp_baud, scan_ports};
use crate::strings::fill;
use eframe::egui;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// 界面语言。
///
/// 它只决定一件事：`Language::strings()` 取哪张文案表（见 [`crate::strings`]）。
/// 枚举本身不持有任何文案，所以加一种语言 = 在 `strings.rs` 的宏里加一列。
///
/// # 顺序即"表列顺序"
///
/// [`Language::ALL`] 的顺序**必须与 `strings.rs` 里 `langs:` 声明的列顺序一致**
/// ——`Language::strings()` 按下标把枚举映射到表。两处顺序错位会让界面显示另一种
/// 语言，所以 `strings::tests::langs_table_matches_language_order` 把它们钉在一起。
///
/// # 序列化兼容
///
/// `#[derive(Serialize, Deserialize)]` 用的是**变体名**，所以新语言只能**追加**到
/// 末尾，不能插到 `Zh` / `En` 中间——否则既有存档里的 `"En"` 会读到错误的变体。
/// （`Zh` / `En` 必须保持在前两位。）
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Language {
    /// 简体中文（默认）。
    Zh,
    /// English。**保持在第二位**：既有存档里的 `"En"` 依赖这个位置。
    En,
    // ── 第一批（风险最低，见 I18N_TARGET_LANGUAGES.md §0）──
    /// 繁体中文。
    ZhHant,
    /// 日本語。
    Ja,
    /// Español.
    Es,
    /// Italiano.
    It,
    /// Português.
    Pt,
    /// Bahasa Indonesia.
    Id,
    // ── 第二批 ──
    /// Français.
    Fr,
    /// Deutsch.
    De,
    /// 한국어（韩语）。
    Ko,
    /// Tiếng Việt（越南语）。
    Vi,
}

impl Language {
    /// 全部语言，顺序 = 文案表的列顺序 = 设置窗口下拉里的顺序。
    ///
    /// 下拉框与守卫测试都从这里取，不再各处手写一遍（早先设置窗口是手写两行的，
    /// 加语言时容易漏）。
    pub const ALL: &'static [Language] = &[
        Self::Zh,
        Self::En,
        Self::ZhHant,
        Self::Ja,
        Self::Es,
        Self::It,
        Self::Pt,
        Self::Id,
        Self::Fr,
        Self::De,
        Self::Ko,
        Self::Vi,
    ];

    /// 语言在设置窗口下拉里**显示的名字**（自名，endonym）。
    ///
    /// 用**该语言自己的写法**而不是当前界面语言：界面已经切成英文时，只想切回中文的人
    /// 要能认出"中文"那一档。这与 `strings.rs` 里 `ui_sources_do_not_hardcode_chinese`
    /// 的白名单是一回事——那些自名字符串是**唯一**允许硬编码在 UI 源码里的非 ASCII。
    ///
    /// 这些自名**同时是字体覆盖的证人**：日本语/한국어/Tiếng Việt 里的假名、谚文、
    /// 越南语带调字母必须在字体链里有字形，否则下拉框自己就先变成方框。
    pub const fn endonym(self) -> &'static str {
        match self {
            Self::Zh => "中文（简体）",
            Self::En => "English",
            Self::ZhHant => "中文（繁體）",
            Self::Ja => "日本語",
            Self::Es => "Español",
            Self::It => "Italiano",
            Self::Pt => "Português",
            Self::Id => "Bahasa Indonesia",
            Self::Fr => "Français",
            Self::De => "Deutsch",
            Self::Ko => "한국어",
            Self::Vi => "Tiếng Việt",
        }
    }
}

/// 连接类型（F4 起）。
///
/// 它决定三件事，缺一不可：
///
/// 1. **建哪种通道**——见 [`ComTool::open_connection`]；
/// 2. **界面上显示哪些参数控件**——串口是「端口 / 波特率 / 数据位 / 停止位 / 校验 /
///    流控」，网络是「地址 / 端口」；串口的那些字段在网络模式下**必须整段隐藏**，
///    否则用户会对着一个"波特率 115200"去连 TCP，然后问为什么没生效；
/// 3. **状态词**——串口与 TCP 说「已连接」，UDP 只能说「**已绑定**」（它没有连接，
///    说"已连接"会让用户以为对端在线、以为掉线会被发现）。
///
/// `#[default]` 落在 `Serial`：既有用户的存档里没有这个字段，
/// 反序列化时走默认值必须是"和以前一样"。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ConnectionKind {
    #[default]
    Serial,
    TcpClient,
    Udp,
}

impl ConnectionKind {
    /// 需要串口那些参数（端口名 / 波特率 / 数据位 / 停止位 / 校验 / 流控）吗。
    ///
    /// 抽成一个方法而不是各处写 `kind == ConnectionKind::Serial`：将来加一种
    /// "也是串口类"的传输（比如串口转网络）时，判定规则只改这一处。
    pub fn uses_serial_settings(self) -> bool {
        matches!(self, Self::Serial)
    }

    /// 需要网络参数（地址 / 端口）吗。
    pub fn uses_net_settings(self) -> bool {
        !self.uses_serial_settings()
    }
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

/// 全局 UI 字体大小档位。
///
/// **存的是档位、不是像素值**：像素值是实现细节（egui 每个 `TextStyle` 各有自己的
/// 基准字号，See `default_text_styles()`：Small 9 / Body 13 / Button 13 / Heading 18 /
/// Monospace 13），档位才是用户能理解、也稳定的东西。改基准字号或改档位含义时，
/// 存档里的 "Large" 仍然读得出来。
///
/// `#[default]` 落在 `Normal`（系数 1.0）：既有存档没有这个字段，反序列化必须
/// "和以前一模一样"，否则升级一次程序字就变大了。
///
/// 换算成系数后统一乘在**每个** `TextStyle` 的 `FontId.size` 上（见
/// [`ComTool::apply_font_scale`]），所以是"全局"缩放：标题、正文、按钮、等宽一起动，
/// 不会出现"标题变大了、说明文字还是小字"的错位。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum UiFontScale {
    /// 小（0.85×）
    Small,
    /// 标准（1.0×，与改动前完全一致）
    #[default]
    Normal,
    /// 大（1.15×）
    Large,
    /// 特大（1.3×）
    ExtraLarge,
}

impl UiFontScale {
    /// 乘到各 `TextStyle` 基准字号上的系数。
    ///
    /// 四档刻意取"对称但不夸张"的步长：`Small` 0.85 与 `Large` 1.15 相对 `Normal`
    /// 各差 15%，`ExtraLarge` 1.3 是再上一级。为什么不给更大的上限（比如 2.0）：
    /// 界面里有多处 `add_sized([LABEL_WIDTH, 20.0], ..)` 这类固定像素的标签列与
    /// 行高，字号放大过头会先撞上这些硬编码高度而挤成一团；真要更大的字，得先把
    /// 那些尺寸也变成随档位缩放的量，那是另一件事。
    pub const fn factor(self) -> f32 {
        match self {
            Self::Small => 0.85,
            Self::Normal => 1.0,
            Self::Large => 1.15,
            Self::ExtraLarge => 1.3,
        }
    }

    /// 全部档位，供设置窗口按固定顺序列出。
    pub const ALL: [Self; 4] = [Self::Small, Self::Normal, Self::Large, Self::ExtraLarge];
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
    //
    // 选了下拉里的「自定义」时，这个字段仍然是**真值**——界面上的输入框读写的
    // 就是它。`baud_custom`（下一个字段）只记录"下拉选中的是哪一档"，不是标记值，
    // 两者都不会把哨兵值混进这里提交给驱动。
    pub baud_rate: u32,

    /// 下拉框当前选中的是「自定义」那一档吗。
    ///
    /// **为什么不从"`baud_rate` 是否在 `BAUD_RATES` 里"推导**：那样用户选自定义后
    /// 手输一个列表里**恰好有**的值（比如 9600），下拉会自己跳回 9600 那一档，
    /// 输入框消失——他看着自己刚输的数字把输入框弄没了，只会以为程序坏了。
    /// 用户"选的是哪一档"和"那个数字是多少"是两件事，就存两个字段。
    pub baud_custom: bool,

    // 当前串口参数
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,

    // —— 连接类型与网络参数（F4 起；全部落盘）——
    /// 用哪种传输：串口 / TCP 客户端 / UDP。
    pub connection_kind: ConnectionKind,
    /// 网络目标地址（IP 字面量或域名）
    pub net_remote_host: String,
    /// 网络目标端口
    pub net_remote_port: u16,
    /// UDP 本地绑定端口（0 = 系统分配；TCP 不用）
    pub net_local_port: u16,

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
    /// 文本编码（接收解码与发送编码共用）。
    ///
    /// 一个偏好管两个方向：用户不会希望"收到的中文按 GBK 解出来了，发出去的却是
    /// UTF-8"，那等于手动做两次设置才自洽。
    pub text_encoding: TextEncoding,
    /// 发送历史（最多 [`crate::send::MAX_SEND_HISTORY`] 条，落盘）
    pub send_history: SendHistory,
    /// 常用数据（最多 [`crate::send::MAX_SEND_PRESETS`] 条，落盘）。
    ///
    /// 与发送历史是两回事：历史是"自动记下发过什么"，常用数据是"我挑出来、想一键
    /// 重发的几条"。所以两者各自一份，互不影响。
    pub send_presets: SendPresets,
    /// 定时发送的间隔（毫秒，实际使用时夹到 10~600000）。
    ///
    /// 只存间隔不存"是否开启"：开关是 [`ComTool::repeat_send`]，它是运行时状态。
    pub repeat_interval_ms: u64,

    // 通信通道（F4 起可能是串口 / TCP / UDP；基于 ByteChannel trait，句柄不可序列化）
    //
    // 字段名是 `channel` 而不是 `serial`：它从 F4 起真的可能装一个 TCP 或 UDP
    // 通道，叫 `serial` 会让读代码的人以为这里只可能是串口。
    #[serde(skip)]
    pub channel: Option<Box<dyn ByteChannel>>,

    /// [`ComTool::channel`] 里那个对象是**为哪种连接类型造的**。
    ///
    /// 它存在的原因只有一个：[`ComTool::open_connection`] 要判断"能不能就地复用现成的
    /// 通道对象"。两种情况必须区分开——
    ///
    /// * **类型没变**（串口断线重连、测试注入的替身通道）→ 复用：前者是"同一个句柄
    ///   再 `open` 一次"，后者若不复用就会当场造一个真串口，测试也就测不到替身了；
    /// * **类型变了**（用户把「串口」切成「TCP」）→ 必须拆掉重建：旧对象是另一种传输，
    ///   它身上的连接先得断掉，否则会留下一个没人管的后台线程。
    ///
    /// 与 `channel` 同生共死：[`ComTool::teardown_channel`] 会把两者一起清空，
    /// 所以"`channel` 是 `Some` 而 `channel_kind` 是 `None`"这种半截状态不存在。
    #[serde(skip)]
    pub channel_kind: Option<ConnectionKind>,

    // 串口事件通道（Bytes / Failed / Closed）
    #[serde(skip)]
    pub event_rx: Option<Receiver<ChannelEvent>>,

    // 上次自动重连尝试时间（用于节流，避免高频重试）
    #[serde(skip)]
    pub last_reconnect_attempt: Option<Instant>,

    // —— 串口控制线的请求状态（N1，不落盘）——
    /// 我们**请求**把 RTS 拉成什么电平（`true` = 断言 / active）。
    ///
    /// 这是"请求"而不是"测量"：[`serialport`] 只提供写、**不提供读**，所以界面上的
    /// 勾选反映的是"我们最后要求过什么"，不是引脚此刻的真实电平。
    ///
    /// **不落盘，且默认 `false`**：Windows 上 serialport-rs 打开端口时显式把 DTR 与
    /// RTS 都设为 Disable（`serialport/src/windows/dcb.rs` 里
    /// `set_fDtrControl(DtrControl::Disable)`，以及 `FlowControl::None` 分支的
    /// `set_fRtsControl(RtsControl::Disable)`），也就是"刚打开时两根都是低"。
    /// 落盘会把"上次会话的请求"当成"本次的初始状态"，那是假话。
    #[serde(skip)]
    pub rts: bool,
    /// 我们**请求**把 DTR 拉成什么电平。含义与 [`ComTool::rts`] 相同。
    #[serde(skip)]
    pub dtr: bool,

    /// 窗口上一帧是否处于聚焦状态（N6 用来识别"重新获得焦点"这一瞬间）。
    ///
    /// 初值 `true`：[`ComTool::default`] 已经扫过一次端口列表，置 `true` 可以省掉
    /// 启动第一帧那次多余的扫描。
    #[serde(skip)]
    pub was_focused: bool,

    /// 本次连接已经**成功写出**的字节数（不落盘）。
    ///
    /// 只统计确实交给通道且返回 `Ok` 的那些——与「发送后清空输入框」同一条原则：
    /// "确实发出去了"才算数。排队时计数会显示出一个并不成立的"已发送"。
    #[serde(skip)]
    pub tx_bytes: u64,

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
    /// 全局 UI 字体大小档位（见 [`UiFontScale`]）
    pub ui_font_scale: UiFontScale,
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
    /// 手动「刷新」按钮：重扫端口列表，并在**当前所选端口已经消失**时改选第一个。
    ///
    /// 只在用户明确点了刷新时做那一步"改选"：它是在帮用户收拾一个已经失效的选择。
    pub fn refresh_ports(&mut self) {
        self.ports = scan_ports();

        if !self.ports.contains(&self.selected_port) {
            self.selected_port = self.ports.first().cloned().unwrap_or_default();
        }
    }

    /// 静默重扫端口列表：**只更新列表，绝不改 `selected_port`**（N6）。
    ///
    /// 与 [`ComTool::refresh_ports`] 分成两个函数、而不是加一个 `bool` 参数：调用
    /// 时机的语义完全不同。这个版本由"窗口重新获得焦点"自动触发，没有人在旁边看着，
    /// 所以它连"所选端口消失了"都不能顺手处理——用户可能正对着那个刚被拔掉的端口
    /// 看它的最后几条数据，或者（更糟）**正连着它**，此时改选会让界面与真实通道
    /// 对不上。`auto_reconnect` 里那句"只刷新列表供 UI 显示"就是同一条规则。
    pub fn rescan_ports(&mut self) {
        self.ports = scan_ports();
    }

    /// 记录窗口焦点变化，并在**重新获得焦点**的那一刻静默重扫端口列表（N6）。
    ///
    /// 规则本体放在这里而不是内联进 `ui()`：`ui()` 要收一个 `eframe::Frame`，无头
    /// 测试构造不出来，内联的写法就等于"这条规则没有测试"。抽出来之后
    /// `regaining_focus_rescans_ports_without_touching_the_selection` 能直接驱动它。
    ///
    /// 刻意**不做定时轮询**：一是白耗 CPU，二是枚举串口要问一遍系统，频率高了本身
    /// 就是负担。"重新获得焦点"是零空转的等价信号——插拔设备这个动作几乎必然伴随
    /// 用户切回窗口。用 [`ComTool::rescan_ports`] 而不是 [`ComTool::refresh_ports`]，
    /// 理由见前者的注释。
    ///
    /// 判重靠 `was_focused`：没有它就会**每帧**枚举一次串口，而 `focused` 在绝大多数
    /// 帧里都是 true。
    pub fn note_focus(&mut self, focused: bool) {
        if focused && !self.was_focused {
            self.rescan_ports();
        }
        self.was_focused = focused;
    }

    /// 断线后自动重连。**只对串口生效**（F4 起）。
    ///
    /// 不用于 TCP / UDP 的理由写在 `FUTURE_ROADMAP.md` §5：这个函数依赖
    /// `last_opened_port`（"上次那个端口"），那是串口语义；对 TCP 而言，
    /// 对端没启动时反复重试只会产生一批半死不活的 socket，而用户完全看不出
    /// 它在重试（界面已经是"未连接"）。
    pub fn auto_reconnect(&mut self) {
        // 只有串口曾经打开过、现在断开时才尝试自动重连
        if self.connection_kind != ConnectionKind::Serial {
            return;
        }
        if self.connected || self.channel.is_none() {
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

        self.open_connection();
    }

    pub fn export_receive_text(&mut self) {
        if self.receive_text.is_empty() {
            self.push_receive_note("没有可导出的数据\n".to_string());
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .set_title(self.language.strings().export_dialog_title)
            .add_filter(self.language.strings().export_text_filter, &["txt"])
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

    /// 建立连接。UI 的「连接」按钮与自动重连都走这里——**只有这一条路**。
    ///
    /// 按 [`ComTool::connection_kind`] 决定建哪种通道（串口 / TCP 客户端 / UDP）。
    /// 三者都实现 [`ByteChannel`]，所以之后的接收链路、发送链路、图表、实时日志
    /// 全部共用，不因为换了传输而分叉。
    ///
    /// # 失败时不会置 `connected`
    ///
    /// `open()` 返回 `Err` 就直接报错退出，界面保持"未连接"。这一点对网络通道
    /// 是**硬要求**：`TcpChannel` / `UdpChannel` 的 `open()` 会当场把 connect /
    /// bind 做完再返回，所以这里的 `Ok` 就是"确实连上了"。
    ///
    /// 串口通道的 `open()` 只负责拉起后台线程，真正的打开失败要等 `Failed` 事件
    /// 回来才知道——那是既有的 `connected` 假阳性缺陷（见 `FEATURE_GAP_AND_PLAN.md`
    /// §4），本次未动：修它要引入"正在打开"这个中间状态，属独立课题。
    pub fn open_connection(&mut self) {
        // 前置条件不满足（没选端口 / 没填目标地址）时**什么都不做**——尤其不能
        // 乐观置位 `connected`。
        if !self.connection_ready() {
            return;
        }

        // 通道对象：类型没变就**就地复用**，类型变了才拆掉重建。复用不是省一次
        // 分配，而是两条硬需求：断线重连是"同一个句柄再 `open` 一次"，而测试里
        // 注入的替身通道一旦被拆掉就会换成真串口。理由详见 `channel_kind` 的注释。
        if self.channel.is_none() || self.channel_kind != Some(self.connection_kind) {
            self.teardown_channel();
        }
        if self.channel.is_none() {
            let channel = self.new_channel();
            self.channel = Some(channel);
            self.channel_kind = Some(self.connection_kind);
        }

        // 配置对象与提示措辞都随类型变化，但"配置 → 打开 → 接管事件通道"这三步
        // 只有下面这一处实现。若按传输类型各写一遍，抄漏任何一步都是一整类难查的
        // 故障（漏了事件通道 = "连上了收不到数据"；漏了 configure = "参数怎么改都没用"）。
        let (settings, action, noun, target) = self.channel_descriptor();

        // 事件通道必须在 `open` **之前**建好：`open` 一旦返回，后台线程就可能
        // 开始投事件，而事件的接收端只能建立一次（漏了这一步表现为"连上了但
        // 收不到数据"，很难查）。
        let (event_tx, event_rx) = mpsc::channel::<ChannelEvent>();

        // 失败原因先攒在局部变量里：下面 `channel` 持有 `self.channel` 的可变借用，
        // 而落笔用的 `push_receive_note` 需要 `&mut self`，两个借用不能重叠。
        let mut failure: Option<String> = None;
        {
            let channel = self.channel.as_mut().expect("上一段已保证通道存在");
            if let Err(e) = channel.configure(&*settings) {
                failure = Some(format!("设置{noun}参数失败: {e}\n"));
            } else if let Err(e) = channel.open(event_tx) {
                failure = Some(format!("{action}失败: {e}\n"));
            }
        }
        if let Some(msg) = failure {
            self.push_receive_note(msg);
            return;
        }

        self.event_rx = Some(event_rx);
        self.connected = true;

        // 刚打开时两根控制线都是低的（理由见 `rts` 字段的注释：serialport-rs 在
        // Windows 上打开端口时显式把 DTR / RTS 都置为 Disable）。**这里只是把勾选态
        // 复位成事实，不向端口发任何指令**——"一打开就去拉 DTR/RTS"会让 ESP32 那类
        // 自动下载电路在用户还没反应过来时就把板子复位/带进下载模式，而他会以为
        // 是程序坏了。要动这两根线，必须由用户显式点。
        self.rts = false;
        self.dtr = false;

        if self.connection_kind.uses_serial_settings() {
            // `last_opened_port` 只对串口有意义（自动重连的"回到哪个端口"）
            self.last_opened_port = Some(self.selected_port.clone());
        }

        // 成功的措辞按传输类型分叉：UDP 说"绑定"而不是"连接"——它没有连接。
        let open_msg = match self.connection_kind {
            ConnectionKind::Serial => format!("已打开 {target}\n"),
            ConnectionKind::TcpClient => format!("已连接 {target}\n"),
            ConnectionKind::Udp => format!("UDP 已绑定，目标 {target}\n"),
        };
        self.display_spans.push(ColoredSpan {
            text: open_msg.clone(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
        self.push_receive_note(open_msg);
    }

    /// 建连接的前置条件是否满足。
    ///
    /// 唯一的判定入口：`open_connection` 与侧边栏的「连接」按钮都靠它决定"这一下
    /// 点下去有没有意义"，两处各写一遍必然出现"按钮亮着但点了没反应"。
    pub fn connection_ready(&self) -> bool {
        if self.connection_kind.uses_net_settings() {
            !self.net_peer_label().is_empty()
        } else {
            !self.selected_port.is_empty()
        }
    }

    /// 造一个**崭新**的、还没打开的通道对象。
    fn new_channel(&self) -> Box<dyn ByteChannel> {
        match self.connection_kind {
            ConnectionKind::Serial => Box::new(SerialChannel::new()),
            ConnectionKind::TcpClient => Box::new(TcpChannel::new()),
            ConnectionKind::Udp => Box::new(UdpChannel::new()),
        }
    }

    /// 当前连接类型的配置对象，外加提示行要用的措辞。
    ///
    /// 返回 `(配置, 动作名, 名词, 目标名)`：
    ///
    /// * 动作名用于"打开串口失败" / "连接失败" / "绑定失败"；
    /// * 名词用于"设置串口参数失败" / "设置TCP参数失败"；
    /// * 目标名用于失败信息与成功提示（`COM3` / `192.168.1.10:1234`）。
    ///
    /// 调用前必须先过 [`ComTool::connection_ready`]，因此这里不再校验前置条件——
    /// 两个地方各判一次，迟早出现"这里放过了、那里拦住了"。
    fn channel_descriptor(&self) -> (Box<dyn std::any::Any>, &'static str, &'static str, String) {
        match self.connection_kind {
            ConnectionKind::Serial => (
                Box::new(SerialConfig {
                    port: self.selected_port.clone(),
                    // 夹一下再下发（N5）：界面上的输入框已经限制了范围，这里防的是
                    // **手改过的存档**。与 `repaint_interval_ms` 在 `ui()` 里的做法
                    // 是同一条约定——界面上不合法、但存档里存在过的值要能自愈。
                    baud_rate: clamp_baud(self.baud_rate),
                    data_bits: self.data_bits,
                    stop_bits: self.stop_bits,
                    parity: self.parity,
                    flow_control: self.flow_control,
                }),
                "打开串口",
                self.transport_noun(),
                self.selected_port.clone(),
            ),
            ConnectionKind::TcpClient => (
                Box::new(self.net_config()),
                "连接",
                self.transport_noun(),
                self.net_peer_label(),
            ),
            ConnectionKind::Udp => (
                Box::new(self.net_config()),
                "绑定",
                self.transport_noun(),
                self.net_peer_label(),
            ),
        }
    }

    /// 当前传输类型在提示行里的名词（`串口` / `TCP` / `UDP`）。
    ///
    /// 提示行本身**不翻译**（它们会写进实时日志与导出文件，见 [`crate::strings`]
    /// 的模块文档），但名词必须跟着传输类型走——否则 TCP 掉线会写一句"串口连接已关闭"，
    /// 日志一旦发给别人看就是误导。
    pub fn transport_noun(&self) -> &'static str {
        match self.connection_kind {
            ConnectionKind::Serial => "串口",
            ConnectionKind::TcpClient => "TCP",
            ConnectionKind::Udp => "UDP",
        }
    }

    /// 当前 UI 上的网络参数。
    pub fn net_config(&self) -> NetConfig {
        NetConfig {
            remote_host: self.net_remote_host.clone(),
            remote_port: self.net_remote_port,
            local_port: self.net_local_port,
        }
    }

    /// 网络对端的显示名（`host:port`），用于提示行与状态回显。
    ///
    /// 单独抽出来是因为它出现在三处（连接成功的提示、侧边栏状态、UDP 的绑定
    /// 说明），三处各拼一遍迟早会出现"一处带端口一处不带"。
    pub fn net_peer_label(&self) -> String {
        let host = self.net_remote_host.trim();
        if host.is_empty() || self.net_remote_port == 0 {
            return String::new();
        }
        format!("{}:{}", host, self.net_remote_port)
    }

    /// 拆掉当前通道：关掉后台线程、清掉句柄与连接标志、停掉定时发送。
    ///
    /// **不写任何提示行**，也不动 `last_opened_port`。这两个"不做"都是刻意的：
    ///
    /// * 提示行由调用方决定——[`ComTool::open_connection`] 开头也要拆（换传输类型
    ///   时），那里写一句"连接已关闭"会让人以为刚才的连接断了；
    /// * `last_opened_port` 是"用户主动关闭过就不要再自动重连"的依据，
    ///   只有 [`ComTool::close_connection`] 才该清它。
    fn teardown_channel(&mut self) {
        if let Some(mut channel) = self.channel.take() {
            let _ = channel.close();
        }

        // 与 `channel` 一起清空：`channel_kind` 描述的是"那个对象是为哪种类型造的"，
        // 对象没了它就没有意义。留着它还会让下一次 `open_connection` 误判成"可复用"。
        self.channel_kind = None;
        self.event_rx = None;
        self.connected = false;

        // 计数与线状态都是"本次连接"的运行时事实，通道没了就没有意义。
        // （接收字节数不在这里归零：它由 `receive_text.len()` 推导，清空接收区时归零。）
        self.tx_bytes = 0;
        self.rts = false;
        self.dtr = false;

        // 通道没了，定时发送必须一起停：否则重连之后它会自己接着发，
        // 而用户并不知道是谁在发。
        self.stop_repeat_send();
    }

    /// 用户主动断开：拆掉通道、清掉"上次连接"的记忆，并在接收区留一条提示。
    pub fn close_connection(&mut self) {
        let had_channel = self.channel.is_some() || self.connected;
        self.teardown_channel();

        // 主动关闭后不该再自动重连，所以把"上次打开的端口"也清掉
        // （断线走的是另一条路：`receive_data` 只置 `connected = false`）。
        self.last_opened_port = None;
        self.last_reconnect_attempt = None;

        if !had_channel {
            return;
        }

        // 提示行按传输类型换名词；串口那两个字的原文保持不变（F4 之前就是"串口已关闭"）。
        let note = format!("{}已关闭\n", self.transport_noun());

        // 一次性追加一条彩色提示到彩色日志（不随渲染每帧重复）
        self.display_spans.push(ColoredSpan {
            text: note.clone(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(205, 49, 49)), // 红
                bold: false,
            },
        });
        self.push_receive_note(note);
    }

    /// 发送一次输入框里的内容。UI 的「发送」按钮、Enter 键、定时发送都走这里——
    /// **只有这一条路**。
    ///
    /// 返回是否真的把字节交给了通道。UI 的调用点**不需要**看返回值（失败原因已经
    /// 写进接收区了），它存在的意义是让测试能断言"解析失败时确实什么都没发出去"。
    pub fn send_data(&mut self) -> bool {
        if !self.connected {
            return false;
        }

        let payload = match self.payload_for(&self.send_text) {
            Ok(payload) => payload,
            Err(reason) => {
                // 关键：**解析失败时不清空输入框**。这条提示的意思是"你输入的十六进制
                // 文本有问题"，用户必须能接着改它；顺手清空等于把用户刚写的内容删掉。
                // 与下面那条"发送失败"不同——那条是通道错误，输入本身是好的。
                self.push_receive_note(format!("发送失败: {reason}\n"));
                return false;
            }
        };

        // 输入为空且没有行尾时确实无字节可发：不算失败，也不该进历史。
        if payload.bytes.is_empty() {
            return false;
        }

        // 历史记的是"**你尝试发过什么**"，所以写失败也记——用户下一步多半是
        // 连好线再发一遍，而不是重新敲一遍。
        self.send_history.record(&self.send_text);
        self.send_history_nav.reset();

        let sent = self.transmit(payload);

        // 「发送后清空」的语义是"**确实发出去了**才清空"。写失败（通道已断开、
        // 设备长时间不接受数据）时数据根本没出去，清掉等于让用户重打一遍——
        // 与上面解析失败时不清空是同一条原则。
        if sent && self.clear_send_after_send {
            self.send_text.clear();
        }

        sent
    }

    /// 发送一条常用数据（N2）。
    ///
    /// 与输入框走**完全同一条**通路（同一个 [`ComTool::payload_for`]、同一个
    /// [`ComTool::transmit`]）：编码、行尾、失败提示、字节计数都只有一处实现。
    /// 若在按钮回调里自己拼一遍 `write`，"发送字节计数"与"写失败提示"两处都会
    /// 各漏一次，而且是静默地漏。
    ///
    /// 三处刻意的"不做"：
    ///
    /// * **不进发送历史**：历史记的是"我敲过什么"，点按钮不产生输入；
    /// * **不动输入框**：发一条常用数据不等于把用户正在写的内容替换掉；
    /// * **不受「发送后清空」影响**：那条设置说的是输入框。
    pub fn send_preset(&mut self, index: usize) -> bool {
        if !self.connected {
            return false;
        }
        let Some(text) = self.send_presets.entries().get(index).cloned() else {
            return false;
        };

        match self.payload_for(&text) {
            // 空负载（内容为空且行尾为「无」）不算失败，也没什么可发的。
            Ok(payload) if payload.bytes.is_empty() => false,
            Ok(payload) => self.transmit(payload),
            Err(reason) => {
                self.push_receive_note(format!("发送失败: {reason}\n"));
                false
            }
        }
    }

    /// 把输入框里现在的内容存成一条常用数据（N2）。
    ///
    /// 空内容与已有内容都会被拒绝（界面上的按钮也会先置灰，这里再挡一次——
    /// "按钮可达"这件事不该只由界面保证）。
    pub fn save_current_as_preset(&mut self) -> bool {
        let text = self.send_text.clone();
        self.send_presets.add(&text)
    }

    /// 按当前格式 / 行尾 / 编码把一段文本组装成待发字节。
    ///
    /// 输入框与常用数据共用它。**唯一入口**：编码与行尾的规则只在这里出现一次
    /// （见 [`crate::send::build_payload`] 的文档）。
    fn payload_for(&self, text: &str) -> Result<Payload, String> {
        build_payload(
            text,
            self.send_format,
            self.send_line_ending,
            self.text_encoding,
        )
    }

    /// 把已经组装好的负载交给通道，并在**成功之后**记账。
    ///
    /// 三条发送入口（输入框 / 常用数据 / 定时发送——最后一条走的仍是输入框那条）
    /// 在这里收口：只管"送出去、报错、计数"，不碰编码与行尾。
    fn transmit(&mut self, payload: Payload) -> bool {
        // 有字符在当前编码里表示不了（编码器写了 HTML 字符引用之类的替代形式）：
        // 照样发，但必须明说——静默换掉用户要发的字符比直接报错更难排查。
        if payload.had_unmappable {
            self.push_receive_note(format!(
                "警告: 有字符在 {} 里无法表示，已被替换\n",
                self.text_encoding.label()
            ));
        }

        let len = payload.bytes.len();

        // 同 `open_connection`：`channel` 的可变借用还活着时不能调 `push_receive_note`。
        let mut failure: Option<String> = None;
        let mut wrote = false;
        match &mut self.channel {
            Some(channel) => match channel.write(payload.bytes) {
                Ok(()) => wrote = true,
                Err(e) => failure = Some(format!("发送失败: {e}\n")),
            },
            // `connected` 为真时不该走到这里；真走到了也不能算作"发出去了"，
            // 否则计数会凭空增长、输入框会被清掉。
            None => failure = Some("发送失败: 通道不存在\n".to_string()),
        }
        if let Some(msg) = failure {
            self.push_receive_note(msg);
            return false;
        }

        // 只在返回 `Ok` 之后累加（N4）：排队时计数会显示出一个并不成立的"已发送"。
        if wrote {
            self.tx_bytes += len as u64;
        }
        wrote
    }

    /// 手动改变串口控制线的电平（N1）。`None` 表示那一根不动。
    ///
    /// 勾选态**先按用户点的改**再投给通道：它是"我们的请求"，不是对引脚的测量
    /// （`serialport` 只提供写、不提供读，见 [`ComTool::rts`]）。
    ///
    /// 两条失败路径都不会把界面判成掉线：这里投递失败只写一条提示；通道侧执行失败
    /// 则回一条 [`ChannelEvent::Notice`]。
    pub fn apply_line_state(&mut self, rts: Option<bool>, dtr: Option<bool>) {
        if let Some(level) = rts {
            self.rts = level;
        }
        if let Some(level) = dtr {
            self.dtr = level;
        }

        let mut failure: Option<String> = None;
        match &mut self.channel {
            Some(channel) => {
                if let Err(e) = channel.set_line_state(rts, dtr) {
                    failure = Some(format!("设置 RTS/DTR 失败: {e}\n"));
                }
            }
            None => failure = Some("设置 RTS/DTR 失败: 通道未打开\n".to_string()),
        }
        if let Some(msg) = failure {
            self.push_receive_note(msg);
        }
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

    /// 把全局字体档位下发到 egui。
    ///
    /// **为什么每帧都算一遍基准值、而不是"上一帧的 13.0 乘 1.15"**：后者会逐帧连乘，
    /// 字体指数级膨胀。所以这里每次都从一个**固定基准表**出发，乘上当前系数，
    /// 与上一帧的值无任何关系——同一档位算一万次，结果恒等（幂等）。
    ///
    /// 基准值取自 egui 的 `default_text_styles()`（Small 9 / Body 13 / Button 13 /
    /// Heading 18 / Monospace 13）。**刻意抄成常量而不是调 `ctx.style()` 去读现有值**：
    /// 读现有值就把上面的连乘问题带回来了。代价是 egui 若哪天改了默认字号，这里会落后
    /// ——由 `font_scale_baseline_matches_egui_defaults` 这条测试盯着（它直接调
    /// `egui::Style::default().text_styles` 比对），一改就红。
    ///
    /// 用 `all_styles_mut` 而不是 `style_mut`：前者同时改深色与浅色两套 `Style`
    /// （egui 内部按主题各存一份）。只改当前主题那一份的话，用户在深色下调好字号、
    /// 切成浅色就变回去了。
    pub fn apply_font_scale(&self, ctx: &egui::Context) {
        let factor = self.ui_font_scale.factor();
        ctx.all_styles_mut(|style| {
            for (text_style, base_size) in FONT_SIZE_BASELINE {
                // `entry` 而不是覆盖整个 `text_styles`：表里若将来多出 egui 新增、
                // 而我们还没登记的 TextStyle，它至少保留 egui 的默认值而不是消失。
                //
                // `clone()`：`TextStyle` 不是 `Copy`（它有 `Name(String)` 变体），
                // 即使这里只用得上那五个内建变体。
                style
                    .text_styles
                    .entry(text_style.clone())
                    .or_default()
                    .size = base_size * factor;
            }
        });
    }
}

/// 各 `TextStyle` 的**基准字号**（系数为 1.0 时的值）。
///
/// 与 egui `default_text_styles()` 一一对应，见 [`ComTool::apply_font_scale`] 的说明。
const FONT_SIZE_BASELINE: &[(egui::TextStyle, f32)] = &[
    (egui::TextStyle::Small, 9.0),
    (egui::TextStyle::Body, 13.0),
    (egui::TextStyle::Button, 13.0),
    (egui::TextStyle::Heading, 18.0),
    (egui::TextStyle::Monospace, 13.0),
];

impl Default for ComTool {
    fn default() -> Self {
        let ports = scan_ports();

        let selected_port = ports.first().cloned().unwrap_or_default();

        Self {
            ports,
            selected_port,
            baud_rate: DEFAULT_BAUD_RATE,
            baud_custom: false,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            connection_kind: ConnectionKind::default(),
            net_remote_host: DEFAULT_REMOTE_HOST.to_string(),
            net_remote_port: DEFAULT_REMOTE_PORT,
            net_local_port: 0,
            connected: false,
            receive_text: String::new(),
            send_text: String::new(),
            send_format: SendFormat::default(),
            send_line_ending: SendLineEnding::default(),
            text_encoding: TextEncoding::default(),
            send_history: SendHistory::default(),
            send_presets: SendPresets::default(),
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            channel: None,
            channel_kind: None,
            event_rx: None,
            last_reconnect_attempt: None,
            rts: false,
            dtr: false,
            was_focused: true,
            tx_bytes: 0,
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
            ui_font_scale: UiFontScale::default(),
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

impl ComTool {
    /// 发送区下面的「常用数据」一行（N2）：存为常用 / 一键重发 / 右键删除。
    ///
    /// 从 [`ComTool::ui`] 里抽出来的，与 [`ComTool::render_chart_panel`]、
    /// [`ComTool::render_color_receive`] 同一套写法。抽出来不是为了好看，是为了
    /// **可测**：`ui()` 要收一个 `eframe::Frame`，无头测试构造不出那个参数，写在
    /// 里面的控件等于一次也不会被 `cargo test` 画到；这个方法只收 `&mut Ui`，
    /// `egui::__run_test_ui` 就能把它整条走一遍（见
    /// `panels::tests::preset_row_renders_without_panic`）。
    ///
    /// 与调用方没有隐式约定：进出都只靠 `self`。
    pub(crate) fn render_send_presets(&mut self, ui: &mut egui::Ui) {
        let s = self.language.strings();

        // 条目先拷成 `Vec<String>` 再画。闭包里既要读条目、又要触发发送与删除
        // （后两个都是 `&mut self`），直接借 `self.send_presets` 会与那几个调用
        // 撞借用。列表上限 16 条，这一次拷贝的开销可以忽略。
        //
        // 动作攒在局部变量里、画完再执行：与发送区「发送 / 清空」用
        // `do_send` / `do_clear` 的写法一致——闭包内不直接改状态，改由闭包外统一
        // 派发，出了问题只看一处。
        let preset_entries: Vec<String> = self.send_presets.entries().to_vec();
        let presets_empty = self.send_presets.is_empty();
        let can_save_preset = self.send_presets.can_add(&self.send_text);
        let preset_full = self.send_presets.is_full();
        let preset_duplicate = self.send_presets.contains(&self.send_text);
        let connected = self.connected;
        let mut preset_to_send: Option<usize> = None;
        let mut preset_to_remove: Option<usize> = None;
        let mut do_save_preset = false;

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(s.presets_header);
            ui.separator();

            let save_button = ui.add_enabled(can_save_preset, egui::Button::new(s.save_preset));
            if save_button.clicked() {
                do_save_preset = true;
            } else if !can_save_preset {
                // 置灰的原因**必须**说出来，否则用户只看到"按钮亮不起来 / 点了没
                // 反应"，只会以为程序坏了。三种拒绝理由各说各的，判定见
                // [`SendPresets::can_add`]——按钮的置灰条件与 `add` 的实际规则是同
                // 一处，不会出现"按钮亮着、点了没反应"。
                save_button.on_disabled_hover_text(if preset_full {
                    fill(s.preset_full, &[&crate::send::MAX_SEND_PRESETS])
                } else if preset_duplicate {
                    s.preset_duplicate.to_string()
                } else {
                    s.preset_input_empty.to_string()
                });
            }

            if !connected {
                // 与发送按钮同一条约定：按钮不置灰（点一下什么都不发生），理由用
                // 一行弱提示挂在旁边。
                ui.weak(s.preset_needs_connection);
            }

            ui.separator();

            if presets_empty {
                ui.weak(s.preset_empty);
            } else {
                for (index, entry) in preset_entries.iter().enumerate() {
                    // 按钮上只放**短的**标签，完整内容挂在悬停里：一条几十字节的
                    // 指令直接把按钮撑爆，把整行挤成一条。
                    let button = ui.button(crate::send::preset_label(
                        entry,
                        crate::send::PRESET_LABEL_CHARS,
                    ));
                    // 左键发、右键删。不做右键菜单：只有"删"一个动作，为它弹一层
                    // 菜单要用户多点一次，还没有任何好处。
                    if button.clicked() {
                        preset_to_send = Some(index);
                    } else if button.secondary_clicked() {
                        preset_to_remove = Some(index);
                    }
                    button.on_hover_text(entry);
                }
                ui.weak(s.preset_hint);
            }
        });

        if do_save_preset {
            self.save_current_as_preset();
        }
        if let Some(index) = preset_to_remove {
            self.send_presets.remove(index);
        }
        if let Some(index) = preset_to_send {
            self.send_preset(index);
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

        // 窗口重新获得焦点时静默重扫端口列表（N6）：插上 USB 转串口、切回窗口就能
        // 看见新端口，不必手点「刷新」。规则本体在 `note_focus` 里，这里只负责把
        // 当前的焦点态交给它。
        let focused = ui.ctx().input(|i| i.focused);
        self.note_focus(focused);

        // 定时发送的节拍（只在自己请求重绘的前提下工作，见该方法注释）
        self.tick_repeat_send(ui.ctx());

        // 主题偏好只在与当前生效值不同时下发一次
        self.apply_theme(ui.ctx());

        // 字体档位每帧重建（幂等，不会连乘）——它没有"便宜的比较"可做：
        // 要判断"是否已生效"就得遍历所有 TextStyle 逐个比对，跟直接写一遍差不多。
        self.apply_font_scale(ui.ctx());

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

        // ③ 底部状态栏
        // **必须在 CentralPanel 之前**：egui 的面板先画先占位，后画的只能拿到
        // 剩下的空间。放在中央区之后画，状态栏就只横跨中央区剩余宽度，
        // 左右两边露出缝隙，看起来像没对齐。
        self.show_status_bar(ui);

        // ④ 中央区
        let s = self.language.strings();
        egui::CentralPanel::default().show(ui, |ui| {
            // 设置 / 信息两个窗口跟着中央区一起绘制（主界面的常驻部分）
            self.show_settings_window(ui);
            self.show_info_window(ui);

            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.sidebar_open, s.sidebar_toggle);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(s.settings_button).clicked() {
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
                    ui.label(s.receive_settings);
                    if ui.checkbox(&mut show_hex, s.hex).changed() && show_hex {
                        show_text = false; // 勾 HEX → 取消文本
                        hex_just_enabled = true;
                    }
                    if ui.checkbox(&mut show_text, s.text).changed() && show_text {
                        show_hex = false; // 勾文本 → 取消 HEX
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.color_view, s.color);
                        if ui.checkbox(&mut self.ascii_view, s.byte_view).changed()
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
                    show_plain_receive(ui, &self.receive_text, self.auto_scroll, s.copy_all_log);
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
                    ui.label(s.send_area);
                    ui.separator();
                    ui.label(s.format);
                    send_format_radios(ui, &mut self.send_format, s);
                    ui.separator();
                    ui.label(s.line_ending);
                    send_line_ending_radios(ui, &mut self.send_line_ending, s);
                    ui.separator();
                    if ui.checkbox(&mut self.repeat_send, s.repeat_send).changed()
                        && self.repeat_send
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
                    // 字号跟随全局字体档位：写死 14.0 的话，用户把字体调大后
                    // 发送框仍是小字，与接收区不一致（两者都是等宽内容，本该同号）。
                    .font(crate::receive::monospace_font_id(
                        ui,
                        crate::receive::RECEIVE_MONOSPACE_BASE_SIZE,
                    ))
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
                        ui.weak(s.preview_too_long);
                    } else {
                        match build_payload(
                            &self.send_text,
                            self.send_format,
                            self.send_line_ending,
                            self.text_encoding,
                        ) {
                            Ok(payload) => {
                                ui.weak(fill(s.will_send_bytes, &[&payload.bytes.len()]));
                                if payload.had_unmappable {
                                    let color = ui.visuals().error_fg_color;
                                    ui.colored_label(
                                        color,
                                        fill(
                                            s.unmappable_in_encoding,
                                            &[&self.text_encoding.label()],
                                        ),
                                    );
                                }
                            }
                            Err(reason) => {
                                let error_color = ui.visuals().error_fg_color;
                                ui.colored_label(error_color, fill(s.hex_parse_failed, &[&reason]));
                            }
                        }
                    }
                    // 让用户知道现在按 ↑ / ↓ 是在翻历史而不是在移光标
                    if self.send_history_nav.is_active() {
                        ui.separator();
                        ui.weak(s.history_browsing);
                    }
                });

                let enter_to_send = send_has_focus
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

                let mut do_send = false;
                let mut do_clear = false;
                ui.horizontal(|ui| {
                    if ui.button(s.send_title).clicked() {
                        do_send = true;
                    }
                    if ui.button(s.clear_send_button).clicked() {
                        do_clear = true;
                    }
                    if self.repeat_send && !self.connected {
                        ui.weak(s.repeat_needs_connection);
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

                self.render_send_presets(ui);
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
    /// 走 [`ComTool::open_connection`] 而不是手工拼状态：这样 `configure` → `open` →
    /// 建立事件通道这一段真实路径也被覆盖，而它是不需要串口硬件就能跑的。
    ///
    /// 替身能被复用的前提是 `channel_kind` 与 `connection_kind` 对得上——两个字段
    /// 一起写，正是 `open_connection` 里那条复用规则的显式表达（见 `channel_kind`
    /// 的注释：类型一致才复用，否则会当场造一个真串口）。
    fn connected_app() -> (ComTool, FakeHandle) {
        connected_app_as(ConnectionKind::Serial)
    }

    /// 同上，但连接类型可指定（F4）。
    ///
    /// 替身通道对任何 `configure` 都点头，所以同一个替身可以扮串口、TCP 或 UDP——
    /// 这正是"换了传输而接收/发送链路不变"的可测形式。
    fn connected_app_as(kind: ConnectionKind) -> (ComTool, FakeHandle) {
        let (channel, handle) = FakeChannel::new();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(kind),
            connection_kind: kind,
            selected_port: "COM_TEST".to_string(),
            net_remote_host: "127.0.0.1".to_string(),
            net_remote_port: 1234,
            ..ComTool::default()
        };
        app.open_connection();
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

    // ---------- 文本编码（F2） ----------

    /// 发送侧确实按所选编码编码：同一段中文在 GBK 档下发出的是 GBK 字节。
    #[test]
    fn send_uses_the_selected_encoding() {
        let (mut app, handle) = connected_app();
        app.text_encoding = TextEncoding::Gbk;
        app.send_text = "你好".to_string();

        assert!(app.send_data());

        assert_eq!(handle.written_bytes(), vec![0xC4, 0xE3, 0xBA, 0xC3]);
    }

    /// 有字符在当前编码里表示不了时，**照样发出去但必须警告**——静默替换掉用户
    /// 要发的字符，比直接报错更难排查。
    #[test]
    fn unmappable_character_produces_a_visible_warning() {
        let (mut app, handle) = connected_app();
        app.text_encoding = TextEncoding::Gbk;
        app.send_text = "\u{1D11E}".to_string();

        assert!(app.send_data());

        assert_eq!(handle.write_count(), 1, "能发就还是发");
        assert!(
            app.receive_text.contains("无法表示"),
            "必须给出可见警告：{}",
            app.receive_text
        );
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
    fn closing_the_connection_stops_repeat_send() {
        let (mut app, _handle) = connected_app();
        app.repeat_send = true;

        app.close_connection();

        assert!(!app.connected);
        assert!(!app.repeat_send);
    }

    // ---------- 连接类型分叉（F4） ----------

    #[test]
    fn only_serial_uses_the_serial_settings() {
        assert!(ConnectionKind::Serial.uses_serial_settings());
        assert!(!ConnectionKind::Serial.uses_net_settings());
        for kind in [ConnectionKind::TcpClient, ConnectionKind::Udp] {
            assert!(!kind.uses_serial_settings(), "{kind:?} 不该显示串口参数");
            assert!(kind.uses_net_settings());
        }
    }

    /// 提示行里的名词必须跟着传输类型走。
    ///
    /// 这些提示会写进实时日志与导出文件，TCP 掉线却写一句"串口连接已关闭"
    /// 是把日志发给别人看时的误导。
    #[test]
    fn transport_noun_follows_the_connection_kind() {
        let mut app = ComTool {
            connection_kind: ConnectionKind::Serial,
            ..ComTool::default()
        };

        assert_eq!(app.transport_noun(), "串口");
        app.connection_kind = ConnectionKind::TcpClient;
        assert_eq!(app.transport_noun(), "TCP");
        app.connection_kind = ConnectionKind::Udp;
        assert_eq!(app.transport_noun(), "UDP");
    }

    #[test]
    fn connection_ready_tracks_the_selected_transport() {
        let mut app = ComTool {
            selected_port: String::new(),
            ..ComTool::default()
        };

        // 串口语义：没选端口就不该允许连接
        app.connection_kind = ConnectionKind::Serial;
        assert!(!app.connection_ready());
        app.selected_port = "COM7".to_string();
        assert!(app.connection_ready());

        // 网络语义：默认的 127.0.0.1:1234 就是完整目标
        app.connection_kind = ConnectionKind::Udp;
        assert!(app.connection_ready());
        app.net_remote_host = "   ".to_string();
        assert!(!app.connection_ready(), "地址只有空白等于没填");
        app.net_remote_host = "127.0.0.1".to_string();
        app.net_remote_port = 0;
        assert!(!app.connection_ready(), "端口 0 不是合法目标");
    }

    /// 前置条件不满足时**什么都不做**——不建通道、不置 `connected`、也不留提示。
    ///
    /// 这条比"能连上"更要紧：乐观置位会让界面显示"已连接"，而实际上一个字节
    /// 都发不出去，用户只会去怀疑设备。
    #[test]
    fn net_connection_without_a_target_does_nothing() {
        let mut app = ComTool {
            connection_kind: ConnectionKind::TcpClient,
            net_remote_host: String::new(),
            ..ComTool::default()
        };

        app.open_connection();

        assert!(!app.connected);
        assert!(app.channel.is_none(), "参数不全时连通道对象都不该造");
        assert!(app.receive_text.is_empty(), "什么都没发生，就不该留下提示");
    }

    /// 通道对象**同类型复用**：串口掉线后重新连接走的就是这条路。
    #[test]
    fn a_dropped_connection_reuses_the_same_channel_object() {
        let (mut app, handle) = connected_app();

        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();

        assert!(!app.connected);
        assert!(
            app.channel.is_some(),
            "掉线不拆通道对象：重连要在同一个句柄上再 open 一次"
        );
        assert_eq!(app.channel_kind, Some(ConnectionKind::Serial));

        app.open_connection();

        assert!(app.connected, "复用同一条通道应当能重新打开");
        assert_eq!(handle.write_count(), 0, "重新打开不该顺手发数据");
    }

    /// 换传输类型必须**拆掉重建**：旧对象是另一种传输，留着它就会留下一个
    /// 没人管的后台线程。
    ///
    /// 这里让 TCP 去连一个必然没人监听的端口，顺带验证"连接失败不置 `connected`"。
    #[test]
    fn switching_connection_kind_rebuilds_the_channel() {
        let (mut app, handle) = connected_app();
        app.close_connection();
        assert!(app.channel_kind.is_none(), "拆掉通道后类型标记必须一起清空");

        app.connection_kind = ConnectionKind::TcpClient;
        app.net_remote_host = "127.0.0.1".to_string();
        app.net_remote_port = 1; // 保留端口，不会有服务在听

        app.open_connection();

        assert!(!app.connected, "连不上就不能报告为已连接");
        assert_eq!(app.channel_kind, Some(ConnectionKind::TcpClient));
        assert!(
            app.receive_text.contains("连接失败"),
            "失败原因要能被看见：{}",
            app.receive_text
        );
        assert_eq!(
            handle.write_count(),
            0,
            "替身通道已被换掉，一个字节都不该经过它"
        );
    }

    /// 用户主动关闭的提示词按类型分叉，但**串口的原文不能被改动**。
    #[test]
    fn user_close_note_uses_the_transport_noun() {
        let (mut app, _handle) = connected_app();
        app.close_connection();
        assert!(
            app.receive_text.contains("串口已关闭"),
            "{}",
            app.receive_text
        );
        assert!(app.channel_kind.is_none());

        let (mut app, _handle) = connected_app_as(ConnectionKind::TcpClient);
        app.close_connection();
        assert!(
            app.receive_text.contains("TCP已关闭"),
            "{}",
            app.receive_text
        );
    }

    /// 对端主动关闭时，提示行说的是**当前传输**在关闭，而不是串口。
    #[test]
    fn closed_event_names_the_transport() {
        let (mut app, handle) = connected_app_as(ConnectionKind::TcpClient);

        handle.emit(ChannelEvent::Closed);
        app.receive_data();

        assert!(!app.connected);
        assert!(
            app.receive_text.contains("TCP连接已关闭"),
            "{}",
            app.receive_text
        );
    }

    /// UDP 的措辞是"绑定"而不是"连接"：它没有连接，也不会发现对端掉线。
    #[test]
    fn udp_open_reports_bound_and_never_claims_connected() {
        let (app, _handle) = connected_app_as(ConnectionKind::Udp);

        assert!(app.connected);
        assert!(
            app.receive_text.contains("UDP 已绑定"),
            "UDP 只能用「绑定」这个词：{}",
            app.receive_text
        );
    }

    /// 自动重连依赖 `last_opened_port`，是串口语义——TCP / UDP 必须原样返回。
    ///
    /// 若哪天有人把这个判断去掉，对端没启动时会攒出一批半死不活的 socket，
    /// 而界面上完全看不出来它在重试。
    #[test]
    fn auto_reconnect_is_serial_only() {
        let (mut app, handle) = connected_app_as(ConnectionKind::TcpClient);

        handle.emit(ChannelEvent::Closed);
        app.receive_data();
        assert!(!app.connected);

        app.auto_reconnect();

        assert!(!app.connected, "TCP 不该自动重连");
    }

    // —— N1：RTS / DTR ——

    /// 打开端口**不碰**控制线。
    ///
    /// 这条比"能不能设电平"更重要：上电瞬间把 DTR 拉高会把相当多的开发板直接拽进
    /// 复位，而用户只是想打开端口看看。`serialport` 在 Windows 上打开时把两根线都
    /// 置为 Disable，我们也不补任何命令，所以这里的断言是"一条 `set_line_state` 都
    /// 没发生过"，而不是"设成了某个电平"。
    #[test]
    fn opening_a_port_never_touches_the_control_lines() {
        let (app, handle) = connected_app();

        assert!(!app.rts && !app.dtr, "打开端口后勾选态必须是未勾选");
        assert!(
            handle.line_state_calls().is_empty(),
            "打开端口不该向通道投递任何线状态命令"
        );
    }

    /// 勾选只改**点的那一根**，并且原样传到通道。
    #[test]
    fn line_state_requests_reach_the_channel_one_line_at_a_time() {
        let (mut app, handle) = connected_app();

        app.apply_line_state(Some(true), None);

        assert!(app.rts);
        assert!(!app.dtr, "只动 RTS 时 DTR 必须原样不动");
        assert_eq!(handle.line_state_calls(), vec![(Some(true), None)]);

        app.apply_line_state(None, Some(true));

        assert!(app.dtr);
        assert!(app.rts, "只动 DTR 时 RTS 必须原样不动");
        assert_eq!(
            handle.line_state_calls(),
            vec![(Some(true), None), (None, Some(true))]
        );
    }

    /// 线状态设置失败是**非致命**的：界面继续显示"已连接"，数据照样收发。
    ///
    /// 判成掉线的后果是"拉不动一根控制线 → 界面跳到未连接 → 用户去重连"，而通道
    /// 其实一直是好的。所以这条路径走 `Notice` 而不是 `Failed`。
    #[test]
    fn failing_to_set_line_state_is_a_notice_not_a_disconnect() {
        let (mut app, handle) = connected_app();
        handle.reject_line_state(true);

        app.apply_line_state(Some(true), None);

        assert!(app.connected, "拉不动控制线不等于掉线");
        assert!(
            app.receive_text.contains("设置 RTS/DTR 失败"),
            "失败必须留下可见的提示：{}",
            app.receive_text
        );
        assert!(
            app.rts,
            "勾选态表达的是「我们请求的电平」，投递失败也照样记下来"
        );
    }

    /// 断开后两根线的勾选态归位。留着上次的勾选会让下一次打开端口时界面
    /// 与实际电平对不上——那是"界面在说谎"，比丢一个勾选更糟。
    #[test]
    fn control_line_state_resets_on_teardown() {
        let (mut app, _handle) = connected_app();
        app.apply_line_state(Some(true), Some(true));
        assert!(app.rts && app.dtr);

        app.close_connection();

        assert!(!app.rts && !app.dtr, "断开后勾选态必须归位");
    }

    /// `Notice` 是**非致命**事件：只显示一条提示，不动 `connected`、也不停定时发送。
    ///
    /// 与 `failing_to_set_line_state_is_a_notice_not_a_disconnect` 是一对——那条从
    /// 发送侧看，这条从接收侧的事件循环看。两条都要有：事件循环里把 `Notice` 归进
    /// `Failed` 那一支，症状就是"拉不动一根控制线，界面跳到未连接并停掉定时发送"，
    /// 而数据其实一直收得到。
    #[test]
    fn notice_event_does_not_look_like_a_disconnect() {
        let (mut app, handle) = connected_app();
        app.repeat_send = true;

        handle.emit(ChannelEvent::Notice(
            "设置 RTS 失败: 设备不支持".to_string(),
        ));
        app.receive_data();

        assert!(app.connected, "非致命通知不得把界面判成掉线");
        assert!(app.repeat_send, "非致命通知不得停掉定时发送");
        assert!(
            app.receive_text.contains("设置 RTS 失败"),
            "提示必须显示出来：{}",
            app.receive_text
        );
    }

    // —— N4：发送字节计数 ——

    /// 计数只在**确实写出去**之后累加。
    ///
    /// 三种"没发出去"各测一遍：解析失败、写失败、以及恢复之后继续累加。排队时就
    /// 记账会显示出一个并不成立的"已发送"，用户拿它对账就永远对不上。
    #[test]
    fn tx_bytes_counts_only_writes_that_succeeded() {
        let (mut app, handle) = connected_app();
        app.send_text = "abc".to_string();
        assert!(app.send_data());
        assert_eq!(app.tx_bytes, 3);

        // ① 解析失败（HEX 位数是奇数）
        app.send_format = SendFormat::Hex;
        app.send_text = "4".to_string();
        assert!(!app.send_data());
        assert_eq!(app.tx_bytes, 3, "解析失败什么都没发出去，计数不能动");

        // ② 通道写失败
        app.send_format = SendFormat::Text;
        app.send_text = "de".to_string();
        handle.reject_writes(true);
        assert!(!app.send_data());
        assert_eq!(app.tx_bytes, 3, "写失败不算发送");

        // ③ 恢复之后接着累加
        handle.reject_writes(false);
        assert!(app.send_data());
        assert_eq!(app.tx_bytes, 5);
    }

    /// 计数只描述**本次连接**：断开后归零，否则它会跨连接一直涨下去，
    /// 变成一个"这次到底发了多少"回答不了的数字。
    #[test]
    fn tx_bytes_resets_on_teardown() {
        let (mut app, _handle) = connected_app();
        app.send_text = "abcd".to_string();
        assert!(app.send_data());
        assert_eq!(app.tx_bytes, 4);

        app.close_connection();

        assert_eq!(app.tx_bytes, 0);
    }

    // —— N2：常用数据 ——

    /// 常用数据与输入框走**同一条**通路：编码、行尾、计数都只有一处实现。
    ///
    /// 三处"不做"一并锁住：不动输入框、不进历史、不受「发送后清空」影响。
    #[test]
    fn sending_a_preset_reuses_the_normal_send_path() {
        let (mut app, handle) = connected_app();
        app.send_line_ending = SendLineEnding::Crlf;
        app.send_text = "hello".to_string();
        assert!(app.save_current_as_preset());

        // 用户手上正在写别的东西
        app.send_text = "draft".to_string();
        assert!(app.send_preset(0));

        assert_eq!(
            handle.written_bytes(),
            b"hello\r\n",
            "常用数据必须走 build_payload：行尾与编码都在那里"
        );
        assert_eq!(app.send_text, "draft", "发一条常用数据不该动输入框");
        assert_eq!(app.send_history.len(), 0, "点按钮不产生输入，不该进历史");
        assert_eq!(app.tx_bytes, 7, "常用数据发送同样要计数（含行尾两个字节）");
    }

    /// 未连接时不发；越界下标也不发（界面不该给出越界下标，但这层不能只靠界面）。
    #[test]
    fn preset_send_is_refused_when_there_is_no_channel() {
        let (mut app, handle) = connected_app();
        app.send_text = "cmd".to_string();
        assert!(app.save_current_as_preset());
        app.close_connection();

        assert!(!app.send_preset(0), "没有通道就发不出去");
        assert!(!app.send_preset(99), "越界下标不该 panic");
        assert_eq!(handle.write_count(), 0);
    }

    /// 空内容与重复内容都进不去；满了之后也不淘汰旧的（这些是用户亲手挑的）。
    #[test]
    fn presets_reject_empty_duplicate_and_overflow() {
        let mut app = ComTool::default();

        // 默认输入框就是空的：空内容存进去只会得到一个点不动的按钮
        assert!(app.send_text.is_empty());
        assert!(
            !app.save_current_as_preset(),
            "空内容存进去只会得到一个点不动的按钮"
        );
        assert!(app.send_presets.is_empty());

        app.send_text = "cmd".to_string();
        assert!(app.save_current_as_preset());
        assert!(!app.save_current_as_preset(), "一字不差的重复项要拒绝");
        assert_eq!(app.send_presets.len(), 1);

        // 填到满
        for i in 0..crate::send::MAX_SEND_PRESETS {
            app.send_text = format!("filler-{i}");
            app.save_current_as_preset();
        }
        assert!(app.send_presets.is_full());
        assert_eq!(app.send_presets.len(), crate::send::MAX_SEND_PRESETS);

        app.send_text = "one-more".to_string();
        assert!(
            !app.save_current_as_preset(),
            "满了必须拒绝，而不是淘汰第一条"
        );
        assert_eq!(
            app.send_presets.len(),
            crate::send::MAX_SEND_PRESETS,
            "拒绝之后条数不该变"
        );
        assert_eq!(
            app.send_presets.entries()[0],
            "cmd",
            "第一条必须还在：常用数据是用户亲手挑的，静默淘汰会让人以为程序丢了数据"
        );
    }

    // —— N5 / N6 ——

    /// 自定义波特率越界由 `clamp_baud` 兜住（界面上拖不出界，但存档是手改得动的）。
    #[test]
    fn baud_rate_is_clamped_when_building_the_channel_config() {
        assert_eq!(clamp_baud(0), crate::serial_impl::CUSTOM_BAUD_MIN);
        assert_eq!(clamp_baud(u32::MAX), crate::serial_impl::CUSTOM_BAUD_MAX);
        assert_eq!(clamp_baud(921_600), 921_600, "合法值必须原样保留");
    }

    /// 重新获得焦点才重扫；一直有焦点时**不重复枚举串口**。
    ///
    /// 最后一条断言是这条测试的重点：没有 `was_focused` 判重的话，每帧都会枚举一次
    /// 系统的串口列表，而 `focused` 在绝大多数帧里都是 true。
    #[test]
    fn regaining_focus_rescans_ports_without_touching_the_selection() {
        let mut app = ComTool {
            selected_port: "NOT_A_REAL_PORT".to_string(),
            was_focused: false,
            ..ComTool::default()
        };

        app.note_focus(true);

        assert_eq!(
            app.selected_port, "NOT_A_REAL_PORT",
            "热插拔重扫只换列表，绝不能动用户选的那个端口"
        );
        assert!(app.was_focused);

        // 没丢过焦点：不该重扫
        app.ports = vec!["SENTINEL".to_string()];
        app.note_focus(true);
        assert_eq!(
            app.ports,
            vec!["SENTINEL".to_string()],
            "没丢焦点就不该重扫"
        );

        // 真丢了再回来：必须重扫
        app.note_focus(false);
        app.note_focus(true);
        assert!(
            !app.ports.contains(&"SENTINEL".to_string()),
            "丢了焦点再回来必须重扫，而不是沿用旧列表"
        );
    }

    /// 手动「刷新」是另一条规则：它要顺带修正一个已经失效的选择。
    ///
    /// 与上一条配对存在——两者一旦被合并成一个函数，要么是"切回窗口偷偷改掉用户
    /// 选着的端口"，要么是"点了刷新却还留着一个已经拔掉的端口"。
    #[test]
    fn manual_refresh_corrects_a_stale_selection() {
        let mut app = ComTool {
            selected_port: "NOT_A_REAL_PORT".to_string(),
            ..ComTool::default()
        };

        app.refresh_ports();

        assert_ne!(
            app.selected_port, "NOT_A_REAL_PORT",
            "手动刷新要修掉失效的选择，否则点「打开串口」只会失败"
        );
    }

    // ——— 全局字体档位 ———

    /// 从 context 里把某一档文字样式的实际字号读出来。
    ///
    /// 用 `global_style()`（"当前生效的那份"）而不是 `style_of(Theme::Dark)`：
    /// `apply_font_scale` 走的是 `all_styles_mut`，深浅两份都改了，读哪份都一样。
    /// 读"当前生效的那份"才是用户真正看到的那份。
    fn body_size(ctx: &egui::Context) -> f32 {
        ctx.global_style()
            .text_styles
            .get(&egui::TextStyle::Body)
            .expect("Body 样式必须存在")
            .size
    }

    /// 档位必须真的改变字号，而不是"存了个字段但没人用"。
    ///
    /// 断言的是**相对关系**而不是具体像素值：像素值是实现细节，将来换字号基线
    /// 不该让这条测试红。真正要钉死的是"大 > 标准 > 小"这个单调性，以及
    /// `Normal` 恰好等于 egui 的默认字号（那是"升级后界面不变"的承诺）。
    #[test]
    fn font_scale_changes_text_sizes() {
        let ctx = egui::Context::default();
        let mut app = ComTool {
            ui_font_scale: UiFontScale::Normal,
            ..ComTool::default()
        };

        app.apply_font_scale(&ctx);
        let normal = body_size(&ctx);

        app.ui_font_scale = UiFontScale::Small;
        app.apply_font_scale(&ctx);
        let small = body_size(&ctx);

        app.ui_font_scale = UiFontScale::ExtraLarge;
        app.apply_font_scale(&ctx);
        let xlarge = body_size(&ctx);

        assert!(
            small < normal,
            "「小」档必须比「标准」小（小={small} 标准={normal}）"
        );
        assert!(
            xlarge > normal,
            "「特大」档必须比「标准」大（特大={xlarge} 标准={normal}）"
        );
    }

    /// 反复调用必须幂等——**这是最容易写错的一处**。
    ///
    /// 如果实现写成"在当前字号上乘系数"，每帧调用一次就会指数级放大：三秒钟就能
    /// 把界面字号顶到几万像素、控件全部飞出屏幕。正确的写法是每次都从**固定基线表**
    /// 重算，这条测试就是钉住这一点（连调 6 次，字号必须一动不动）。
    ///
    /// ⚠️ 只断言"第一次和第六次相等"是**不够的**：如果实现是"先连乘、再被别的
    /// 赋值盖掉"，末值照样稳定，测试会绿着放行。所以这里额外钉一条**绝对值**——
    /// 字号必须恰好等于 `基线 × 系数`。绝对值只可能由"从基线重算"得到，
    /// 任何"在上一帧结果上做文章"的写法都过不了这一条。
    #[test]
    fn font_scale_is_idempotent_across_frames() {
        let ctx = egui::Context::default();
        let app = ComTool {
            ui_font_scale: UiFontScale::Large,
            ..ComTool::default()
        };
        let baseline = FONT_SIZE_BASELINE
            .iter()
            .find(|(text_style, _)| *text_style == egui::TextStyle::Body)
            .map(|(_, base_size)| *base_size)
            .expect("基线表必须有 Body");

        app.apply_font_scale(&ctx);
        let first = body_size(&ctx);

        // 绝对值：只可能由"基线 × 系数"得到
        assert_eq!(
            first,
            baseline * UiFontScale::Large.factor(),
            "字号必须恰好等于「基线 × 系数」（基线 {baseline} × 系数 {}），\
             实际是 {first}——说明它不是从固定基线重算出来的",
            UiFontScale::Large.factor()
        );

        for _ in 0..5 {
            app.apply_font_scale(&ctx);
        }
        let after = body_size(&ctx);

        assert_eq!(
            first, after,
            "字体档位每帧重建必须幂等：连调 6 次后字号从 {first} 变到 {after}，\
             说明实现是在上一帧的结果上连乘（会指数放大）"
        );
    }

    /// 基线表必须与 egui 的默认字号一致。
    ///
    /// `apply_font_scale` 每次从 `FONT_SIZE_BASELINE` 重算，所以这张表是"标准档
    /// 长什么样"的唯一真相。它与 egui 自身默认值漂移的后果很隐蔽：上游把 Body
    /// 从 13 调到 14，我们的"标准"档却还写 13——用户选了标准，界面反而比不设时小。
    /// 这条测试就是防这个漂移的（表里加了新样式却忘了同步，也会在这里红）。
    #[test]
    fn font_scale_baseline_matches_egui_defaults() {
        let defaults = egui::Style::default().text_styles;

        for (text_style, base_size) in FONT_SIZE_BASELINE {
            let upstream = defaults
                .get(text_style)
                .unwrap_or_else(|| panic!("基线表里的 {text_style:?} 在 egui 默认样式里不存在"));
            assert_eq!(
                base_size, &upstream.size,
                "{text_style:?} 的基线字号（{base_size}）与 egui 默认值（{}）不一致——\
                 上游改了默认字号，我们的「标准」档必须跟着改",
                upstream.size
            );
        }

        // 反向：egui 默认有的样式，基线表里不该漏——漏了的那一档会永远停在
        // 上游默认值上，用户调大字体时它纹丝不动（比如标题不变、正文变大）。
        for text_style in defaults.keys() {
            assert!(
                FONT_SIZE_BASELINE
                    .iter()
                    .any(|(pinned, _)| pinned == text_style),
                "{text_style:?} 在 egui 默认样式里存在，却没进 FONT_SIZE_BASELINE——\
                 这一档不会跟随全局字体档位"
            );
        }
    }
}
