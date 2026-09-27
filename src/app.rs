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
use crate::receive::{AnsiDecoder, AnsiStyle, ColoredSpan, TextEncoding, TimestampPrefix};
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
    // ── 第三批（tr / ru / pl；见 I18N_REMAINING_LANGUAGES.md §4）──
    // 这三种语言的字体已被微软雅黑**全覆盖**（实测缺失数均为 0），复数成本也是 0
    // （现有英文列本身即不做复数），所以属于"零前置工作"即可落地的档位。
    /// Türkçe（土耳其语）。
    Tr,
    /// Русский（俄语）。
    Ru,
    /// Polski（波兰语）。
    Pl,
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
        Self::Tr,
        Self::Ru,
        Self::Pl,
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
            Self::Tr => "Türkçe",
            Self::Ru => "Русский",
            Self::Pl => "Polski",
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

/// 发送框右下角那个"拖动改高度"把手的 id。
///
/// 显式命名而不用自动 id：自动 id 由"这一帧前面已经画了多少个控件"决定，前面
/// 加一个控件它就跟着变，测试也就无法稳定地把把手读出来。测试走
/// `egui::Context::read_response(egui::Id::new(SEND_RESIZE_GRIP_ID))` 拿它这一帧的
/// 矩形——"把手在框的右下角、拖完以后框变高了"是唯一能在无头环境里量到的判据。
pub(crate) const SEND_RESIZE_GRIP_ID: &str = "send_resize_grip";

/// 发送输入框的 id。
///
/// 显式常量而不是就地写字符串：界面与测试都要用它（测试靠
/// [`egui::Context::read_response`] 读回输入框这一帧的矩形，量"框自己有多高"）。
pub(crate) const SEND_INPUT_ID: &str = "send_input";

/// 把手的高度（像素）。
///
/// 10 px 是"看得见、点得中"与"不占版面"之间的折中：再窄一点鼠标很难压上去，
/// 再高就变成一条突兀的空条。
const SEND_RESIZE_GRIP_HEIGHT: f32 = 10.0;

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
    //
    // 语义是"**已经发起过一次打开**"，不是"通道当前确实通着"。后者由
    // [`ComTool::channel_is_ready`] 回答——串口的 `open()` 只拉起后台线程，端口
    // 要到那里才真正打开，所以这两件事在串口上会短暂不一致（那一小段叫"正在
    // 打开"，见 [`ComTool::opening`]）。**任何"该不该放行"的判断都必须看就绪**，
    // 只看这个字段就是把 `connected` 假阳性缺陷换个地方复现。
    #[serde(skip)]
    pub connected: bool,

    /// "已打开 / 已连接"那条提示是否已经补过（活不过进程边界）。
    ///
    /// 只有串口用得上：它的就绪发生在 `open_connection` **返回之后**，那条成功
    /// 提示只能延后到 [`ComTool::settle_connection_state`] 里补。没有这个记账位，
    /// 补提示会因为"通道一直就绪"而每帧说一遍。
    #[serde(skip)]
    pub announced_open: bool,

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
    /// 发送框一次能预览多少行（用户拖右下角把手调整）。
    ///
    /// 存**行数**而不是像素高度：像素值换一台机器（不同 DPI）或改一次字体档位就
    /// 对不上，而用户在意的是"我能一次看见几行"。像素换算发生在
    /// [`ComTool::render_send_editor`]，用的是当前字号下的实际行高。
    ///
    /// 落盘：这是用户亲手调过的界面尺寸，下次启动该保持原样。
    pub send_visible_rows: u32,
    /// 拖把手时"不足一行"的像素余量（纯运行时状态，不落盘）。
    ///
    /// 见 [`crate::send::apply_send_rows_drag`]：拖动是连续的、行数是整数，不攒着
    /// 慢慢拖就永远跨不过一行。松手即清零。
    #[serde(skip)]
    pub send_rows_drag_px: f32,

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

    /// 连续失败次数，用于指数退避（**纯运行时状态，不落盘**）。
    ///
    /// 旧实现是"每 500 ms 定时重试一次、永不放弃"：串口被别的程序占用时，
    /// 它会以每秒约 2 次的频率反复新建后台线程，日志与界面提示被刷屏，
    /// 而重试本身注定失败。改为"间隔随连续失败次数指数增长 + 有上限次数"。
    ///
    /// **连接成功即清零**——它描述的是"这一轮失败连着几次"，不是累计故障数。
    #[serde(skip)]
    pub reconnect_failures: u32,

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
    ///
    /// 与 [`ComTool::color_view`] **可以共存**（HEX 文本 + 彩色组件），这一组合是合法的。
    pub hex_view: bool,
    #[serde(skip)]
    pub display_spans: Vec<ColoredSpan>,
    #[serde(skip)]
    pub decoder: AnsiDecoder,
    /// 彩色文本视图：把接收到的 ANSI 颜色按显式色渲染（没发颜色的部分用主题正文色）。
    /// 它与普通文本视图吃的是**同一份**解析结果，差别只在渲染方式。
    ///
    /// **与 [`ComTool::ascii_view`] 互斥**——字节模式只写 `ascii_spans`，而彩色视图渲染
    /// `display_spans`，两个同时为真时彩色视图**必然是空的**（2026-09-24 用户目视发现，
    /// 见 `HANDOVER.md` §3.16）。与 [`ComTool::hex_view`] 可以共存。
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
    /// **与 [`ComTool::hex_view`] 互斥**（HEX 分支只写 `display_spans`），**也与
    /// [`ComTool::color_view`] 互斥**（彩色视图渲染 `display_spans`）——两个同时为真时
    /// 这个视图或那个视图必然是空的。互斥由 [`ComTool::resolve_display_mode`] 统一裁决，
    /// 界面（菜单与接收设置行）与启动修复都走它，不要在别处另写一份判断。
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
    /// 窗口是否总在最前端（「视图」菜单里的开关，落盘）
    ///
    /// **真正的生效在 [`ComTool::apply_always_on_top`]**：这一项只是偏好值，改它不会
    /// 自己碰到窗口——给窗口下命令需要一个 `egui::Context`，那是绘制期才有的东西。
    /// 判据也就落在那条命令上：无头测试读 `FullOutput::viewport_output`。
    pub always_on_top: bool,
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
    /// 上一次绘制接收区时，视口是否贴着底部。
    ///
    /// 贴底 = 正在跟随最新内容，此时淘汰最旧的内容用户看不见；不贴底 = 用户翻到历史
    /// 里在阅读，此时**不能再淘汰**——被淘汰的内容会同时从他视口上方消失，而滚动位置
    /// 不变，等于阅读位置被一行行抽走，看起来就是"自动滚动根本没停"（2026-09-26 用户
    /// 报告）。判定在 [`crate::receive::is_at_bottom`]，消费在
    /// [`crate::receive::ComTool::trim_receive_logs`]。
    ///
    /// 与 `connected` / `show_settings` 同款：字段会被序列化，但 [`crate::config::restore_runtime_state`]
    /// 启动时一律重置回 `true`（"上一次会话恰好停在历史里"不该让新会话一直不敢裁剪）。
    pub receive_at_bottom: bool,
    /// 贴底追平历史欠账时，**这一帧最多淘汰多少字符**（由上一帧实测的
    /// "可视区高 / 每字符高"推出，见 [`crate::receive::trim_budget_per_frame`]）。
    ///
    /// 为什么不能一次裁完：egui 用**上一帧**的滚动位置给内容布局，把 offset 夹回合法
    /// 范围发生在内容画完之后。一帧里把内容缩短超过一个可视区高度，这一帧的内容就整个
    /// 落在视口上方——用户看到接收区**空一帧**（2026-09-26 实测复现）。分帧追平即可。
    ///
    /// `usize::MAX` = 不限制（还没有实测数据时的兜底：没数据就说明还没画过，那时也不可能
    /// 有欠账；真有异常值也不该因此把内存上限放开）。
    pub receive_trim_budget: usize,
    /// 本帧从接收日志**最旧端**裁掉的那一段原文，等下一次绘制接收区时用来补偿滚动位置。
    ///
    /// 为什么必须留着它：淘汰发生在内容最旧的一端，而它同时改变"滚动位置指向的是哪一行"
    /// ——内容整体上移、像素位置不变，视口里那一行就被换成了更晚的一行（观感＝"日志还在
    /// 自动滚"）。把裁掉的那段**按同样的折行宽度排版一遍**就能量出它占了多少像素，于是
    /// 让滚动位置同步减去这个高度，视口指着的还是原来那一行。
    ///
    /// 只有"有人正在看"才需要补偿 ⇒ 接收区没画时 `trim_receive_logs` 不往这里累积
    /// （见 [`crate::receive::ComTool::trim_receive_logs`]），否则它会一路涨成第二份日志。
    #[serde(skip)]
    pub receive_trimmed_head: String,
    /// 上一次绘制接收区**结束时**的纵向滚动位置（像素）。
    ///
    /// 补偿的基准必须是"用户实际看到的位置"。⚠️ **不能改用
    /// `egui::scroll_area::State::load` 去读**：实测在绘制前的上下文里它恒返回 `None`
    /// （基准取成 0 ⇒ 视口被钉死在最顶部）。而这个值取自 `ScrollArea::show` 的返回值，
    /// 是 egui 自己算完贴底判定之后的最终结果——用户正在跟随最新数据时它等于最大值，
    /// 那种情况下贴底逻辑本来就会覆盖补偿，用哪个值都不影响结果。
    ///
    /// `None` = 还没画过，无从换算 ⇒ 那一帧不补偿。
    #[serde(skip)]
    pub receive_last_offset_y: Option<f32>,
    /// 上一次绘制接收区时的可视区**宽度**（像素）。
    ///
    /// 补偿要按"与当时相同的折行宽度"排版被裁文本，折行宽度不同 ⇒ 量出来的高度不同
    /// （同一段文字窄窗口里会折成两倍的行数）。第一帧还没有这个数（0），此时不补偿。
    #[serde(skip)]
    pub receive_viewport_w: f32,
    /// 上一次绘制接收区实测的"每字符多少像素高"（`内容高 / 字符数`）。
    ///
    /// 把"视口顶行之上有多少像素"（`receive_last_offset_y`）换算成"还能安全裁多少
    /// 字符"要用它——保住阅读位置那条规则的输入就是这两个数（见
    /// [`crate::receive::ComTool::reading_headroom_chars`]）。0.0 = 还没量出来，
    /// 此时不做那条收紧。
    #[serde(skip)]
    pub receive_px_per_char: f32,
    /// 上一次**已经下发给窗口**的置顶值（`true` = 已置顶）。
    ///
    /// ⚠️ **为什么不能像 `apply_theme` 那样问 egui 当前是什么层级**：窗口层级是
    /// 一条**只写的命令**（`ViewportCommand::WindowLevel`），egui 侧不保存、也读不回来
    /// 当前层级——它只是被转发给宿主窗口（winit 的 `set_window_level`）。所以"已生效
    /// 值"只能自己记。
    ///
    /// 记它的用处是**去重**：没有它就会每帧下一次命令，Windows 上等于每帧一次
    /// `SetWindowPos`。`None` = 还没下发过（启动时必然是这个值 ⇒ 第一帧一定下发，
    /// 这才让"上次勾选过置顶"能在启动时恢复）。
    #[serde(skip)]
    pub applied_always_on_top: Option<bool>,
    /// 正在运行的实时文件日志。`Some` 即表示「记录中」——不另设 bool 标志位，
    /// 两个字段表达同一件事迟早会不一致。
    #[serde(skip)]
    pub log: Option<LogSink>,
    /// 实时图表的样本缓冲。每次收到数据都会推进，且与接收日志一样是纯运行时
    /// 内容——落盘只会让退出时的保存变慢。
    #[serde(skip)]
    pub chart: ChartData,
}

