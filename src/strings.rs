//! 界面文案表（F3 起）。
//!
//! # 为什么不是 gettext
//!
//! gettext 的查找发生在**运行期**：漏翻译的地方不会编译失败，只会在界面上露出
//! 一句原语言。这个项目宁愿要"编译期就过不去"。所以这里是一张**编译期**的表：
//! 每条文案都是 `Strings` 的一个命名字段，中英两列写在同一个宏臂里。
//!
//! # 与路标里写法的差异（有意为之）
//!
//! `FUTURE_ROADMAP.md` §4 写的是"`struct Strings` + 两个 `static`"，那样新增字段时
//! 两个实例都要补齐，**漏一个**是编译错误。这里用 [`strings!`] 把中英两列写成**同一条
//! 宏臂的两个值**：漏翻译不再是"编译错误"，而是**根本写不出来**——比原方案更强，
//! 且中英对照就并排放在一行，review 时不必来回翻两处。
//!
//! # 哪些文案**不**翻译
//!
//! 只翻译 **UI 控件文案**。接收区里由程序写出的提示行（"串口已关闭"、"发送失败: …"、
//! "已导出到: …"）与实时日志的内容**保持固定中文**，理由见 `FUTURE_ROADMAP.md` §4：
//! 这些文字会同步写进实时日志文件与导出文件，若随界面语言变化，同一台设备在两个
//! 语言设置下产出的两份日志就不可比、也不能被 grep 脚本处理。
//!
//! 因此本文件里**没有**那几条提示行的字段。看到 `receive.rs` / `app.rs` / `log_file.rs`
//! 里仍然硬编码中文的 `push_receive_note("串口已关闭\n")` 时，那是决策，不是漏改。
//!
//! # 带占位符的文案
//!
//! `format!` 的格式串**必须是字面量**（变量会报 `format argument must be a string
//! literal`），所以带参数的文案不能直接 `format!(s.xxx, arg)`。改用 [`fill`]：
//! 模板里的 `{}` 按顺序被实参替换。这条限制正是 `fill` 存在的原因。

use crate::app::Language;
use crate::receive::TimestampPrefix;