/// 自动重连的首次退避间隔。
///
/// 与旧实现保持一致（500 ms）：第一次失败后仍然很快重试一次，覆盖"对端只是
/// 慢了一拍"这种最常见的情况——一上来就等好几秒会让正常重连的体感明显变差。
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(500);

/// 自动重连退避的上限。
///
/// 30 s 是"仍然算自动重连"与"还不如用户自己点一下"之间的折中：再长的话，
/// 用户盯着一个不动的界面等重连不如自己按按钮。
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

/// 自动重连的最大连续失败次数。
///
/// 取 6：按下面的指数退避，从 500 ms 累加到停手总计约 30 s 量级的重试窗口
/// （0.5 + 1 + 2 + 4 + 8 + 16 ≈ 31.5 s），足够覆盖"设备复位中 / 驱动还在初始化"，
/// 又不至于让用户对着一个徒劳的重试过程干等。到顶后彻底停手并留一条提示。
///
/// `pub(crate)`：`receive.rs` 在收到 `Failed` 时要判断"是不是刚好撞到上限"，
/// 以便只提示一次"自动重连已停止"。
pub(crate) const MAX_RECONNECT_ATTEMPTS: u32 = 6;

/// 第 `failures` 次连续失败之后，下一次重连前应等待多久（**纯函数**）。
///
/// `failures` 的含义是"**已经**连续失败了这么多次"，因此首轮重试（`failures == 0`）
/// 得到 [`RECONNECT_BASE_DELAY`]（500 ms），之后按 2 的幂增长，封顶
/// [`RECONNECT_MAX_DELAY`]。
///
/// 抽成纯函数是为了能被单测直接驱动——重连的实际时序依赖时钟与真实串口，
/// 在门禁里造不出；而"第 n 次该等多久"这件事本身与时钟无关，可以钉死。
///
/// 用 `saturating_mul` 与 `min` 双重保护：`failures` 理论上不会很大（到
/// [`MAX_RECONNECT_ATTEMPTS`] 就停手），但纯函数不该依赖调用方守规矩——
/// 一个手改过的存档或将来放宽的上限都不该让它 panic 或溢出。
fn reconnect_backoff(failures: u32) -> Duration {
    let factor = 1u32 << failures.min(16);
    RECONNECT_BASE_DELAY
        .saturating_mul(factor)
        .min(RECONNECT_MAX_DELAY)
}

impl ComTool {
    /// 从 `eframe` 存储恢复上次会话。
    ///
    /// 存档只提供用户偏好；串口句柄、事件通道、端口列表等运行时状态由
    /// [`crate::config::load`] 统一重建。
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // `cc.storage` 已经是 `Option<&dyn Storage>`，`as_deref()` 解出来的还是同一个类型
        // （clippy::needless_option_as_deref）。
        crate::config::load(cc.storage)
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
    ///
    /// # 指数退避 + 上限次数
    ///
    /// 旧实现是固定 500 ms、永不放弃：串口被别的程序占用时每秒重试两次、
    /// 每次都新建一个后台线程，注定失败还刷屏。改为
    ///
    /// * 间隔按连续失败次数指数增长（见 [`reconnect_backoff`]），最长 30 s；
    /// * 连续失败达到 [`MAX_RECONNECT_ATTEMPTS`] 次后**彻底停手**，并在接收区
    ///   留一条说明——继续重试只会浪费 CPU，用户需要知道"它不会再试了"。
    ///
    /// 停手之后只要用户重新点「连接」，[`ComTool::open_connection`] 会把
    /// `reconnect_failures` 清零，自动重连也随之恢复。
    pub fn auto_reconnect(&mut self) {
        // 只有串口曾经打开过、现在断开时才尝试自动重连
        if self.connection_kind != ConnectionKind::Serial {
            return;
        }
        if self.connected || self.channel.is_none() {
            return;
        }

        // 连续失败到顶就停手。判定放在节流之前：它是一次性决定，不该被时间门挡着。
        if self.reconnect_failures >= MAX_RECONNECT_ATTEMPTS {
            return;
        }

        let now = Instant::now();
        let wait = reconnect_backoff(self.reconnect_failures);
        // `is_none_or` = "没有上次尝试（第一次）→ 立刻试；有 → 等到退避期满"。
        let need_retry = self
            .last_reconnect_attempt
            .is_none_or(|t| now.duration_since(t) >= wait);
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
        // `as_bytes()` 已经是 `&[u8]`，再取一次引用只是多一层（clippy::needless_borrow）。
        match std::fs::write(&path, self.receive_text.as_bytes()) {
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
    /// # 串口：不再乐观置位，而是"按通道的实际状态结算"
    ///
    /// 旧实现在最后无条件 `connected = true`，于是打开一个**不存在**的端口时，
    /// 界面会先亮一下"已连接"，等 `Failed` 事件回来再跳回"未连接"——用户在亮着
    /// 的那一瞬点「发送」，数据会交给一个不存在的端口。这就是
    /// `FEATURE_GAP_AND_PLAN.md` §4 记的 `connected` 假阳性。
    ///
    /// 现在改为在最后调一次 [`ComTool::settle_connection_state`]：是否算连上**由
    /// 通道自己说了算**（`is_active()`）。串口的 `open()` 只拉起后台线程，端口在
    /// 后台线程里才打开，因此刚返回时状态是 [`ComTool::opening`]（界面显示"正在
    /// 打开"，发送按钮照旧不生效），下一帧由 `settle_connection_state` 结算；
    /// 真失败则走 `Failed` 事件回到"未连接"。TCP / UDP 同一步会当场判定为已连接，
    /// 行为与从前完全一致。
    ///
    /// ⚠️ **串口的 `settle_connection_state()` 之前，`connected` 为 `true` 而通道
    /// 未就绪**——那正是"正在打开"。任何"该不该放行"的判断都必须看
    /// [`ComTool::channel_is_ready`]，不能只看 `connected`，否则这个缺陷只是换了个
    /// 地方复现（见 `connected` 字段的文档）。
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
        // `connected` 在这里的含义是"**已经发起过一次打开**"（记账位）。
        // 真正的"通没通"由 `channel_is_ready()` 回答，界面据此显示"正在打开"。
        self.connected = true;
        // 新的连接发起 ⇒ 成功提示要重新补一次（见 `settle_connection_state`）。
        self.announced_open = false;

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
        //
        // 串口那句有个必须点明的差别：`open()` 返回 `Ok` 只表示"后台线程起来了"，
        // 端口**还没有**打开。写"已打开"会和紧接着可能到来的 `Failed` 自相矛盾
        // （用户看到"已打开"下一行就是"打开失败: 拒绝访问"）。所以串口写"正在
        // 打开"，由 [`ComTool::settle_connection_state`] 确认后再补一条"已打开"。
        let open_msg = match self.connection_kind {
            ConnectionKind::Serial => format!("正在打开 {target}\n"),
            ConnectionKind::TcpClient => format!("已连接 {target}\n"),
            ConnectionKind::Udp => format!("UDP 已绑定，目标 {target}\n"),
        };
        // 网络通道此刻**确实**已经就绪（`open()` 同步做完了 connect / bind），
        // 上面那句就是最终的成功提示，不需要结算再补——把记账位置真以免重复说。
        // 串口此刻还没就绪（`is_ready` 为假），留假，等端口真的打开时由结算补。
        self.announced_open = self.channel_is_ready();
        self.display_spans.push(ColoredSpan {
            text: open_msg.clone(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
        self.push_receive_note(open_msg);
    }

    /// 把"通道已经就绪"这件事落成一条"已打开"的提示。每帧调用一次。
    ///
    /// 存在的唯一理由是串口：它的 `open()` 只拉起后台线程，端口要到后台线程里
    /// 才真正打开。`open_connection` 返回时通道可能还没就绪，"正在打开"就是
    /// 那一小段的呈现；等后台线程把状态推到就绪，这里补一条"已打开"。
    ///
    /// # 判定为什么是"通道就绪 且 提示还没补过"
    ///
    /// 不是"`connected` 为假才动手"——`connected` 现在的含义是"**已经发起过一次
    /// 打开**"（见字段文档），`open_connection` 在返回前就把它置真了。用它当门
    /// 会让本函数在串口这条路上**一次也不执行**。
    ///
    /// 去重的落点是 `announced_open` 这个**一次性记账位**：通道就绪可能持续很多
    /// 帧，没有它，"已打开"会每帧刷一条。而 `announced_open` 只在连接发起时清零，
    /// 所以"连上 → 掉线 → 重连"这条路径每次都能重新补一条。
    ///
    /// 三条硬约束：
    ///
    /// * **只升不降**。通道不就绪不等于掉线——那要由 `Failed` / `Closed` 事件说了算
    ///   （只有它们知道"为什么"）。这里主动降会与事件重复报错，还会把 `close` 时
    ///   说过的"连接已关闭"再说一遍。
    /// * **按类型选词**：`connection_kind` 而不是 `channel_kind`。复用同一条通道
    ///   对象时两者相等、看不出差别，但类型切换的那一帧只有前者是对的。
    /// * **不对网络通道说第二遍**：它们的 `open_connection` 已经当场报过"已连接"，
    ///   所以发起时就把 `announced_open` 置真（见 `open_connection`）。
    pub fn settle_connection_state(&mut self) {
        if self.announced_open || !self.channel_is_ready() {
            return;
        }
        self.announced_open = true;
        // 只有串口会走到这里（网络通道在 `open_connection` 里就说了）。
        // 措辞按类型分叉，免得将来某个网络通道也变成异步打开时这里说错话。
        let noun = self.transport_noun();
        self.push_receive_note(format!("{noun}已打开\n"));
    }
    /// 通道当前是否**确实**是通的。
    ///
    /// 与 [`ComTool::connected`] 的分工是这次修缺陷的核心：
    ///
    /// * 结论来自**通道自己**（[`ByteChannel::is_ready`]，直读状态位），不是应用猜的；
    /// * 串口的 `open()` 只拉起后台线程，端口要到后台线程里才真正打开，所以
    ///   `open()` 返回后的**一小段时间里**它就是假——这段时间在界面上显示为
    ///   "正在打开"（见 [`ComTool::opening`]），而不是"已连接"。
    ///
    /// `channel` 为 `None` 时返回 `false`。
    ///
    /// ⚠️ **不要再加"串口特殊处理"**（例如"串口拿不到就绪态就沿用 `connected`"）：
    /// 那正是把这次修的缺陷原样请回来。串口那三态由
    /// [`crate::serial_impl::SerialChannel`] 覆盖 [`ByteChannel::is_ready`] 提供，
    /// 网络通道走 trait 的默认实现（等价于 `is_active()`），分叉点**只有一处**。
    ///
    /// **调用方**：发送闸门（`send_data` / `send_preset` / `tick_repeat_send`）与
    /// 侧边栏的 RTS / DTR 勾选框（见 `crate::sidebar`）。后者原先用的是 `connected`，
    /// 于是串口"正在打开"那一档里勾选框是可点的（命令只能排队）——判据统一到这条。
    pub(crate) fn channel_is_ready(&self) -> bool {
        self.channel.as_ref().is_some_and(|c| c.is_ready())
    }

    /// 是否正处在"已经发起打开、通道还没就绪"的中间态。
    ///
    /// 只有串口会有这一档：TCP / UDP 的 `open()` 在调用线程里就把 connect / bind
    /// 做完，返回 `Ok` 时通道必然已就绪。判定必须同时看 `connected`——它是
    /// "已经发起过一次打开"的记账位，断开之后就不该再自称"正在打开"。
    pub fn opening(&self) -> bool {
        self.connected && !self.channel_is_ready()
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
        // 提示记账位与"是否连上"同生共死：通道没了，下一次打开要重新说一遍
        // "已打开"。不清它的话，掉线重连之后那句成功提示永远不会再出现。
        self.announced_open = false;

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
        // 退避计数一并清零：它是"这一轮连续失败了几次"，用户主动断开就是这一轮的
        // 结束；下次再连应当是干干净净的一次尝试，而不是继承上一次失败后的长等待。
        self.reconnect_failures = 0;

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
        // 判据是 [`ComTool::channel_is_ready`] 而不是 `connected`：串口在"正在
        // 打开"的那一小段里 `connected` 已经是真，但端口还没打开，这时发出去的
        // 字节没有去处。只看 `connected` 就是那个假阳性缺陷换个地方复现。
        if !self.channel_is_ready() {
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
        // 与 `send_data` 同一条判据，见那里的注释。
        if !self.channel_is_ready() {
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
        // 与 `send_data` 同一条判据（通道确实就绪）。这里额外保留 `connected`：
        // 用户在"正在打开"时勾上定时发送，计时也要先起步，不能等连上才开始计。
        if !self.repeat_send || !self.connected || !self.channel_is_ready() {
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

    /// 把「总在最前端」的偏好同步给真正的窗口。
    ///
    /// **只在"偏好值"与"已下发值"不同时发一次命令**（去重靠
    /// [`ComTool::applied_always_on_top`]）。关掉时下发的是 `Normal` 而不是"什么都不发"
    /// ——窗口层级是三态（Normal / AlwaysOnBottom / AlwaysOnTop），不显式回到 Normal
    /// 窗口就停在置顶上不动了。
    ///
    /// **为什么挂在菜单栏 [`ComTool::show_menu_bar`] 而不是 `ui()`**：`ui()` 要收一个
    /// `eframe::Frame`，无头测试构造不出来，写在里面的代码一次也不会被 `cargo test`
    /// 走到。而"勾选 → 真的给窗口下命令"这条链路必须有接入测试（本项目的规矩：纯函数
    /// 全绿 ≠ 界面真的调了它）。菜单栏**每帧必画**、又只收 `&mut Ui`，是同时满足
    /// "每帧同步"与"可测"的那个点——这也正是启动时能恢复上次置顶的原因。
    ///
    /// 判据是 `FullOutput::viewport_output` 里的命令：egui 把视口命令公开在每帧输出里，
    /// 所以"发没发、发的是哪一档"在无头测试里可以直接断言。
    pub fn apply_always_on_top(&mut self, ctx: &egui::Context) {
        if self.applied_always_on_top == Some(self.always_on_top) {
            return;
        }
        let level = if self.always_on_top {
            egui::viewport::WindowLevel::AlwaysOnTop
        } else {
            egui::viewport::WindowLevel::Normal
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        self.applied_always_on_top = Some(self.always_on_top);
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
            announced_open: false,
            receive_text: String::new(),
            send_text: String::new(),
            send_format: SendFormat::default(),
            send_line_ending: SendLineEnding::default(),
            text_encoding: TextEncoding::default(),
            send_history: SendHistory::default(),
            send_presets: SendPresets::default(),
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            send_visible_rows: crate::send::DEFAULT_SEND_VISIBLE_ROWS,
            send_rows_drag_px: 0.0,
            channel: None,
            channel_kind: None,
            event_rx: None,
            last_reconnect_attempt: None,
            reconnect_failures: 0,
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
            // 默认**不置顶**：置顶会盖住别的窗口，是用户主动要才该有的行为，
            // 不能默认替所有人决定。
            always_on_top: false,
            receive_at_bottom: true,
            receive_trim_budget: usize::MAX,
            receive_trimmed_head: String::new(),
            receive_last_offset_y: None,
            receive_viewport_w: 0.0,
            receive_px_per_char: 0.0,
            applied_always_on_top: None,
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

            // 走全界面统一的按钮外观（见 `crate::button`）。禁用态用 `add_enabled_ui`
            // 下发而不是自己传标志：自绘按钮读 `ui.is_enabled()` 取禁用配色，同时把
            // `Sense` 降成 hover——灰着就点不动，不会出现"看起来禁用、点下去有反应"。
            let save_button = ui
                .add_enabled_ui(can_save_preset, |ui| {
                    crate::button::primary(ui, s.save_preset)
                })
                .inner;
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
                    let label = crate::send::preset_label(entry, crate::send::PRESET_LABEL_CHARS);
                    let button = crate::button::standard(ui, &label);
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

    /// 发送区表头那一行：格式 / 行尾 / 定时发送 + 间隔 / 发送历史。
    ///
    /// 从 [`ComTool::ui`] 里抽出来的理由与 [`ComTool::render_send_presets`] 相同：
    /// `ui()` 要收一个无头测试构造不出的 `eframe::Frame`，写死在里面等于永不被
    /// `cargo test` 画到。这个方法只收 `&mut Ui`，可以整行走一遍
    /// （见 `panels::tests::send_area_header_keeps_the_send_settings`）。
    ///
    /// ⚠️ 这一行是**发送格式 / 行尾 / 定时 / 历史四项设置的唯一入口**
    /// （2026-09-24 删掉菜单栏「发送」菜单之后）。删任何一项之前先想清楚用户去哪改。
    pub(crate) fn render_send_settings_row(&mut self, ui: &mut egui::Ui) {
        let s = self.language.strings();
        ui.horizontal(|ui| {
            ui.label(s.send_area);
            ui.separator();
            ui.label(s.format);
            send_format_radios(ui, &mut self.send_format, s);
            ui.separator();
            ui.label(s.line_ending);
            send_line_ending_radios(ui, &mut self.send_line_ending, s);
            ui.separator();
            if ui.checkbox(&mut self.repeat_send, s.repeat_send).changed() && self.repeat_send {
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
    }

    /// 发送框本体：定高视口（多行预览）+ 右下角可拖高的把手 + ↑ / ↓ 翻历史。
    ///
    /// 抽出来的理由与 [`ComTool::render_send_presets`] 相同：`ui()` 收一个无头测试
    /// 造不出的 `eframe::Frame`，写死在里面等于一次也不会被 `cargo test` 画到；而
    /// "把手在这里、拖完变高"恰恰是必须被真实画一遍才测得出来的事。
    ///
    /// **返回**：输入框这一帧是否持有键盘焦点。调用方据此判断 Enter 算不算"发送"
    /// ——判定留在调用方，是因为"Enter 发送"只在有焦点时成立，而焦点是这里的
    /// 局部信息。
    ///
    /// # 视口为什么是定高而不是"跟着内容长"
    ///
    /// 之前 `TextEdit::multiline` 用的是 `desired_rows(4)`：那只是**最小**高度，内容
    /// 一多它就把自己撑高，直到把下面的「发送 / 清空」按钮和常用数据挤出可视区
    /// （中央区没有整体滚动，挤出去就点不着）。改成"视口高度 = 用户选的行数、内容
    /// 更多时框内滚动"之后：把手永远停在原地（拖第二下才找得到），界面也不会被一
    /// 段粘贴进来的长文本撑变形。
    pub(crate) fn render_send_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let s = self.language.strings();
        let send_edit_id = egui::Id::new(SEND_INPUT_ID);

        // 行高取自输入框真正用的那套等宽字体（**含**全局字体档位）：把字体调大
        // 之后"拖 100 px 能加几行"本来就该变少，写死 14.0 会让换算与实际对不上。
        let font_id =
            crate::receive::monospace_font_id(ui, crate::receive::RECEIVE_MONOSPACE_BASE_SIZE);
        // 用 `fonts_mut`：`row_height` 要按需加载字形度量，需要可变访问
        // （`TextEdit` 内部也是这么取的，见 egui 的 `text_edit/builder.rs`）。
        let line_height =
            ui.fonts_mut(|f| f.row_height(&font_id)) + ui.spacing().extra_text_line_spacing;
        let visible_rows = self.send_visible_rows;

        // 4.0 = 输入框上下内边距之和（`TextEdit` 的 margin 是 `symmetric(4, 2)`）。
        // 少了它，视口会比"N 行"矮一截——最后一行永远差一点点露不出来。
        let viewport_height = line_height * visible_rows as f32 + 4.0;

        // 焦点与箭头键都必须在绘制 `TextEdit` **之前**读：TextEdit 会消费导航键
        // 事件，等它画完再读 `key_pressed` 有可能已经拿不到了。
        let had_focus = ui.ctx().memory(|m| m.has_focus(send_edit_id));
        let (up_pressed, down_pressed) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp) && !i.modifiers.shift,
                i.key_pressed(egui::Key::ArrowDown) && !i.modifiers.shift,
            )
        });

        let mut out = None;
        egui::ScrollArea::vertical()
            .id_salt("send_input_scroll")
            // 两个方向都不自动收缩：视口高度由用户决定，不该被一行内容"缩回去"。
            .auto_shrink([false, false])
            .min_scrolled_height(viewport_height)
            .max_height(viewport_height)
            .show(ui, |ui| {
                out = Some(
                    egui::TextEdit::multiline(&mut self.send_text)
                        .id(send_edit_id)
                        .desired_width(f32::INFINITY)
                        // 最小高度也是用户选的行数：内容不满时框照样占满，
                        // 于是"拖到 10 行"在空输入框上也看得出来。
                        .desired_rows(visible_rows as usize)
                        // 字号跟随全局字体档位：写死的话，用户把字体调大后发送框
                        // 仍是小字，与接收区不一致（两者都是等宽内容，本该同号）。
                        .font(font_id.clone())
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        ))
                        .show(ui),
                );
            });
        // 闭包一定被 `show` 调用一次；为 `None` 只能说明 egui 的行为变了。
        let out = out.expect("ScrollArea 必须绘制它的内容一次");
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

        // —— 把手：在框的右下角，上下拖动改可见行数 ——
        //
        // 用 `allocate_exact_size` 占位 + 显式 id 的 `interact`，而不是让 egui 派一个
        // 自动 id：自动 id 依赖"这一帧前面画了多少个控件"，换个顺序就跟着变，
        // 测试也就没法稳定地把它读出来。
        let (grip_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), SEND_RESIZE_GRIP_HEIGHT),
            egui::Sense::empty(),
        );
        // 感 click + drag 而不是只感 drag：`double_clicked` 的前提是这一帧有
        // `Flags::CLICKED`，而只有感 click 的控件才会被置上（双击复位要靠它）。
        // 代价是"按下"这一帧还分不清是单击还是拖动，要等指针移动开才判成拖动——
        // 对一条 10 px 高的把手来说这不是问题，拖的人都得先挪一段。
        let grip = ui.interact(
            grip_rect,
            egui::Id::new(SEND_RESIZE_GRIP_ID),
            egui::Sense::click_and_drag(),
        );

        if grip.dragged() {
            let (rows, carried) = crate::send::apply_send_rows_drag(
                visible_rows,
                self.send_rows_drag_px,
                grip.drag_delta().y,
                line_height,
            );
            self.send_visible_rows = rows;
            self.send_rows_drag_px = carried;
        } else {
            // 松手就丢掉余量：下一次拖动从零开始，否则上一次差的那半行会让这一
            // 次的头几像素没反应。
            self.send_rows_drag_px = 0.0;
        }
        if grip.double_clicked() {
            // 双击复位：拖到某个奇怪的高度之后总要有个"回到默认"的口子。
            self.send_visible_rows = crate::send::DEFAULT_SEND_VISIBLE_ROWS;
            self.send_rows_drag_px = 0.0;
        }

        // 静止时只在右下角留三道斜线（低调），鼠标一靠近就把整条点亮——
        // "这里能拖"要能被看见，但不该在平时抢走注意力。
        let grip_visuals = ui.style().interact(&grip);
        if grip.hovered() || grip.dragged() {
            ui.painter()
                .rect_filled(grip_rect, 2.0, grip_visuals.bg_fill);
        }
        let corner = grip_rect.right_bottom() - egui::vec2(2.0, 2.0);
        for step in 0..3 {
            let reach = 3.0 + step as f32 * 4.0;
            ui.painter().line_segment(
                [
                    egui::pos2(corner.x - reach, corner.y),
                    egui::pos2(corner.x, corner.y - reach),
                ],
                egui::Stroke::new(1.0, grip_visuals.fg_stroke.color),
            );
        }
        if grip.hovered() || grip.dragged() {
            ui.set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        grip.on_hover_text(s.send_resize_hint);

        send_has_focus
    }

    /// 中央区最上面那一行：侧边栏显隐开关（左）+ 「⚙ 设置」按钮（右）。
    ///
    /// 从 [`ComTool::ui`] 里抽出来的理由与 [`ComTool::render_send_presets`] 相同：
    /// `ui()` 要收一个无头测试构造不出的 `eframe::Frame`，写死在里面等于永不被
    /// `cargo test` 画到。这个方法只收 `&mut Ui`，可以整行走一遍
    /// （见 `panels::tests::top_toolbar_opens_the_settings_window`）。
    ///
    /// ⚠️ **右侧那一颗「⚙ 设置」按钮已经在 2026-09-24 删除**：它与顶部菜单栏的
    /// 「设置」菜单功能重复（两者都只是把 `show_settings` 置 true），按用户要求
    /// **保留顶部的**。这里现在只画左侧那颗侧边栏显隐开关。要打开设置请走菜单栏，
    /// 或在代码里置 `self.show_settings = true`。
    pub(crate) fn render_top_toolbar(&mut self, ui: &mut egui::Ui) {
        let s = self.language.strings();
        ui.horizontal(|ui| {
            ui.toggle_value(&mut self.sidebar_open, s.sidebar_toggle);
        });
    }
}

impl eframe::App for ComTool {
    /// 整幅窗口的底色（面板之外露出来的那层）。
    ///
    /// **必须自己实现**，不能用 eframe 的默认值：默认实现返回一个写死的深灰
    /// `rgba(12,12,12,180)`，参数名就叫 `_visuals`——**它拿得到主题却故意不用**
    /// （`eframe-0.36.1/src/epi.rs`，那里的注释只说"`window_fill` 也是个自然的选择"）。
    /// 一旦有任何区域没被面板盖住，浅色主题下就会露出一条深色——2026-09-24 用户目视
    /// 发现的"状态栏在浅色下没变色"正是这条路径（状态栏当时用 `Frame::NONE`，
    /// 自己不画背景，于是透出了这层默认色）。状态栏现在已经自己画背景了（见
    /// `status_bar.rs` 的 `Frame::side_top_panel`），这里是**同一类问题的兜底**：
    /// 把底色也接上主题，以后新增面板时不会再踩一次。
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        // 用 `panel_fill` 而不是 `window_fill`：两者在 egui 的两套主题里取值相同
        // （深色 gray(27) / 浅色 gray(248)），但"与面板同色"才是这里想要的语义
        // ——露出来的一条要与相邻面板连成一片，而不是像个窗口背景。
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 检查串口接收数据
        self.receive_data();

        // 写日志线程可能已经报错退出：每帧检查一次，不能让界面继续显示「记录中」
        self.poll_log_events();

        // 串口"正在打开"→"已打开"的结算（见 `settle_connection_state`）。
        //
        // 放在 `receive_data` **之后**：这一帧新到的 `Failed` / `Closed` 已经落笔，
        // 结算只会把真正就绪的通道认成已连接，不会把刚报错的又拉回来。
        self.settle_connection_state();

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
        egui::CentralPanel::default().show(ui, |ui| {
            self.render_central_column(ui);
        });
    }

    /// `eframe` 在退出前以及周期性自动保存时调用。
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        crate::config::save(storage, self);
    }
}

impl ComTool {
    /// 中央区那一列：实时图表 → 接收区 → 发送区，自上而下。
    ///
    /// 从 [`ComTool::ui`] 里抽出来的（项目约定：绘制逻辑要能脱离 `eframe::Frame` 被
    /// 无头驱动）——`ui()` 需要 `eframe::Frame`，无头测试造不出来，写在里面等于一次
    /// 也不会被 `cargo test` 画到。抽出来之后，"图表在接收区**之上**"这种纵向顺序
    /// 才有一个能被真实反驳的判据（见 `tests::chart_sits_above_the_receive_area`）。
    pub(crate) fn render_central_column(&mut self, ui: &mut egui::Ui) {
        let s = self.language.strings();

        // 设置 / 信息两个窗口跟着中央区一起绘制（主界面的常驻部分）
        self.show_settings_window(ui);
        self.show_info_window(ui);

        self.render_top_toolbar(ui);

        ui.separator();

        // ① 实时图表（2026-09-24 用户要求移到中央区**顶部**）：它原先夹在接收区与
        //    发送区中间，把同一列切成了三段；放到最上面之后，"看数据"的窗口第一眼
        //    就能看到，也不会再把接收区与发送区从中间截断。
        self.render_chart_panel(ui);

        if self.show_chart_panel && self.show_receive_panel {
            ui.separator();
        }

        if self.show_receive_panel {
            // 接收设置：文本/HEX 用 checkbox 表达（底层互斥，因为收到数据只能按一种格式解析）
            let mut show_hex = self.hex_view;
            let mut show_text = !self.hex_view;
            // 「字节查看」与「HEX 接收」也互斥（见 `resolve_display_mode`）。这里必须
            // 分别记下"哪个是刚被勾上的"，交给那一个函数统一裁决，而不是就地写
            // `if hex { ascii = false; }`——规则两处各写一遍，迟早有一处忘了改。
            let mut hex_just_enabled = false;
            let mut ascii_just_enabled = false;
            let mut color_just_enabled = false;
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
                    // 「颜色」同样要交给互斥裁决：它与「字节查看」同时勾上时接收区会
                    // 一片空白（两者写的不是同一份镜像）。这里只报"刚被勾上"，
                    // 规则由 `resolve_display_mode` 唯一裁决。
                    let mut color_view = self.color_view;
                    if ui.checkbox(&mut color_view, s.color).changed() && color_view {
                        color_just_enabled = true;
                    }
                    self.color_view = color_view;
                    if ui.checkbox(&mut self.ascii_view, s.byte_view).changed() && self.ascii_view {
                        ascii_just_enabled = true;
                    }
                });
            });
            self.hex_view = show_hex; // 文本 = !hex_view；两个都取消时退回文本
            self.resolve_display_mode(hex_just_enabled, ascii_just_enabled, color_just_enabled);

            // 接收区：画哪一份镜像由 [`ComTool::display_mode`] 决定——与上面写镜像的
            // 判定是同一个，所以不存在"写了一份、画另一份"的空视图。分发本身抽在
            // `receive::ComTool::render_receive_area` 里，那条路有自动化测试。
            self.render_receive_area(ui);
        }