/// 中英两列的文案表。
///
/// 每条一行：`字段名: "中文", "English";`。带占位符的文案用 `{}`，由 [`fill`] 填充。
/// 长文案可以把两列各写一行（宏臂里的换行只是排版）。
macro_rules! strings {
    ($( $field:ident: $zh:literal, $en:literal; )*) => {
        /// 一个语言的界面文案。字段名即用途；取值只从 [`ZH`] / [`EN`] 里取。
        pub(crate) struct Strings {
            $( pub(crate) $field: &'static str, )*
        }

        /// 简体中文文案（默认）。切到中文时界面与 F3 之前**逐字一致**
        /// （唯一例外是设置窗口里那句"语言选项尚未接入界面文案，仅作占位"，
        /// 它描述的事实已经不成立，见 [`Strings::language_applies_immediately`]）。
        pub(crate) static ZH: Strings = Strings { $( $field: $zh, )* };

        /// 英文文案。
        pub(crate) static EN: Strings = Strings { $( $field: $en, )* };

        /// `(字段名, 中文, 英文)` 三列，**仅供测试**遍历用。
        #[cfg(test)]
        pub(crate) const ALL: &[(&str, &str, &str)] =
            &[ $( (stringify!($field), $zh, $en), )* ];
    };
}

strings! {
    // —— app.rs：顶栏按钮 ——
    sidebar_toggle: "☰ 侧边栏", "☰ Sidebar";
    settings_button: "⚙ 设置", "⚙ Settings";

    // —— app.rs：接收设置行 ——
    receive_settings: "接收设置:", "Receive:";
    color: "颜色", "Color";
    byte_view: "字节查看", "Byte view";

    // —— app.rs：发送区 ——
    send_area: "发送区:", "Send:";
    format: "格式", "Format";
    line_ending: "行尾", "Line ending";
    repeat_send: "定时发送", "Repeat send";
    preview_too_long: "输入过长，已停止实时校验", "Input too long — live validation paused";
    will_send_bytes: "将发送 {} 字节", "Will send {} bytes";
    unmappable_in_encoding:
        "有字符在 {} 里无法表示，将被替换",
        "Some characters cannot be represented in {} and will be replaced";
    hex_parse_failed: "HEX 解析失败: {}", "HEX parse failed: {}";
    history_browsing: "历史浏览中：↑ 更旧 / ↓ 更新", "Browsing history: ↑ older / ↓ newer";
    clear_send_button: "清空发送", "Clear input";
    repeat_needs_connection:
        "未连接：连上之后才会开始定时发送",
        "Not connected — repeat send only starts once connected";
    escape: "转义", "Escape";

    // —— app.rs：常用数据（N2）——
    presets_header: "常用数据", "Presets";
    save_preset: "存为常用", "Save as preset";
    preset_empty:
        "还没有常用数据：把输入框里的内容存进来，之后点一下就能重发",
        "No presets yet — save the input box contents, then send them with one click";
    preset_hint: "点一下发送；右键删除", "Click to send; right-click to delete";
    preset_full: "上限 {} 条", "Up to {} presets";
    preset_duplicate: "已在列表中", "Already saved";
    preset_input_empty: "输入框为空", "Input is empty";
    preset_count_text: "{} / {} 条", "{} / {} saved";
    clear_presets: "清空常用数据", "Clear presets";
    preset_needs_connection: "未连接：连上之后才能发送", "Not connected — cannot send yet";

    // —— app.rs / log_file.rs：原生文件对话框 ——
    export_dialog_title: "导出接收数据.txt", "Export received data.txt";
    export_text_filter: "文本文件", "Text files";
    pick_log_dir_title: "选择日志输出目录", "Choose log output directory";

    // —— info.rs：关于窗口 ——
    info_window_title: "信息", "Info";
    version_label: "版本 {}", "Version {}";
    app_description:
        "基于 eframe/egui 的串口调试上位机",
        "Serial port terminal built on eframe/egui";
    serial_backend: "串口后端: serialport", "Serial backend: serialport";
    ui_framework: "界面框架: eframe / egui", "UI framework: eframe / egui";
    settings_dir: "设置存档: {}", "Settings dir: {}";
    settings_dir_unavailable:
        "设置存档: 当前平台不适用",
        "Settings dir: not applicable on this platform";
    shortcuts: "快捷键", "Shortcuts";
    shortcut_send: "Enter 发送 / Shift+Enter 换行", "Enter to send / Shift+Enter for a newline";
    shortcut_copy: "接收区右键：复制全部日志", "Receive area, right-click: copy the whole log";

    // —— sidebar.rs：只读信息回显 ——
    nav_sidebar: "侧边栏", "Sidebar";
    collapse: "收起", "Collapse";
    serial_info: "串口信息", "Port info";
    port_label: "端口: {}", "Port: {}";
    not_selected: "未选择", "not selected";
    baud_label: "波特率: {}", "Baud rate: {}";
    status_connected: "状态: 已连接", "Status: connected";
    status_disconnected: "状态: 未连接", "Status: disconnected";
    rx_stats: "接收统计", "Receive stats";
    rx_bytes_label: "接收字节数: {}", "Received bytes: {}";
    tx_bytes_label: "发送字节数: {}", "Sent bytes: {}";
    tx_bytes_hover:
        "自本次连接以来累计送出，断开后归零；与上面「接收字节数」口径不同——那个是当前缓冲区里的字节数，会被上限裁掉",
        "Counted since this connection and reset on disconnect; note the different meaning from \"Received bytes\" above, which is the size of the current buffer and is trimmed at the cap";
    display_mode_line: "显示模式: {}", "Display mode: {}";
    suffix_color: " + 颜色", " + color";
    suffix_byte_view: " + 字节查看", " + byte view";
    timestamp_prefix_line: "时间戳前缀: {}", "Timestamp prefix: {}";
    available_ports: "可用串口", "Available ports";
    none_placeholder: "（无）", "(none)";

    // —— status_bar.rs：底部状态栏 ——
    // 这里刻意是**短词**（"已连接" / "未连接"），与侧边栏那套 "状态: 已连接" 分开写：
    // 侧边栏是一行行读数（前缀"状态:"有助于扫读），状态栏是常驻指示，前缀是噪音。
    // 两者必须给出**同一结论**，所以词义按同一套四态来（含 UDP 的"已绑定/未绑定"，
    // 见 `ConnectionKind` 的文档：UDP 没有连接，说"已连接"会让用户以为对端在线）。
    statusbar_connected: "已连接", "Connected";
    statusbar_disconnected: "未连接", "Not connected";
    statusbar_bound: "已绑定", "Bound";
    statusbar_unbound: "未绑定", "Not bound";
    // 圆点**不是一个字符**，是用 `Color32` 画的实心圆（见 `status_bar.rs`）。
    // 早期版本用 🟢/🔴 两个 emoji，但 egui 的文本管线只光栅化字体的**矢量轮廓**，
    // 拿不到 COLR/CPAL 彩色图层，结果是两个大小不一、颜色灰黑的圆——不是字体选错，
    // 是根本渲染不了彩色字形。整个状态栏已不依赖任何 emoji 字体。

    // —— sidebar.rs：连接参数控件 ——
    port_field: "端口", "Port";
    select_port: "请选择端口", "Select a port";
    refresh: "刷新", "Refresh";
    baud_field: "波特率", "Baud rate";
    baud_custom_entry: "自定义…", "Custom…";
    baud_custom_hint: "可输入 {} ~ {} 之间的任意值", "Any value from {} to {}";
    line_state_header: "控制线", "Control lines";
    line_state_hint:
        "连接后可手动控制；打开端口本身不会去动这两根线",
        "Controllable while connected; opening a port never touches these lines";
    line_state_hover:
        "勾选表示「我们请求的电平」，不是对引脚的测量——驱动只接受写、不回读，所以硬件把线拉到哪里，界面看不到",
        "The checkbox is the level we requested, not a measurement — the driver only accepts writes and cannot read the lines back, so the UI cannot show where the hardware actually pulled them";
    open_port_button: "打开串口", "Open";
    close_port_button: "关闭串口", "Close";
    data_bits: "数据位", "Data bits";
    stop_bits: "停止位", "Stop bits";
    parity: "校验", "Parity";
    flow_control: "流控", "Flow control";

    // —— sidebar.rs：连接类型与网络参数（F4）——
    connection_type: "连接类型", "Connection";
    conn_serial: "串口", "Serial";
    conn_tcp: "TCP 客户端", "TCP client";
    conn_udp: "UDP", "UDP";
    connection_info: "连接信息", "Connection info";
    target_label: "目标: {}", "Target: {}";
    remote_host_field: "远端地址", "Remote host";
    remote_host_hint: "IP 或域名", "IP or hostname";
    remote_port_field: "远端端口", "Remote port";
    local_port_field: "本地端口", "Local port";
    local_port_line: "本地端口: {}", "Local port: {}";
    local_port_hint: "0 = 由系统分配", "0 = assigned by the system";
    connect_button: "连接", "Connect";
    disconnect_button: "断开", "Disconnect";
    bind_button: "绑定", "Bind";
    status_bound: "状态: 已绑定", "Status: bound";
    status_unbound: "状态: 未绑定", "Status: not bound";
    switch_kind_needs_disconnect:
        "连接中不能切换连接类型：请先断开，否则界面显示的参数与实际打开的通道会对不上",
        "Cannot switch the connection type while connected — disconnect first, or the shown parameters will not match the open channel";
    need_port_to_connect: "请先选择端口", "Select a port first";
    need_target_to_connect: "请先填写目标地址与端口", "Enter a target host and port first";

    // —— sidebar.rs：日志操作 ——
    clear_log: "清空日志", "Clear log";
    export_log: "导出日志", "Export log";
    file_log: "文件日志", "File log";
    stop_logging: "停止记录", "Stop logging";
    start_logging: "开始记录", "Start logging";
    pick_dir: "选择目录", "Choose dir";
    logged_bytes: "已记录 {} 字节", "{} bytes logged";
    not_logging: "未记录", "Not logging";
    output_dir_line: "输出目录: {}", "Output dir: {}";

    // —— panels/chart.rs：图表面板 ——
    chart_panel_label: "实时图表:", "Realtime chart:";
    auto_bounds: "自动缩放", "Auto bounds";
    clear_chart: "清空图表", "Clear chart";
    chart_summary:
        "{} 通道 · {} 样本 · 每通道上限 {}",
        "{} channels · {} samples · max {} each";
    ignored_fields: "忽略 {} 个非数值字段", "ignored {} non-numeric fields";
    discarded_segments: "丢弃 {} 段超长数据", "dropped {} oversized segments";
    chart_format_hint:
        "{}（格式：一行一组样本，字段用逗号分隔，例如 1.0,2.0,3.0）",
        "{} (format: one sample group per line, comma-separated fields, e.g. 1.0,2.0,3.0)";
    waiting_data: "等待数据…", "Waiting for data…";

    // —— panels/menu_bar.rs：菜单 ——
    menu_file: "文件", "File";
    menu_info: "信息", "Info";
    menu_quit: "退出", "Quit";
    settings_title: "设置", "Settings";
    menu_open_settings: "打开设置窗口", "Open settings window";
    theme_label: "主题", "Theme";
    menu_view: "视图", "View";
    panel_label: "面板", "Panels";
    display_mode_heading: "显示模式", "Display mode";
    auto_scroll: "自动滚动", "Auto scroll";
    text: "文本", "Text";
    hex: "HEX", "HEX";
    encoding_shared_label: "编码（接收与发送共用）", "Encoding (shared by receive and send)";
    encoding_gbk_note_menu:
        "GBK / GB18030 接收结果相同，仅发送时的可表示字符范围不同",
        "GBK and GB18030 decode identically; only the encodable character range differs";
    send_title: "发送", "Send";
    hex_receive: "HEX 接收", "HEX receive";
    nav_receive: "接收区", "Receive";
    nav_chart: "实时图表", "Realtime chart";
    nav_send: "发送区", "Send";
    at_least_one_panel: "至少保留一个面板", "At least one panel must stay visible";
    theme_system: "跟随系统", "System";
    theme_light: "浅色", "Light";
    theme_dark: "深色", "Dark";
    ts_off: "关闭", "Off";
    ts_per_line: "每行", "Per line";
    ts_per_batch: "每批", "Per batch";
    ending_none: "无", "None";
    ending_lf: "LF", "LF";
    ending_crlf: "CRLF", "CRLF";

    // —— panels/menu_bar.rs：发送历史 ——
    send_history_label: "发送历史", "Send history";
    no_send_history: "还没有发送记录", "No sends yet";
    send_history_count: "发送历史（{} / {}）", "Send history ({} / {})";
    empty_line: "<空行>", "<empty line>";
    clear_history: "清空历史", "Clear history";

    // —— panels/settings.rs：界面 ——
    sec_interface: "界面", "Interface";
    language_label: "语言", "Language";
    language_applies_immediately:
        "切换语言立即生效，不需要重启",
        "Switching language takes effect immediately, no restart needed";
    font_scale_label: "字体大小", "Font size";
    font_scale_small: "小", "Small";
    font_scale_normal: "标准", "Normal";
    font_scale_large: "大", "Large";
    font_scale_xlarge: "特大", "Extra large";
    font_scale_note:
        "整体缩放界面文字（标题、正文、按钮、等宽一起变），立即生效；接收区与发送框的等宽字号也跟着变。\
         若调到「特大」后某些标签显得拥挤，属已知情况——它遇到的是界面里固定像素的标签列宽度",
        "Scales all interface text together (headings, body, buttons, monospace), effective immediately; \
         the monospace size in the receive area and send box follows too. If some labels look cramped at \
         \"Extra large\", that is expected — they run into the fixed-pixel label column widths";
    font_scale_preview: "预览：{}", "Preview: {}";

    // —— panels/settings.rs：文本编码 ——
    sec_text_encoding: "文本编码", "Text encoding";
    encoding_label: "编码", "Encoding";
    encoding_shared_note:
        "接收解码与发送编码共用这一档：不会出现「收到的中文按 GBK 解出来、发出去的却是 UTF-8」",
        "Receive decoding and send encoding share this setting: you will never decode incoming Chinese as GBK while sending out UTF-8";
    encoding_gbk_note:
        "GBK 与 GB18030 的接收结果相同（共用解码器），仅发送时的可表示字符范围不同",
        "GBK and GB18030 decode identically (same decoder); only the encodable character range differs";
    encoding_switch_note:
        "切换到别的编码会丢弃上一批残留的半个字符，已经显示的内容不受影响",
        "Switching encoding discards a half character left over from the previous batch; text already shown is unaffected";

    // —— panels/settings.rs：接收 ——
    sec_receive: "接收", "Receive";
    timestamp_prefix_field: "时间戳前缀", "Timestamp prefix";
    ts_note:
        "形如 [12:00:00.123] 的本地时间前缀；每行 = 每个换行后重新标注，每批 = 每次收到数据标注一次",
        "Local time prefix such as [12:00:00.123]; per line = re-stamped after every newline, per batch = stamped once per received chunk";
    ts_note_2:
        "前缀是显示注解而不是收到的数据：它会随接收区一起进导出文件与实时日志，但不会进入图表解析",
        "The prefix is a display annotation, not received data: it enters exports and the live log along with the receive area, but never chart parsing";
    max_log_field: "日志长度上限", "Max log length";
    chars_unlimited: "字符（0 = 不限）", "characters (0 = unlimited)";
    max_log_note:
        "超出上限时从最旧的内容开始淘汰，三种接收视图同步裁剪",
        "When over the limit the oldest content is dropped; all three receive views are trimmed together";
    auto_scroll_latest: "自动滚动到最新", "Auto scroll to newest";
    repaint_field: "重绘间隔", "Repaint interval";
    repaint_note:
        "ms（16~1000，越小越跟手也越耗电）",
        "ms (16~1000; smaller is snappier and uses more power)";

    // —— panels/settings.rs：发送 ——
    payload_format: "负载格式", "Payload format";
    hex_note:
        "HEX 时输入按十六进制解析：允许空格 / 逗号分隔与 0x 前缀；解析失败不会发送，也不会清空输入框",
        "In HEX mode the input is parsed as hexadecimal: spaces/commas and a 0x prefix are allowed; a parse failure sends nothing and does not clear the input";
    escape_note:
        "转义时 \\r \\n \\t \\0 \\xNN \\\\ 会变成对应的字节，其余文字仍按上面的编码发送；认不出的转义不会发送，也不会清空输入框",
        "In escape mode \\r \\n \\t \\0 \\xNN \\\\ become raw bytes and the remaining text is still encoded as above; an unrecognised escape sends nothing and does not clear the input";
    ending_note:
        "追加在字节层：HEX 模式下发出的是 0x0A / 0x0D 0x0A 两个字节，不是字符",
        "Appended at the byte level: in HEX mode this emits the bytes 0x0A / 0x0D 0x0A, not characters";
    clear_after_send: "发送后清空输入框", "Clear input after sending";
    repeat_interval_field: "定时发送间隔", "Repeat interval";
    repeat_interval_range: "ms（{}~{}）", "ms ({}~{})";
    repeat_note:
        "开关在发送区；断开连接会自动关掉，且不跨进程延续（重启后一定是关的）",
        "The switch lives in the send area; disconnecting turns it off automatically, and it never survives a restart";
    send_history_count_text: "{} / {} 条", "{} / {} entries";
    history_note:
        "↑ / ↓ 在输入框首行 / 末行时取历史（首行按 ↑、末行按 ↓）；连续发送相同内容只记一条",
        "↑ / ↓ recall history at the first/last line of the input (↑ on the first line, ↓ on the last); repeated identical sends are stored once";

    // —— panels/settings.rs：实时图表 ——
    chart_panel_checkbox:
        "显示图表面板（也可从「视图 → 面板」切换）",
        "Show the chart panel (also toggled from View → Panels)";
    max_samples_field: "样本上限", "Max samples";
    samples_range: "点/曲线（{}~{}）", "points/curve ({}~{})";
    samples_note:
        "超出上限时从最旧的样本开始淘汰；绘制时每条曲线最多取 4000 点",
        "When over the limit the oldest samples are dropped; each curve is drawn with at most 4000 points";
    auto_bounds_checkbox:
        "自动缩放（关掉后保留手动拖动/缩放的视图）",
        "Auto bounds (when off, your manual pan/zoom is kept)";
    current_data: "当前数据", "Current data";
    chart_count_text: "{} 通道 · {} 样本", "{} channels · {} samples";
    chart_format_note:
        "格式：一行一组样本，字段用逗号分隔（如 1.0,2.0,3.0）；与接收显示模式无关",
        "Format: one sample group per line, comma-separated fields (e.g. 1.0,2.0,3.0); independent of the receive display mode";

    // —— panels/settings.rs：文件日志 ——
    file_name_template: "文件名模板", "File name template";
    next_file_name:
        "下次开始记录时生成: {}",
        "Will be created when logging next starts: {}";
    output_dir_field: "输出目录", "Output directory";
    restore_default: "恢复默认", "Restore default";
    logging_written: "记录中，已写 {} 字节", "Logging, {} bytes written";
    log_note:
        "接收区的内容会同步写入该文件；日志文件不受上面的长度上限约束",
        "Whatever appears in the receive area is written to this file; the file is not subject to the length limit above";
    prefs_note:
        "以上均为用户偏好，改动即时生效并在退出时自动保存",
        "Everything above is a user preference: changes apply immediately and are saved on exit";

    // —— receive.rs：接收区右键菜单 ——
    copy_all_log: "复制全部日志", "Copy the whole log";
}

impl Language {
    /// 该语言对应的文案表。
    ///
    /// 返回 `&'static`：调用处拿到的引用不借用 `self`，因此可以在
    /// `&mut self` 的方法体里随时取用，不会与其它字段的可变借用打架。
    pub(crate) fn strings(self) -> &'static Strings {
        match self {
            Self::Zh => &ZH,
            Self::En => &EN,
        }
    }
}