        // 中央区里这几块是纵向堆叠的，分隔线只在"上面有东西、下面也有东西"
        // 时才有意义。
        if self.show_receive_panel && self.show_send_panel {
            ui.separator();
        }

        if self.show_send_panel {
            // 发送区
            self.render_send_settings_row(ui);

            // 输入框本体（含右下角那个可拖高的把手）；返回值是"这一帧是否有焦点"，
            // 下面的 Enter 发送要用。
            let send_has_focus = self.render_send_editor(ui);

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
                                    fill(s.unmappable_in_encoding, &[&self.text_encoding.label()]),
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
                // 「发送」是这一区的主操作 ⇒ 绿；「清空」是普通操作 ⇒ 中性灰。
                if crate::button::primary(ui, s.send_title).clicked() {
                    do_send = true;
                }
                if crate::button::standard(ui, s.clear_send_button).clicked() {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communicationtrait::fake::{FakeChannel, FakeHandle};

    /// 实时图表画在中央区的**顶部**，排在接收区之上（2026-09-24 用户要求）。
    ///
    /// ⚠️ 判据是**位置先后**，不是"文字存在"：断言"画出的文字里有「实时图表」"是空转的
    /// ——把它挪回底部照样通过。这里比的是两块各自落笔文字的 y 坐标：图表的页眉必须比
    /// 接收区的内容高。
    ///
    /// ⚠️ 驱动的是 `render_central_column` 而不是 `eframe::App::ui`——后者需要一个无头
    /// 环境构造不出的 `eframe::Frame`。这也正是把它抽出来的理由。
    #[test]
    fn chart_sits_above_the_receive_area() {
        /// 接收区里独有的标记串，用来定位它落在哪一行。
        const MARKER: &str = "RX-MARKER-8f3a";

        let mut app = ComTool {
            show_chart_panel: true,
            show_receive_panel: true,
            show_send_panel: true,
            ..ComTool::default()
        };
        app.receive_text.push_str(MARKER);
        app.chart.push_bytes(b"1,2\n", app.chart_max_samples);

        // `Language::strings()` 给的是 `&'static`，所以下面闭包对 `app` 的可变借用不受影响。
        let header = app.language.strings().chart_panel_label;

        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 900.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                app.render_central_column(ui);
            });
        });

        let mut texts: Vec<(String, f32)> = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(t) = &clipped.shape {
                texts.push((t.galley.text().to_owned(), t.pos.y));
            }
        }
        // 不清理纹理会 panic（与断言无关的清理问题，不是缺陷）。
        out.textures_delta.clear();

        let y_of = |needle: &str| -> f32 {
            texts
                .iter()
                .find(|(text, _)| text == needle)
                .map(|(_, y)| *y)
                .unwrap_or_else(|| panic!("这一帧里没画出文字 {needle:?}——实际画出的有：{texts:?}"))
        };

        let header_y = y_of(header);
        let marker_y = y_of(MARKER);
        assert!(
            header_y < marker_y,
            "图表页眉落在 y={header_y:.1}，接收区内容落在 y={marker_y:.1}——\
             图表不在接收区之上，它被挪回下面了？"
        );
    }

    /// 画一帧中央区（900×900 视口）。
    ///
    /// 事件由调用方给：要测拖动就得在某一帧放 `PointerButton` 按下、下一帧放
    /// `PointerMoved`——egui 的"拖动"是跨帧状态，一帧里按下再移动不算。
    fn draw_central_frame(ctx: &egui::Context, app: &mut ComTool, events: Vec<egui::Event>) {
        draw_central_frame_at(ctx, app, 0.0, events);
    }

    /// 同上，但能指定这一帧的时间戳（模拟双击需要让两帧挨得够近）。
    fn draw_central_frame_at(
        ctx: &egui::Context,
        app: &mut ComTool,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 900.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                app.render_central_column(ui);
            });
        });
        // 不清理纹理会在 drop 时 panic（与断言无关的清理问题，不是缺陷）。
        out.textures_delta.clear();
    }

    /// 这一帧发送框把手落在哪。
    ///
    /// 用显式 id 从 `Context` 读，而不是从 `shapes` 里猜那三道斜线：`TextEdit` 画出
    /// 来的实际高度无头环境拿不到（没有真的窗口），"框变高了没有"只能靠把手的
    /// 位置来量——而把手恰恰是用户眼睛盯着的那个东西。
    fn grip_rect(ctx: &egui::Context) -> egui::Rect {
        match ctx.read_response(egui::Id::new(SEND_RESIZE_GRIP_ID)) {
            Some(response) => response.rect,
            None => panic!("这一帧没有画出发送框把手——把手被删掉了或被挪出了中央区"),
        }
    }

    /// 这一帧**输入框自己**画了多大。
    ///
    /// 与 [`grip_rect`] 是两个不同的东西：把手的位置只反映滚动视口的高度，输入框
    /// 自己可以比视口矮（内容为空、又没把最小高度设成视口高度时就是这样）。用户
    /// 拖到 20 行却只看见 4 行高的白框，正是这条要挡住的缺陷。
    fn send_input_rect(ctx: &egui::Context) -> egui::Rect {
        match ctx.read_response(egui::Id::new(SEND_INPUT_ID)) {
            Some(response) => response.rect,
            None => panic!("这一帧没有画出发送输入框"),
        }
    }

    /// 可见行数调大 ⇒ 发送框真的变高（把手被推下去）。
    ///
    /// ⚠️ 判据是**把手的位移**，不是"界面里有没有这么个控件"：后者把把手画出来却
    /// 不接高度照样通过（那正是"能拖、拖了没反应"的缺陷形态）。
    ///
    /// 阈值 150 px 是字面量：20 行与 4 行差 16 行，标准字号下一行 13 px，差 208 px；
    /// 写 150 是"用户一眼能看出变高了"的量级，不引用 `MAX_SEND_VISIBLE_ROWS`。
    #[test]
    fn send_box_grows_when_the_visible_row_setting_grows() {
        // 两个独立的 `Context`：一次 `run_ui` 之后控件矩形会留在 `Context` 里，
        // 换一帧继续用同一个 `Context` 读到的是更早那次的值。"4 行 vs 20 行"是
        // 两个互不相干的画面，就该各画各的（需要跨帧状态的拖动测试才复用同一个）。
        let ctx = egui::Context::default();
        let mut app = ComTool::default();
        draw_central_frame(&ctx, &mut app, vec![]);
        let short_box_grip = grip_rect(&ctx);
        let short_box = send_input_rect(&ctx);

        let ctx = egui::Context::default();
        app.send_visible_rows = 20;
        draw_central_frame(&ctx, &mut app, vec![]);
        let tall_box_grip = grip_rect(&ctx);
        let tall_box = send_input_rect(&ctx);

        // ① 把手被推下去 = 滚动视口确实变高了
        let grew = tall_box_grip.min.y - short_box_grip.min.y;
        assert!(
            grew > 150.0,
            "可见行数从 4 调到 20，把手只下移了 {grew:.1} px——\
             发送框没有跟着变高，把手是画上去的摆设"
        );

        // ② 输入框**自己**也要长到 20 行（内容为空时同样如此）。
        //    少了这一条，"视口变高、白框仍是 4 行"能悄悄溜过去——用户拖完看到的
        //    就是一个矮框加一大片空白。
        assert!(
            tall_box.height() > 200.0,
            "调到 20 行，输入框自身只有 {:.1} px 高——它没跟着长，\
             用户拖完看到的是矮框配一块空白",
            tall_box.height()
        );
        assert!(
            short_box.height() < 100.0,
            "默认 4 行的输入框不该有 {:.1} px 高——'多行预览'若对默认值也生效，\
             老用户一升级界面就变了样",
            short_box.height()
        );
    }

    /// 草稿再长也不把发送框撑大：多出来的部分在框内滚动。
    ///
    /// 这条守的是"视口高度由用户说了算"——没有它，粘一段 30 行的文本进来就会把
    /// 「发送 / 清空」按钮和常用数据挤出可视区（中央区没有整体滚动，挤出去就点
    /// 不着）。把手的 y 不变，正是"视口没被内容撑开"的可量形式。
    #[test]
    fn a_long_draft_scrolls_instead_of_pushing_the_send_box_down() {
        let draft: String = (1..=30).map(|line| format!("line {line}\n")).collect();

        let ctx = egui::Context::default();
        let mut app = ComTool::default();
        draw_central_frame(&ctx, &mut app, vec![]);
        let empty_draft_grip_y = grip_rect(&ctx).min.y;

        let ctx = egui::Context::default();
        let mut app = ComTool {
            send_text: draft,
            ..ComTool::default()
        };
        draw_central_frame(&ctx, &mut app, vec![]);
        let long_draft_grip_y = grip_rect(&ctx).min.y;

        assert!(
            (long_draft_grip_y - empty_draft_grip_y).abs() < 4.0,
            "30 行草稿把把手从 y={empty_draft_grip_y:.1} 推到了 y={long_draft_grip_y:.1}——\
             视口被内容撑开了，长文本会把下面的按钮挤出屏幕"
        );
    }

    /// 真的按住把手往下拖 ⇒ 可见行数变多。
    ///
    /// 判据是**状态被拖动改了**，不是"拖动的事件送进去了"：egui 那边的拖动态由
    /// 上一帧的控件矩形决定，这里走的是与真实鼠标完全相同的输入路径。
    #[test]
    fn dragging_the_grip_down_raises_the_visible_rows() {
        let ctx = egui::Context::default();
        let mut app = ComTool::default();
        assert_eq!(app.send_visible_rows, 4, "默认 4 行，与加把手之前一致");

        draw_central_frame(&ctx, &mut app, vec![]);
        let start = grip_rect(&ctx).center();

        // 第一帧：把指针移到把手上并按下（按下发生在**上一帧**的矩形上，
        // 所以这一帧之前必须先画过一帧）。
        draw_central_frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        // 第二帧：往下拖 150 px。
        draw_central_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(egui::pos2(
                start.x,
                start.y + 150.0,
            ))],
        );

        assert!(
            app.send_visible_rows >= 10,
            "往下拖了 150 px，可见行数只到 {} —— 拖动没有换算成高度",
            app.send_visible_rows
        );
    }

    /// 往上拖 ⇒ 变矮，且不会拖成 0（下限 2 行）。
    #[test]
    fn dragging_the_grip_up_shrinks_the_visible_rows() {
        let ctx = egui::Context::default();
        let mut app = ComTool {
            send_visible_rows: 12,
            ..ComTool::default()
        };

        draw_central_frame(&ctx, &mut app, vec![]);
        let start = grip_rect(&ctx).center();
        draw_central_frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        // 往上拖 500 px：远超 12 行，应当一路撞到下限
        draw_central_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(egui::pos2(
                start.x,
                start.y - 500.0,
            ))],
        );

        assert_eq!(
            app.send_visible_rows, 2,
            "拖过头也必须停在下限 2 行，不能把输入框压成一条缝"
        );

        // 中途量一次：小幅上拖要按行递减，而不是一步到底
        let mut app = ComTool {
            send_visible_rows: 12,
            ..ComTool::default()
        };
        let ctx = egui::Context::default();
        draw_central_frame(&ctx, &mut app, vec![]);
        let start = grip_rect(&ctx).center();
        draw_central_frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        draw_central_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(egui::pos2(
                start.x,
                start.y - 39.0,
            ))],
        );
        // 判据写**区间**而不是精确值：一行的高度是字体的实际度量（Monospace
        // 基准 13 px 上下，随字体档位变），39 px 折算 2~3 行。精确到某一行会把
        // "字号改了"误判成缺陷。
        let shrunk = 12 - app.send_visible_rows;
        assert!(
            (2..=4).contains(&shrunk),
            "往上拖 39 px 应当缩掉 2~3 行，实际缩了 {shrunk} 行（剩 {} 行）",
            app.send_visible_rows
        );
    }

    /// 把手横跨发送框、贴在右侧——用户会在右下角找它。
    ///
    /// 判据是**几何位置**而不是"画出来了"：把手画在左边缘的话，这条会红。
    #[test]
    fn resize_grip_sits_at_the_bottom_right_of_the_send_box() {
        let ctx = egui::Context::default();
        let mut app = ComTool::default();
        draw_central_frame(&ctx, &mut app, vec![]);
        let grip = grip_rect(&ctx);

        assert!(
            grip.width() > 700.0,
            "把手只有 {:.1} px 宽——它该横跨整个发送框，不然右下角根本压不中",
            grip.width()
        );
        assert!(
            grip.right() > 700.0,
            "把手的右边缘在 x={:.1}（视口宽 900）——它该贴着右边，而不是缩在左侧",
            grip.right()
        );
        assert!(
            grip.height() <= 12.0,
            "把手高 {:.1} px，太占版面了——它是一条细把手，不是一块面板",
            grip.height()
        );
    }

    /// 双击把手 ⇒ 回到默认高度（拖到某个奇怪的高度之后总要有条退路）。
    #[test]
    fn double_clicking_the_grip_restores_the_default_height() {
        let ctx = egui::Context::default();
        let mut app = ComTool {
            send_visible_rows: 20,
            ..ComTool::default()
        };
        draw_central_frame(&ctx, &mut app, vec![]);
        let p = grip_rect(&ctx).center();

        let press = |pressed: bool| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        // 两轮"按下→松开"，间隔远小于 egui 的双击判定窗口
        draw_central_frame_at(&ctx, &mut app, 1.00, vec![press(true)]);
        draw_central_frame_at(&ctx, &mut app, 1.05, vec![press(false)]);
        draw_central_frame_at(&ctx, &mut app, 1.10, vec![press(true)]);
        draw_central_frame_at(&ctx, &mut app, 1.15, vec![press(false)]);

        assert_eq!(
            app.send_visible_rows, 4,
            "双击把手应当回到默认 4 行，实际是 {} 行",
            app.send_visible_rows
        );
    }

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
        // 造一个**真的**断开：只把 `connected` 置假而通道仍就绪，是"记账位与通道
        // 不一致"的非法状态（现实中不存在——断开要嘛走 `close_connection`、要嘛
        // 有 `Failed`/`Closed` 事件把通道一起带下来）。发送闸门看的是通道是否就绪
        // （见 `channel_is_ready` 的注释），所以这里也必须让通道跟着断开。
        app.close_connection();
        app.send_text = "ping".to_string();
        // `close_connection` 会往接收区留一条提示，清掉以免干扰下面的断言。
        app.receive_text.clear();

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

    /// ⭐ **串口打开请求返回后，界面不得自称"已连接"**——本次修的缺陷本体。
    ///
    /// 用替身的"异步就绪"模式扮串口：`open()` 只是拉起线程，端口还没打开。
    /// 旧实现在这里会无条件 `connected = true`，于是界面先亮一下"已连接"、等
    /// `Failed` 回来才跳回"未连接"——用户在那一瞬点发送，字节交给一个不存在的
    /// 端口。这条断言的是**那一刻的判定口径**，不是最终状态。
    #[test]
    fn serial_open_does_not_claim_connected_before_the_port_is_ready() {
        let (channel, handle) = FakeChannel::new();
        // 必须在 open 之前置位：`open()` 读它决定初始的 active。
        handle.defer_ready();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            send_text: "A5".to_string(),
            ..ComTool::default()
        };

        app.open_connection();
        assert!(
            app.opening(),
            "串口 open 返回时端口还没打开，必须处于「正在打开」而不能是「已连接」"
        );
        assert!(
            !app.send_data(),
            "「正在打开」期间不得放行发送——这正是假阳性缺陷会造成的数据丢失"
        );
        assert_eq!(app.tx_bytes, 0, "没就绪就不该记账已发送字节");

        // 后台线程随后把端口打开了。
        handle.signal_ready();
        app.settle_connection_state();
        assert!(
            !app.opening() && app.connected,
            "端口就绪之后结算应当把它认成已连接"
        );
        assert!(
            app.receive_text.contains("已打开"),
            "结算时要补一条「已打开」，实际接收区：{}",
            app.receive_text
        );
    }

    /// 「正在打开」期间**不得**放行发送，且不得记账。
    ///
    /// 与上一条是一对：那条看状态口径，这条看**发送闸门**。把闸门留在
    /// `if !self.connected` 上时，这个测试会红——`connected` 在"正在打开"时已经是真。
    #[test]
    fn sending_is_blocked_while_the_port_is_still_opening() {
        let (channel, handle) = FakeChannel::new();
        handle.defer_ready();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            send_text: "A5".to_string(),
            ..ComTool::default()
        };

        app.open_connection();
        assert!(app.opening(), "前提：此刻应当处于「正在打开」");

        assert!(
            !app.send_data(),
            "「正在打开」时发送必须返回 false——端口还没打开，字节没有去处"
        );
        assert_eq!(handle.write_count(), 0, "一个字节都不该交给通道");
        assert_eq!(app.tx_bytes, 0, "没发出去就不该记账");
        assert!(
            !app.send_preset(0),
            "常用数据走同一条闸门，也不能在「正在打开」时放行"
        );
    }

    /// 通道**不就绪**绝不等于"掉线"——结算只升不降。
    ///
    /// 这条防的是最省事的错误实现：把 `settle_connection_state` 写成"每帧把
    /// `connected` 对齐成 `channel.is_active()`"。那样一来，任何一次通道状态
    /// 波动都会让界面跳成"未连接"、还会停掉定时发送，而 `Failed` / `Closed`
    /// 事件紧接着再报一次"连接已关闭"——同一次掉线被说了两遍。
    #[test]
    fn settling_never_downgrades_an_established_connection() {
        let (mut app, handle) = connected_app();

        // 造一个"通道报已就绪、界面也认已连接"的稳态。
        assert!(app.connected && !app.opening());

        // 通道挂了，但**事件还没到**（这正是事件通道的意义：只有它知道为什么）。
        handle.emit(ChannelEvent::Failed("模拟".to_string()));
        // 先不谈事件，直接结算：不许把 connected 拉下来。
        app.settle_connection_state();
        assert!(
            app.connected,
            "结算只该升不该降：降了就会与随后到来的 Failed 事件重复报告同一次掉线"
        );
    }

    /// 通道类型与连接类型**不一致**时，结算的用词按 `connection_kind`。
    ///
    /// 复用同一条通道对象时 `channel_kind` 与 `connection_kind` 恒等，看不出差别；
    /// 这条故意造出不一致，钉住"结算说哪种传输"的依据。用 `transport_noun()`
    /// 反查最省事：它读的就是 `connection_kind`。
    ///
    /// 不调 `open_connection`（那会因为类型不一致而把替身拆掉、换成真 TCP 通道），
    /// 而是把状态摆成"已发起打开 + 带替身的通道 + 另一个连接类型"——这正是结算
    /// 函数唯一需要的三个前提。
    #[test]
    fn settle_wording_follows_the_connection_kind_not_the_channel_kind() {
        let (channel, handle) = FakeChannel::new();
        handle.defer_ready();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            // 通道对象是"按串口造的"，但当前选择的传输是 TCP。
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::TcpClient,
            net_remote_host: "127.0.0.1".to_string(),
            net_remote_port: 1234,
            ..ComTool::default()
        };

        // 已发起打开、提示还没补过——结算的进入条件。
        app.connected = true;
        app.announced_open = false;
        // 端口就绪。
        handle.signal_ready();
        app.settle_connection_state();

        assert_eq!(app.transport_noun(), "TCP");
        assert!(
            app.receive_text.contains("TCP已打开"),
            "结算的措辞必须按 connection_kind（TCP），而不是 channel_kind（串口），\
             实际接收区：{}",
            app.receive_text
        );
        assert!(
            !app.receive_text.contains("串口已打开"),
            "用错了依据就会把 TCP 说成串口，实际接收区：{}",
            app.receive_text
        );
    }

    /// 结算那条提示**不能每帧刷**。
    ///
    /// 通道就绪是一个持续状态（会为真很多帧），没有记账位的话"已打开"会被反复
    /// 写进接收区。这条连调两次，断言第二次不再追加。
    #[test]
    fn settle_announces_open_only_once_per_connection() {
        let (channel, handle) = FakeChannel::new();
        handle.defer_ready();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            ..ComTool::default()
        };

        app.open_connection();
        handle.signal_ready();

        app.settle_connection_state();
        let after_first = app.receive_text.matches("已打开").count();

        // 再结算两帧。
        app.settle_connection_state();
        app.settle_connection_state();
        let after_more = app.receive_text.matches("已打开").count();

        assert!(after_first > 0, "第一次结算应当补一条「已打开」");
        assert_eq!(
            after_first, after_more,
            "通道就绪会持续很多帧，结算不能每帧都补一条「已打开」（接收区：{}）",
            app.receive_text
        );
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

    // —— 自动重连退避 ——

    /// 纯函数的退避曲线：首轮 500 ms，之后翻倍，封顶 30 s。
    ///
    /// 断言的是**绝对值**而不是"后者不小于前者"——只看单调性的话，把基准从
    /// 500 ms 改成 5 s 也会通过，而那会让正常重连的体感明显变差。
    #[test]
    fn reconnect_backoff_doubles_until_capped() {
        assert_eq!(reconnect_backoff(0), Duration::from_millis(500));
        assert_eq!(reconnect_backoff(1), Duration::from_millis(1000));
        assert_eq!(reconnect_backoff(2), Duration::from_millis(2000));
        assert_eq!(reconnect_backoff(3), Duration::from_millis(4000));
        assert_eq!(reconnect_backoff(4), Duration::from_millis(8000));
        assert_eq!(reconnect_backoff(5), Duration::from_millis(16000));
        // 第 6 次会算到 32 s，被上限截到 30 s。
        assert_eq!(reconnect_backoff(6), RECONNECT_MAX_DELAY);
        assert_eq!(reconnect_backoff(30), RECONNECT_MAX_DELAY, "再大也只到上限");
        // 极端值不得 panic 或溢出。
        assert_eq!(reconnect_backoff(u32::MAX), RECONNECT_MAX_DELAY);
    }

    /// `Failed` 事件必须让连续失败计数递增——退避的输入就是它。
    ///
    /// 若这条计数不涨，退避会永远停在首轮 500 ms，等于退避根本没生效。
    #[test]
    fn channel_failure_increments_reconnect_failures() {
        let (mut app, handle) = connected_app();

        assert_eq!(app.reconnect_failures, 0, "刚连上时应为 0");
        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();
        assert_eq!(app.reconnect_failures, 1, "一次失败应记一次");

        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();
        assert_eq!(app.reconnect_failures, 2, "两次失败应累加");
    }

    /// 收到数据 = 连接真的通了，连续失败计数必须清零。
    ///
    /// 这是"串口乐观置位"下唯一可靠的**成功**证据：`connected` 在串口侧是
    /// 提前置 true 的，光看它无法区分"打开了"与"待会儿才失败"。
    #[test]
    fn receiving_bytes_resets_reconnect_failures() {
        let (mut app, handle) = connected_app();

        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();
        assert_eq!(app.reconnect_failures, 1);

        handle.emit(ChannelEvent::Bytes(b"hello".to_vec()));
        app.receive_data();
        assert_eq!(
            app.reconnect_failures, 0,
            "收到数据即证明连接可用，计数应清零"
        );
    }

    /// 连续失败到上限后，自动重连**彻底停手**，且恰好在跨过阈值时提示一次。
    #[test]
    fn reconnect_gives_up_after_max_attempts() {
        let (mut app, handle) = connected_app();

        for _ in 0..MAX_RECONNECT_ATTEMPTS {
            handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
            app.receive_data();
        }
        assert_eq!(app.reconnect_failures, MAX_RECONNECT_ATTEMPTS);
        assert!(
            app.receive_text.contains("自动重连已停止"),
            "到顶时必须明确告诉用户不再自动重试：{}",
            app.receive_text
        );

        // 再失败一次也不再多提示（提示只在跨过阈值那一刻出现一次）。
        let note_count_before = app.receive_text.matches("自动重连已停止").count();
        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();
        assert_eq!(
            app.receive_text.matches("自动重连已停止").count(),
            note_count_before,
            "「已停止」提示不应每帧重复刷屏"
        );
    }

    /// 用户主动断开应把退避计数清零——下一轮连接从干净的 500 ms 开始。
    #[test]
    fn explicit_close_resets_reconnect_failures() {
        let (mut app, handle) = connected_app();

        handle.emit(ChannelEvent::Failed("设备已拔出".to_string()));
        app.receive_data();
        assert!(app.reconnect_failures > 0);

        app.close_connection();
        assert_eq!(app.reconnect_failures, 0, "主动断开后应清零");
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

    // ——— 窗口底色 ———

    /// **窗口底色必须跟着主题走**（面板之外露出来的那层）。
    ///
    /// eframe 的默认实现返回一个**写死的深灰** `rgba(12,12,12,180)`，参数名就叫
    /// `_visuals`——**拿得到主题却故意不用**。浅色主题下，任何没被面板盖住的区域
    /// 都会露出这条深色；2026-09-24 用户目视发现的「状态栏在浅色下没变色」正是
    /// 这条路径（状态栏当时用 `Frame::NONE` 自己不画背景）。状态栏已经单独修了
    /// （见 `status_bar::tests::status_bar_background_follows_the_theme`），
    /// 这条测试守的是同一类问题的**兜底**。
    ///
    /// 断言"底色 == 该主题的面板底色"而不是"两色不同"：后者会被"换个写死的浅灰"
    /// 混过去（深色主题下就露馅）。
    #[test]
    fn window_clear_color_follows_the_theme() {
        // `clear_color` 是 `eframe::App` 的方法，测试里要显式把 trait 带进来
        // （生产代码不需要：`ui()` 就在那个 impl 块里）。
        use eframe::App as _;

        let app = ComTool::default();

        let light = app.clear_color(&egui::Visuals::light());
        let dark = app.clear_color(&egui::Visuals::dark());

        assert_eq!(
            light,
            egui::Visuals::light().panel_fill.to_normalized_gamma_f32(),
            "浅色主题下窗口底色必须是浅色面板底色，不能是 eframe 默认的那个深灰"
        );
        assert_eq!(
            dark,
            egui::Visuals::dark().panel_fill.to_normalized_gamma_f32(),
            "深色主题下窗口底色必须是深色面板底色"
        );
        assert_ne!(
            light, dark,
            "两套主题的窗口底色完全相同 ⇒ 说明它读到的根本不是主题"
        );
    }
}