/// `TimestampPrefix` 的界面标签。
///
/// 放在文案表这边而不是 `receive.rs` 里：这三个词是**界面词汇**，不是领域概念。
/// （旧版 `TimestampPrefix::label()` 硬编码中文，已随 F3 删除——留着它迟早有人
/// 在别处顺手用上，于是漏翻译。）
pub(crate) fn timestamp_prefix_label(strings: &Strings, prefix: TimestampPrefix) -> &'static str {
    match prefix {
        TimestampPrefix::Off => strings.ts_off,
        TimestampPrefix::PerLine => strings.ts_per_line,
        TimestampPrefix::PerBatch => strings.ts_per_batch,
    }
}

/// 把模板里的 `{}` 依次替换成实参。
///
/// 存在的唯一原因：`format!` 的格式串必须是字面量，而这里的格式串来自文案表
/// （运行期才决定是中文还是英文）。功能上等价于 `format!(模板, 参数…)`。
///
/// 实参多于 `{}` 时多余的被忽略；少于 `{}` 时**保留**未填充的 `{}`（不 panic）。
/// 两种情形在界面上都不该出现，`tests::every_template_placeholder_is_filled`
/// 用真实的调用参数数把这些模板钉住。
pub(crate) fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut next = 0;

    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(arg) = args.get(next) {
            use std::fmt::Write as _;
            // 写进 String 不会失败（`fmt::Write for String` 永远返回 Ok）
            let _ = write!(out, "{arg}");
        } else {
            out.push_str("{}");
        }
        next += 1;
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中日韩字符与中文标点。英文文案里出现任何一个都算漏翻译。
    fn has_cjk(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c,
                '\u{2E80}'..='\u{303F}'   // 部首扩展 + 中日韩符号与标点（。、「」等）
                | '\u{3400}'..='\u{4DBF}' // 扩展 A
                | '\u{4E00}'..='\u{9FFF}' // 基本区
                | '\u{F900}'..='\u{FAFF}' // 兼容表意文字
                | '\u{FF00}'..='\u{FFEF}' // 全角形式（（）等）
            )
        })
    }

    #[test]
    fn english_column_has_no_cjk() {
        for (name, _, en) in ALL {
            assert!(
                !has_cjk(en),
                "英文文案里仍有中文字符，说明漏翻译了：{name} = {en:?}"
            );
        }
    }

    /// 中英两列**相同**是允许的（"HEX" / "LF" 这类专有名词本来就一样），
    /// 但那种情况下中文列不该含中文——否则就是"复制粘贴忘了翻译"。
    #[test]
    fn identical_entries_are_language_neutral_only() {
        for (name, zh, en) in ALL {
            if zh == en {
                assert!(
                    !has_cjk(zh),
                    "中英两列一字不差且含中文，说明漏翻译了：{name} = {zh:?}"
                );
            }
        }
    }

    /// 反向兜底：防止有人把英文整列粘到中文列（或中文列被清空）。
    /// 不逐条断言"必须含中文"——`hex` / `ending_lf` 这类本来就该是 ASCII。
    #[test]
    fn chinese_column_is_overwhelmingly_chinese() {
        let total = ALL.len();
        let with_cjk = ALL.iter().filter(|(_, zh, _)| has_cjk(zh)).count();
        assert!(
            with_cjk * 10 >= total * 8,
            "中文列只有 {with_cjk}/{total} 条含中文，像是被整列替换过了"
        );
    }

    #[test]
    fn language_picks_the_matching_table() {
        let zh = Language::Zh.strings();
        let en = Language::En.strings();

        assert_eq!(zh.menu_file, "文件");
        assert_eq!(en.menu_file, "File");
        assert_eq!(zh as *const Strings, &ZH as *const Strings);
        assert_eq!(en as *const Strings, &EN as *const Strings);
        assert_ne!(
            zh as *const Strings, en as *const Strings,
            "两种语言必须指向不同的表"
        );
    }

    /// 中文列必须与 F3 之前硬编码的原文**逐字一致**（最显眼的那几条）。
    ///
    /// 全量比对做不到（原文散在 8 个文件里、已经删掉了），所以这里只钉住用户
    /// 一眼就会看出来的几条——它们要是变了，"切回中文界面没变化"这个验收标准
    /// 就已经破了。
    #[test]
    fn chinese_column_keeps_the_original_wording() {
        let s = Language::Zh.strings();

        assert_eq!(s.sidebar_toggle, "☰ 侧边栏");
        assert_eq!(s.receive_settings, "接收设置:");
        assert_eq!(s.send_area, "发送区:");
        assert_eq!(s.send_title, "发送");
        assert_eq!(s.open_port_button, "打开串口");
        assert_eq!(s.close_port_button, "关闭串口");
        assert_eq!(s.timestamp_prefix_line, "时间戳前缀: {}");
        assert_eq!(s.waiting_data, "等待数据…");
        assert_eq!(s.none_placeholder, "（无）");
        assert_eq!(s.empty_line, "<空行>");
        assert_eq!(s.ending_none, "无");
        assert_eq!(
            s.prefs_note,
            "以上均为用户偏好，改动即时生效并在退出时自动保存"
        );
        assert_eq!(
            s.encoding_gbk_note,
            "GBK 与 GB18030 的接收结果相同（共用解码器），仅发送时的可表示字符范围不同"
        );
    }

    /// `fill` 的基本行为，以及最常见的坑：实参不足 / 实参过多。
    #[test]
    fn fill_substitutes_placeholders_in_order() {
        assert_eq!(fill("a{}b{}c", &[&1, &2]), "a1b2c");
        assert_eq!(fill("无占位符", &[]), "无占位符");
        assert_eq!(fill("{}", &[&"甲"]), "甲");

        assert_eq!(
            fill("{} 与 {}", &[&1]),
            "1 与 {}",
            "实参不足时留下未填充的 {{}}，而不是 panic"
        );
        assert_eq!(fill("只有 {}", &[&1, &2]), "只有 1", "多余的实参被忽略");
    }

    /// 每条模板的 `{}` 个数必须与**实际调用处**传的实参数一致。
    ///
    /// 这里逐条钉住确实有占位符的模板：文案表里改坏了（比如把 `{}` 删掉、或
    /// 复制成两个）会在这一条测试里立刻暴露，不必等到界面上看见一个孤零零的 `{}`。
    #[test]
    fn template_placeholder_counts_are_pinned() {
        let placeholders = |template: &str| template.matches("{}").count();
        let s = Language::Zh.strings();
        let e = Language::En.strings();

        // (中文模板, 英文模板, 占位符个数)
        let pinned: &[(&str, &str, usize)] = &[
            (s.will_send_bytes, e.will_send_bytes, 1),
            (s.unmappable_in_encoding, e.unmappable_in_encoding, 1),
            (s.hex_parse_failed, e.hex_parse_failed, 1),
            (s.version_label, e.version_label, 1),
            (s.settings_dir, e.settings_dir, 1),
            (s.port_label, e.port_label, 1),
            (s.target_label, e.target_label, 1),
            (s.local_port_line, e.local_port_line, 1),
            (s.baud_label, e.baud_label, 1),
            (s.baud_custom_hint, e.baud_custom_hint, 2),
            (s.rx_bytes_label, e.rx_bytes_label, 1),
            (s.tx_bytes_label, e.tx_bytes_label, 1),
            (s.preset_full, e.preset_full, 1),
            (s.preset_count_text, e.preset_count_text, 2),
            (s.display_mode_line, e.display_mode_line, 1),
            (s.timestamp_prefix_line, e.timestamp_prefix_line, 1),
            (s.logged_bytes, e.logged_bytes, 1),
            (s.output_dir_line, e.output_dir_line, 1),
            (s.chart_summary, e.chart_summary, 3),
            (s.ignored_fields, e.ignored_fields, 1),
            (s.discarded_segments, e.discarded_segments, 1),
            (s.chart_format_hint, e.chart_format_hint, 1),
            (s.send_history_count, e.send_history_count, 2),
            (s.send_history_count_text, e.send_history_count_text, 2),
            (s.repeat_interval_range, e.repeat_interval_range, 2),
            (s.samples_range, e.samples_range, 2),
            (s.chart_count_text, e.chart_count_text, 2),
            (s.next_file_name, e.next_file_name, 1),
            (s.logging_written, e.logging_written, 1),
            (s.font_scale_preview, e.font_scale_preview, 1),
        ];

        for (zh, en, expected) in pinned {
            assert_eq!(
                placeholders(zh),
                *expected,
                "中文模板的 {{}} 个数不对：{zh:?}"
            );
            assert_eq!(
                placeholders(en),
                *expected,
                "英文模板的 {{}} 个数与中文不一致：{en:?}"
            );
        }

        // 反向：不带占位符的文案里不该冒出 `{}`（漏改的模板会让界面上直接显示 `{}`）
        let without_placeholders = [
            s.menu_file,
            s.settings_title,
            s.sec_receive,
            s.send_title,
            s.waiting_data,
            s.none_placeholder,
            s.copy_all_log,
        ];
        for template in without_placeholders {
            assert!(
                placeholders(template) == 0,
                "这条文案不该有占位符：{template:?}"
            );
        }
    }

    /// 把一行代码里的字符串字面量内容都抠出来（先去掉行注释，再逐字符扫，
    /// 反斜杠转义跳两个字符）。不需要正经词法分析：这几个文件里既没有块注释，
    /// 也没有 `r"..."` 原始串——`tests::scanned_ui_files_have_no_block_comments` 挡住后者。
    fn string_literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();

        for line in source.lines() {
            let code = match line.find("//") {
                Some(pos) => &line[..pos],
                None => line,
            };

            let chars: Vec<char> = code.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] != '"' {
                    i += 1;
                    continue;
                }
                i += 1;
                let mut literal = String::new();
                while i < chars.len() {
                    match chars[i] {
                        // 跳过转义序列（\" / \n / \u{4e2d} 都按两个字符处理）
                        '\\' => i += 2,
                        '"' => {
                            i += 1;
                            break;
                        }
                        c => {
                            literal.push(c);
                            i += 1;
                        }
                    }
                }
                found.push(literal);
            }
        }

        found
    }

    /// 纯 UI 源码里**不许**再出现硬编码中文。
    ///
    /// 这是 F3 之后唯一能长期兜住"新加的控件又写死中文"的机制：文案表本身保证不了
    /// 这一点，只有"绕开文案表就失败"能保证。文件清单**只收纯 UI 文件**——
    /// `app.rs` / `receive.rs` / `log_file.rs` 里按设计保留中文提示行（它们会写进
    /// 实时日志与导出文件，见模块文档），所以不能按同一条规则卡。
    #[test]
    fn ui_sources_do_not_hardcode_chinese() {
        // 「中文」是语言档位的**自名**：界面整体切成英文时那一档仍要写"中文"，
        // 否则只想切回中文的人认不出哪一档是中文。这是唯一允许出现在 UI 源码里的中文。
        const ALLOWED: &[&str] = &["中文"];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        for relative in PURE_UI_FILES {
            let path = root.join(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("读不到 {relative}（{}）: {e}", path.display()));

            // 只看 `#[cfg(test)]` 之前的正文：测试里的 `assert!` 消息是给开发者看的，
            // 不是界面文案，不必（也不该）跟着界面语言变。
            let body = match source.find("#[cfg(test)]") {
                Some(pos) => &source[..pos],
                None => source.as_str(),
            };

            for literal in string_literals(body) {
                if !has_cjk(&literal) || ALLOWED.contains(&literal.as_str()) {
                    continue;
                }
                panic!(
                    "{relative} 里有硬编码中文的字符串字面量：{literal:?}\n\
                     界面文案必须放进 `src/strings.rs` 的文案表（中英两列同一个宏臂），\
                     否则切到 English 时这一处会露出中文。"
                );
            }
        }
    }

    /// 纯 UI 文件清单。`ui_sources_do_not_hardcode_chinese` 只扫这几个。
    const PURE_UI_FILES: &[&str] = &[
        "src/sidebar.rs",
        "src/panels/menu_bar.rs",
        "src/panels/settings.rs",
        "src/panels/info.rs",
        "src/panels/chart.rs",
    ];

    /// `text` 里是否出现了 `needle`，且 `needle` 前面不是标识符字符。
    ///
    /// 用来把**原始字符串**（`r"…"` / `r#"…"#`）与"标识符末尾的 r 后面跟着一个普通
    /// 字符串"区分开——`egui::Panel::right("right_sidebar")` 里就有个 `r"`，但它不是
    /// 原始字符串。没有这层判断，下面的前提检查会天天误报。
    fn contains_token(text: &str, needle: &str) -> bool {
        text.match_indices(needle).any(|(index, _)| {
            !matches!(
                text[..index].chars().next_back(),
                Some(c) if c.is_alphanumeric() || c == '_'
            )
        })
    }

    /// 上面那个扫描器的前提条件：这些文件里不能有块注释与原始字符串，
    /// 否则 `string_literals` 会误判（把块注释里的引号当成字符串、或漏掉原始串
    /// 里的中文）。
    ///
    /// 这条测试把"扫描器可以偷懒"这个前提写成了可执行的断言——前提一旦破了，
    /// 这里会先失败，而不是让 `ui_sources_do_not_hardcode_chinese` 变成一条骗人的绿灯。
    #[test]
    fn scanned_ui_files_have_no_block_comments() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        for relative in PURE_UI_FILES {
            let source = std::fs::read_to_string(root.join(relative))
                .unwrap_or_else(|e| panic!("读不到 {relative}: {e}"));

            assert!(
                !source.contains("/*"),
                "{relative} 里出现了块注释，扫描器需要相应升级"
            );
            assert!(
                !contains_token(&source, "r\"") && !contains_token(&source, "r#\""),
                "{relative} 里出现了原始字符串，扫描器需要相应升级"
            );
        }
    }
}
