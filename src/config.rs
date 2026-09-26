//! 应用状态的持久化策略。
//!
//! 原则：**只存用户偏好，不存运行时状态**。
//!
//! 持久化的内容应当满足「下次启动直接可用」——它们是用户显式选过的参数，
//! 或者希望延续的输入。而下面这些一律不存，启动时重建：
//!
//! - 串口句柄与事件通道：句柄无法序列化，连接也必须由用户显式发起；
//! - 端口枚举结果 `ports`：沿用上次退出时的快照只会显示已经拔掉的设备，
//!   启动时必须按当前系统重新枚举；
//! - 接收日志 `receive_text` / `display_spans` / `ascii_text` / `ascii_spans`：虽然
//!   现在有 `max_log_chars` 上限，但把日志写进存档会让退出时的保存变慢，且没有
//!   明确需求（见开发计划 P3），因此仍不落盘；
//! - 模态窗口开关 `show_settings` / `show_info`：不应该在启动瞬间自动弹窗；
//! - 实时文件日志 `log`：它持有文件句柄与写盘线程，活不过进程边界；而且「启动
//!   即自动开始写文件」不是用户要的行为——上一次会话的记录必须由用户显式停止；
//! - 实时图表 `chart`：样本缓冲由数据速率决定大小，把上一次会话的曲线带回来没有
//!   意义（跨会话的曲线会接在序号 0 上，看起来像同一段时间的数据）；
//! - 定时发送开关 `repeat_send` 与历史浏览位置 `send_history_nav`：都是运行时状态。
//!   「启动就开始往外发数据」比「启动就开始写文件」更需要避免——后者只是多一个
//!   文件，前者会往设备上喷数据，而用户根本不知道是谁在发。`repeat_interval_ms`
//!   是用户偏好，照常落盘。
//!
//! 除了"存什么"，本模块还负责**启动时的不变量修复**：手改过的存档可能包含越界的
//! 重绘间隔、或者一个面板都不显示的界面配置，这些都在 [`restore_runtime_state`]
//! 里统一兜底。

use crate::app::ComTool;
use crate::serial_impl::scan_ports;

/// 应用状态在 `eframe` 存储中使用的键名。
pub const STORAGE_KEY: &str = eframe::APP_KEY;

/// 从存储中恢复上次会话。
///
/// 没有存档、存档损坏、或存档来自结构不兼容的旧版本时退化为默认值——
/// `get_value` 在反序列化失败时返回 `None`，而不是 panic。
pub fn load(storage: Option<&dyn eframe::Storage>) -> ComTool {
    let mut app: ComTool = storage
        .and_then(|s| eframe::get_value(s, STORAGE_KEY))
        .unwrap_or_default();
    restore_runtime_state(&mut app);
    app
}

/// 把当前状态写入存储。
pub fn save(storage: &mut dyn eframe::Storage, app: &ComTool) {
    eframe::set_value(storage, STORAGE_KEY, app);
}

/// 重建全部不参与持久化的运行时状态。
///
/// 单独拆成函数而不是内联进 `load`：直接依赖 [`eframe::CreationContext`] 的
/// 代码没法写单元测试，而这里可以——「恢复出来的状态一定是可用的」是本模块
/// 最需要被固定下来的不变量。
pub fn restore_runtime_state(app: &mut ComTool) {
    // 串口句柄与事件通道都活不过进程边界；启动时一律处于未连接状态。
    app.channel = None;
    app.event_rx = None;
    app.connected = false;
    app.last_reconnect_attempt = None;

    // 模态窗口不应在启动时自动出现。
    app.show_settings = false;
    app.show_info = false;

    // 实时文件日志：持有文件句柄与写盘线程，必须重建（也就是「不记录」）。
    // 直接赋值即可——`LogSink` 的 Drop 会投 `Halt` 并等待缓冲区落盘。
    app.log = None;

    // 实时图表的样本缓冲同样是纯运行时内容：把上一次会话的曲线带回来没有意义，
    // 而且它只受 `chart_max_samples` 约束，落盘只会平白撑大存档。
    app.chart.clear();

    // 「接收区视口是否贴底」是一次绘制过程的观察结果，跨进程没有意义；而且它决定
    // 这一帧要不要淘汰旧日志（见 `ComTool::receive_at_bottom`）。上一次会话若正好停在
    // 历史里，带回来的 `false` 会让启动后一直不敢裁剪 ⇒ 日志上限形同失效。一律置 true；
    // 配套的"每帧淘汰额度"同样来自绘制实测，一起复位成"不限制"。
    app.receive_at_bottom = true;
    app.receive_trim_budget = usize::MAX;

    // 端口列表必须反映当前系统：插拔设备后重启，看到的不该是旧快照。
    app.ports = scan_ports();
    if app.ports.is_empty() {
        // 一个端口都没有时，留着上次的名字只会让界面显示一个不存在的端口。
        app.selected_port.clear();
    }

    normalize_panel_visibility(app);

    // 「HEX 接收」「字节查看」「颜色」三者中不合法的那几对**互斥**（见
    // `ComTool::resolve_display_mode`）。旧存档里可能存着非法组合——曾经的接收设置行只
    // 互斥了 HEX 与文本，漏了「字节查看」，而「颜色」两个入口都没参与裁决（2026-09-24
    // 用户目视发现它导致接收区空白，见 `HANDOVER.md` §3.16）。三个参数都传 `false` 表示
    // "本轮没有点击"，于是按修复方向把冲突修掉：`hex` 关 `ascii`、`ascii` 关 `color`；
    // 修好之后给用户的是一份**非空**视图——不然被画的那份镜像自始至终没人写过。
    app.resolve_display_mode(false, false, false);

    // 定时发送的开关是纯运行时状态：**启动时一律关闭**。同一趟把计时基准也清掉，
    // 免得留下一个上次会话的时间戳让第一次发送立刻触发。
    app.stop_repeat_send();

    // 发送历史的浏览位置同样活不过进程边界：下次启动不该停在半截历史里。
    app.send_history_nav.reset();
    app.send_cursor_index = None;

    // 重绘间隔越界会让界面要么卡顿、要么 CPU 空转；越界值在这里夹回合法区间，
    // 使用处 `app.rs` 也会再夹一次，双保险。
    app.repaint_interval_ms = app.repaint_interval_ms.clamp(16, 1000);

    // 定时发送间隔：0 会让每帧都发（把 UI 与串口一起打满），超大值又等于关掉，
    // 越界值在这里夹回合法区间；`clamp_repeat_interval` 在使用处还会再夹一次。
    app.repeat_interval_ms = crate::send::clamp_repeat_interval(app.repeat_interval_ms);

    // 发送框的可见行数（拖把手调出来的界面尺寸）是用户偏好，但要夹回 2~40：
    // 手改成 0 会让输入框退化成一条缝，手改成 u32::MAX 则会把下方按钮挤出可视区。
    app.send_visible_rows = crate::send::clamp_send_visible_rows(app.send_visible_rows);
    // 拖动的"不足一行余量"是纯运行时状态：一次拖动跨不过进程边界。
    app.send_rows_drag_px = 0.0;

    // 图表样本上限同时决定内存与每帧绘制成本，越界值（手改存档）同样夹回去；
    // 解析入口 `ChartData::push_bytes` 还会再夹一次，双保险。
    app.chart_max_samples = app
        .chart_max_samples
        .clamp(crate::chart::MIN_SAMPLES, crate::chart::MAX_SAMPLES);
}

/// 保证「至少有一个面板可见」。
///
/// 视图菜单已经禁止把最后一个可见面板取消掉，但存档可以被手改（或者来自更早的
/// 版本），启动时兜底修正，避免出现"界面一片空白、用户以为程序坏了"。
fn normalize_panel_visibility(app: &mut ComTool) {
    if app.visible_panel_count() == 0 {
        app.show_receive_panel = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ConnectionKind, Language, ThemeChoice, UiFontScale};
    use crate::receive::{AnsiStyle, ColoredSpan, TextEncoding, TimestampPrefix};
    use crate::send::{SendFormat, SendHistory, SendLineEnding, SendPresets};
    // 调用 `get_string`/`set_string` 需要 trait 本身在作用域内
    use eframe::Storage as _;
    use serialport::{DataBits, FlowControl, Parity, StopBits};
    use std::collections::HashMap;

    /// 内存版 [`eframe::Storage`]。
    ///
    /// 用它而不是手写序列化：这样测试走的是与生产完全相同的路径——
    /// `save` → `eframe::set_value` → RON 编码 → 字符串 → `eframe::get_value`
    /// → RON 解码 → `load`。编码格式、字段缺失处理、解码失败退化都被覆盖到，
    /// 也不必为此新增测试依赖。
    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_string(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    /// 一份填满了非默认值的状态，用来验证持久化确实搬动了数据，
    /// 而不是恰好返回了默认值。
    fn customized() -> ComTool {
        ComTool {
            selected_port: "COM42".to_string(),
            // 波特率与「自定义档」标记配成一对：921600 虽然在列表里，但标记为 true，
            // 模拟"用户在下拉里选了自定义、然后手输了这个值"。正好也覆盖那条容易写错的
            // 约定——**选中哪一档与数字是多少是两件事**，不能靠"值是否在列表里"推导。
            baud_rate: 921_600,
            baud_custom: true,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            // F4 新增的连接类型与网络参数同样是"用户偏好"，必须能往返。
            // 这里刻意选**非默认**的组合（默认是 Serial / 127.0.0.1 / 1234 / 0），
            // 否则"字段根本没写进存档"这个缺陷会被默认值掩盖过去。
            connection_kind: ConnectionKind::Udp,
            net_remote_host: "192.168.1.50".to_string(),
            net_remote_port: 8888,
            net_local_port: 12345,
            send_text: "AT+RST\n".to_string(),
            hex_view: true,
            color_view: true,
            // 与 `hex_view`、`color_view` 都**互斥**（见 `ComTool::resolve_display_mode`），
            // 所以这份"填满非默认值"的样本里，`ascii_view` 只能为 `false`。它为真时的往返
            // 由 `ascii_view_alone_survives_round_trip` 单独覆盖。
            ascii_view: false,
            sidebar_open: false,
            language: Language::En,
            last_opened_port: Some("COM42".to_string()),
            theme: ThemeChoice::Dark,
            // 字体档位挑**非默认**的一档（默认是 `Normal`）：否则"字段根本没写进存档"
            // 会被默认值掩盖成"往返成功"。
            ui_font_scale: UiFontScale::ExtraLarge,
            auto_scroll: false,
            max_log_chars: 4_096,
            repaint_interval_ms: 40,
            clear_send_after_send: false,
            default_log_file_name: "MyLog_%Y.txt".to_string(),
            log_dir: "E:/serial-logs".to_string(),
            timestamp_prefix: TimestampPrefix::PerLine,
            send_format: SendFormat::Hex,
            send_line_ending: SendLineEnding::Crlf,
            text_encoding: TextEncoding::Gbk,
            send_history: {
                let mut h = SendHistory::default();
                h.record("AT+RST");
                h.record("01 02 03");
                h
            },
            repeat_interval_ms: 250,
            // 发送框拉到 8 行（默认 4 行）：这是用户拖把手调出来的界面尺寸，
            // 属于用户偏好，也必须能往返。
            send_visible_rows: 8,
            send_presets: {
                let mut p = SendPresets::default();
                p.add("AT");
                p.add("AT+GMR");
                p
            },
            show_receive_panel: true,
            show_send_panel: false,
            show_chart_panel: true,
            chart_max_samples: 500,
            chart_auto_bounds: false,
            ..ComTool::default()
        }
    }

    fn round_trip(app: &ComTool) -> ComTool {
        let mut storage = MemoryStorage::default();
        save(&mut storage, app);
        assert!(
            storage.get_string(STORAGE_KEY).is_some(),
            "save 应当把状态写入存储"
        );
        load(Some(&storage))
    }

    #[test]
    fn preferences_survive_round_trip() {
        let original = customized();
        let restored = round_trip(&original);

        assert_eq!(restored.selected_port, original.selected_port);
        assert_eq!(restored.baud_rate, original.baud_rate);
        assert_eq!(
            restored.baud_custom, original.baud_custom,
            "「哪一档被选中」必须跟着落盘，否则重启后用户选的自定义档会跳回列表档"
        );
        assert_eq!(restored.data_bits, original.data_bits);
        assert_eq!(restored.stop_bits, original.stop_bits);
        assert_eq!(restored.parity, original.parity);
        assert_eq!(restored.flow_control, original.flow_control);
        // F4 新增的连接类型与网络参数
        assert_eq!(restored.connection_kind, original.connection_kind);
        assert_eq!(restored.net_remote_host, original.net_remote_host);
        assert_eq!(restored.net_remote_port, original.net_remote_port);
        assert_eq!(restored.net_local_port, original.net_local_port);
        assert_eq!(restored.send_text, original.send_text);
        assert_eq!(restored.hex_view, original.hex_view);
        assert_eq!(restored.color_view, original.color_view);
        assert_eq!(restored.ascii_view, original.ascii_view);
        assert_eq!(restored.sidebar_open, original.sidebar_open);
        assert_eq!(restored.language, original.language);
        assert_eq!(restored.last_opened_port, original.last_opened_port);
        // P2/P3 新增的偏好字段同样要能往返
        assert_eq!(restored.theme, original.theme);
        assert_eq!(
            restored.ui_font_scale, original.ui_font_scale,
            "字体档位是用户偏好，必须跟着存档走——否则每次重启字体都会跳回标准档"
        );
        assert_eq!(restored.auto_scroll, original.auto_scroll);
        assert_eq!(restored.max_log_chars, original.max_log_chars);
        assert_eq!(restored.repaint_interval_ms, original.repaint_interval_ms);
        assert_eq!(
            restored.clear_send_after_send,
            original.clear_send_after_send
        );
        assert_eq!(
            restored.default_log_file_name,
            original.default_log_file_name
        );
        assert_eq!(restored.log_dir, original.log_dir);
        assert_eq!(restored.timestamp_prefix, original.timestamp_prefix);
        // 发送侧的偏好同样要能往返（历史是用户偏好，间隔也是）
        assert_eq!(restored.send_format, original.send_format);
        assert_eq!(restored.send_line_ending, original.send_line_ending);
        assert_eq!(restored.text_encoding, original.text_encoding);
        assert_eq!(restored.send_history, original.send_history);
        assert_eq!(restored.send_history.entries(), ["AT+RST", "01 02 03"]);
        // 常用数据（N2）是用户亲手挑的，必须跟着存档走
        assert_eq!(restored.send_presets, original.send_presets);
        assert_eq!(restored.send_presets.entries(), ["AT", "AT+GMR"]);
        assert_eq!(restored.repeat_interval_ms, original.repeat_interval_ms);
        assert_eq!(
            restored.send_visible_rows, original.send_visible_rows,
            "发送框的可见行数是用户拖出来的，必须跟着存档走"
        );
        assert_eq!(restored.show_receive_panel, original.show_receive_panel);
        assert_eq!(restored.show_send_panel, original.show_send_panel);
        // P5 新增的图表偏好同样要能往返
        assert_eq!(restored.show_chart_panel, original.show_chart_panel);
        assert_eq!(restored.chart_max_samples, original.chart_max_samples);
        assert_eq!(restored.chart_auto_bounds, original.chart_auto_bounds);
    }

    /// 运行时状态必须被丢弃，而不是从存档里带回来。
    #[test]
    fn runtime_state_is_never_restored() {
        let mut saved = customized();
        saved.connected = true;
        saved.show_settings = true;
        saved.show_info = true;
        saved.repeat_send = true;
        saved.receive_text = "上一次会话的日志".to_string();
        saved.display_spans.push(ColoredSpan {
            text: "x".to_string(),
            style: AnsiStyle::default(),
        });
        saved.chart.push_bytes(b"1,2,3\n", 100);
        assert!(!saved.chart.is_empty(), "先确认确实有图表数据要丢弃");
        // N1 / N4 的运行时状态：勾选态与计数都只描述"这一次连接"
        saved.rts = true;
        saved.dtr = true;
        saved.tx_bytes = 12_345;
        saved.was_focused = false;

        let restored = round_trip(&saved);

        assert!(
            !restored.rts && !restored.dtr,
            "控制线勾选态不得跨进程延续：下次打开端口时界面会声称线在那个电平上"
        );
        assert_eq!(restored.tx_bytes, 0, "发送计数只描述本次连接，不落盘");
        assert!(
            restored.was_focused,
            "焦点判重状态是纯运行时的，必须回到初值"
        );

        let restored = round_trip(&saved);

        assert!(!restored.connected, "连接状态不得跨进程延续");
        assert!(restored.channel.is_none(), "通道句柄必须重建");
        assert!(restored.event_rx.is_none(), "事件通道必须重建");
        assert!(!restored.show_settings, "设置窗口不该自动弹出");
        assert!(!restored.show_info, "信息窗口不该自动弹出");
        assert!(restored.log.is_none(), "实时文件日志必须重建");
        assert!(
            !restored.repeat_send,
            "定时发送不得跨进程延续：启动就开始往外发数据比自动写文件更严重"
        );
        assert!(restored.last_repeat_send.is_none(), "计时基准也要清掉");
        assert!(!restored.send_history_nav.is_active(), "历史浏览位置不落盘");
        assert!(restored.receive_text.is_empty(), "接收日志不参与持久化");
        assert!(restored.display_spans.is_empty());
        assert!(restored.ascii_text.is_empty());
        assert!(restored.ascii_spans.is_empty());
        assert!(restored.chart.is_empty(), "图表样本不参与持久化");
        assert_eq!(restored.chart.channel_count(), 0);
        assert_eq!(
            restored.ports,
            scan_ports(),
            "端口列表必须是当前系统的实时枚举结果，而不是存档快照"
        );
    }

    /// 存档缺少新增字段时要能读出来（字段只增不删的兼容承诺）。
    #[test]
    fn archive_missing_new_fields_still_loads() {
        let mut storage = MemoryStorage::default();
        // 手写一份「旧版本」存档：只有两个字段，其余全部缺席。
        storage.set_string(
            STORAGE_KEY,
            r#"(selected_port:"COM7",baud_rate:4800)"#.to_string(),
        );

        let restored = load(Some(&storage));

        assert_eq!(restored.selected_port, "COM7");
        assert_eq!(restored.baud_rate, 4800);
        // 缺席字段落到默认值，而不是解码失败
        assert_eq!(restored.data_bits, DataBits::Eight);
        assert_eq!(restored.parity, Parity::None);
        assert!(!restored.hex_view);
        assert!(restored.channel.is_none());
        assert!(
            restored.channel_kind.is_none(),
            "通道不存在时类型标记也必须清空"
        );
        assert!(
            !restored.baud_custom,
            "旧存档没有这个字段，必须落到「非自定义档」——否则一个 4800 会被\
             显示成「自定义」，而它明明就在下拉列表里"
        );
        assert_eq!(
            restored.ui_font_scale,
            UiFontScale::Normal,
            "旧存档没有字体档位字段，必须落到「标准」——也就是和加这个功能之前\
             一模一样，不能让老用户一升级就发现界面字号变了"
        );
    }

    /// P6 删掉了字段 `check_update_on_start`（版本检查被整体取消），而**旧存档里
    /// 还留着它**。反序列化默认忽略未知字段，这条测试把"数据兼容承诺"钉下来，
    /// 而不是靠"应该没问题"——一旦哪天有人加上 `deny_unknown_fields`，这里会先红。
    #[test]
    fn archive_with_a_removed_field_still_loads() {
        let mut storage = MemoryStorage::default();
        storage.set_string(
            STORAGE_KEY,
            r#"(selected_port:"COM7",baud_rate:4800,check_update_on_start:true)"#.to_string(),
        );

        let restored = load(Some(&storage));

        assert_eq!(restored.selected_port, "COM7");
        assert_eq!(restored.baud_rate, 4800);
    }

    /// 存档损坏时必须退化为默认值，绝不能 panic——否则用户只要有一个坏存档
    /// 就再也打不开程序。
    #[test]
    fn corrupt_archive_falls_back_to_default() {
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "这不是合法的 RON {{{".to_string());

        let restored = load(Some(&storage));

        assert!(!restored.connected);
        assert!(restored.channel.is_none());
        assert!(
            restored.channel_kind.is_none(),
            "通道不存在时类型标记也必须清空"
        );
        assert_eq!(restored.baud_rate, crate::serial_impl::DEFAULT_BAUD_RATE);
    }

    /// 完全没有存档（首次启动）同样要能正常工作。
    #[test]
    fn missing_archive_falls_back_to_default() {
        let restored = load(None);
        assert!(!restored.connected);
        assert_eq!(restored.baud_rate, crate::serial_impl::DEFAULT_BAUD_RATE);
        assert_eq!(restored.ports, scan_ports());
    }

    /// 默认状态本身必须是可用的。
    #[test]
    fn default_state_is_usable() {
        let mut app = ComTool::default();
        restore_runtime_state(&mut app);
        assert!(app.channel.is_none());
        assert!(!app.connected);
        assert!(app.visible_panel_count() > 0);
        assert_eq!(app.max_log_chars, crate::app::DEFAULT_MAX_LOG_CHARS);
        assert!(!app.is_logging(), "默认不处于记录状态");
        assert!(
            app.log_dir.is_empty(),
            "默认输出目录留空 = 用平台默认目录，而不是把本机路径烧进默认值"
        );
        assert!(!app.show_chart_panel, "图表默认不显示，避免占掉接收区高度");
        assert!(app.chart.is_empty(), "默认没有图表数据");
        assert_eq!(app.chart_max_samples, crate::app::DEFAULT_CHART_MAX_SAMPLES);
        assert_eq!(
            app.timestamp_prefix,
            TimestampPrefix::Off,
            "默认不加时间戳前缀：开启前的接收内容必须逐字节保持不变"
        );
        assert_eq!(app.send_format, SendFormat::Text, "默认按文本发送");
        assert_eq!(
            app.send_line_ending,
            SendLineEnding::None,
            "默认不追加行尾：升级前的发送行为必须逐字节保持不变"
        );
        assert_eq!(
            app.text_encoding,
            TextEncoding::Utf8,
            "默认 UTF-8：不改设置时收到的字节必须按原来的方式解释"
        );
        assert!(app.send_history.is_empty(), "默认没有发送历史");
        assert!(!app.repeat_send, "默认不定时发送");
        assert_eq!(
            app.repeat_interval_ms,
            crate::send::DEFAULT_REPEAT_INTERVAL_MS
        );
    }

    /// 定时发送间隔越界（手改存档）时夹回 10~600000。
    ///
    /// 0 尤其危险：它会让定时发送在**每一帧**都发一次，把 UI 线程与串口一起打满。
    #[test]
    fn repeat_interval_is_clamped_on_load() {
        let mut too_fast = customized();
        too_fast.repeat_interval_ms = 0;
        assert_eq!(
            round_trip(&too_fast).repeat_interval_ms,
            crate::send::MIN_REPEAT_INTERVAL_MS
        );

        let mut too_slow = customized();
        too_slow.repeat_interval_ms = u64::MAX;
        assert_eq!(
            round_trip(&too_slow).repeat_interval_ms,
            crate::send::MAX_REPEAT_INTERVAL_MS
        );
    }

    /// 正在进行的实时文件日志不得跨进程延续。
    ///
    /// 「启动就开始写文件」不是用户要的行为：上一次会话停在哪、新会话该不该记，
    /// 只能由用户显式决定。
    #[test]
    fn running_log_is_not_restored() {
        let dir = std::env::temp_dir().join(format!("comtool-config-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录应可创建");
        let log_path = dir.join("run.txt");
        let file = std::fs::File::create(&log_path).expect("应可创建日志文件");

        let mut saved = customized();
        saved.log = Some(crate::log_file::LogSink::start(log_path, file));
        assert!(saved.is_logging());

        let restored = round_trip(&saved);

        assert!(!restored.is_logging(), "启动后不得自动开始记录");
        assert!(restored.log_path().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 存档被手改成"一个面板都不显示"时，启动兜底修好，而不是给用户一片空白。
    #[test]
    fn all_panels_hidden_is_repaired_on_load() {
        let mut saved = customized();
        saved.sidebar_open = false;
        saved.show_receive_panel = false;
        saved.show_send_panel = false;
        saved.show_chart_panel = false;

        let restored = round_trip(&saved);

        assert!(
            restored.visible_panel_count() >= 1,
            "至少要有面板可见，否则界面一片空白"
        );
        assert!(restored.show_receive_panel);
    }

    /// 只隐藏了一部分面板是合法配置，不能被兜底逻辑改掉。
    #[test]
    fn partially_hidden_panels_are_left_alone() {
        let mut saved = customized();
        saved.show_receive_panel = true;
        saved.show_send_panel = false;
        saved.show_chart_panel = false;
        saved.sidebar_open = false;

        let restored = round_trip(&saved);

        assert!(restored.show_receive_panel);
        assert!(!restored.show_send_panel);
        assert!(!restored.show_chart_panel);
        assert!(!restored.sidebar_open);
    }

    /// **只显示图表**是合法配置：图表也计入 `visible_panel_count`，
    /// 所以它自己就能满足"至少保留一个可见"。
    #[test]
    fn chart_panel_alone_counts_as_visible() {
        let mut saved = customized();
        saved.sidebar_open = false;
        saved.show_receive_panel = false;
        saved.show_send_panel = false;
        saved.show_chart_panel = true;

        let restored = round_trip(&saved);

        assert_eq!(restored.visible_panel_count(), 1);
        assert!(restored.show_chart_panel);
        assert!(
            !restored.show_receive_panel,
            "已经有一个可见面板时不该被兜底逻辑强行打开接收区"
        );
    }

    /// 旧存档里「HEX 接收」与「字节查看」同时为真时，启动修复只留 HEX。
    ///
    /// 那个组合渲染出来是**空的**接收区：HEX 分支写 `display_spans`，字节视图读的却是
    /// `ascii_spans`。修复方向是"更强的视图胜出"（与下一条 `byte_view_wins_over_color_on_load`
    /// 同一方向），不是为了迎合 `display_mode()` 的多解顺序——那个顺序只在"互斥没兜住"时
    /// 才有意义，而它保证任何组合都不会空白。
    #[test]
    fn conflicting_display_modes_are_repaired_on_load() {
        let mut saved = customized();
        saved.hex_view = true;
        saved.ascii_view = true;

        let restored = round_trip(&saved);

        assert!(restored.hex_view, "强的视图胜出：HEX 留住");
        assert!(!restored.ascii_view, "字节视图留着必然是空的，必须被关掉");
    }

    /// 旧存档里「字节查看」与「颜色」同时为真时，启动修复只留字节查看。
    ///
    /// 与上一条同源：彩色视图渲染 `display_spans`，字节模式却只写 `ascii_spans`，两个都
    /// 开着时接收区**一片空白**（2026-09-24 用户目视发现）。留字节查看是因为"只开字节
    /// 查看"是合法配置，且颜色只是文本模式的修饰。
    #[test]
    fn byte_view_wins_over_color_on_load() {
        let mut saved = customized();
        saved.hex_view = false;
        saved.ascii_view = true;
        saved.color_view = true;

        let restored = round_trip(&saved);

        assert!(restored.ascii_view);
        assert!(!restored.color_view, "彩色视图留着必然是空的，必须被关掉");
    }

    /// 只开「字节查看」是合法配置，不能被上面那条修复顺手关掉；
    /// 顺带覆盖 `ascii_view`（非默认值）的往返——`customized()` 里它必须是 `false`，
    /// 因为与 `hex_view` 互斥。
    ///
    /// `color_view` 在这里显式关掉，否则样本就变成"字节查看 + 颜色"——那是**非法**组合，
    /// 会被启动修复动过，测的就不是"只开字节查看"这件事了。
    #[test]
    fn ascii_view_alone_survives_round_trip() {
        let mut saved = customized();
        saved.hex_view = false;
        saved.ascii_view = true;
        saved.color_view = false;

        let restored = round_trip(&saved);

        assert!(!restored.hex_view);
        assert!(restored.ascii_view);
    }

    /// 重绘间隔越界（手改存档）时夹回 16~1000。
    #[test]
    fn repaint_interval_is_clamped_on_load() {
        let mut too_fast = customized();
        too_fast.repaint_interval_ms = 1;
        assert_eq!(round_trip(&too_fast).repaint_interval_ms, 16);

        let mut too_slow = customized();
        too_slow.repaint_interval_ms = 100_000;
        assert_eq!(round_trip(&too_slow).repaint_interval_ms, 1_000);
    }

    /// 发送框的可见行数是用户偏好，必须跟着存档走；越界值（手改存档）在启动时夹回
    /// 2~40——判据写字面量，不引用 `MIN/MAX_SEND_VISIBLE_ROWS`（引用实现常量等于
    /// 让被测代码自己判卷）。
    #[test]
    fn send_visible_rows_survive_round_trip_and_are_clamped() {
        let mut taller = customized();
        taller.send_visible_rows = 12;
        assert_eq!(round_trip(&taller).send_visible_rows, 12);

        // 0 会让输入框退化成一条缝，1 也一样（下限是 2）
        let mut too_small = customized();
        too_small.send_visible_rows = 0;
        assert_eq!(round_trip(&too_small).send_visible_rows, 2);

        let mut too_tall = customized();
        too_tall.send_visible_rows = u32::MAX;
        assert_eq!(round_trip(&too_tall).send_visible_rows, 40);
    }

    /// 拖动余量是运行时状态，不落盘（把上一次拖动差的那几个像素带回来只会让
    /// 下一次拖动的头几像素没反应）。
    #[test]
    fn send_rows_drag_carry_is_not_restored() {
        let mut saved = customized();
        saved.send_rows_drag_px = 9.5;
        assert_eq!(round_trip(&saved).send_rows_drag_px, 0.0);
    }

    /// 图表样本上限越界（手改存档）时同样夹回合法区间。
    ///
    /// 这一项既决定内存也决定每帧绘制成本，0 还会让"每来一个样本就被淘汰"看起来
    /// 像功能坏了，所以下限不是 0。
    #[test]
    fn chart_sample_limit_is_clamped_on_load() {
        let mut too_small = customized();
        too_small.chart_max_samples = 0;
        assert_eq!(
            round_trip(&too_small).chart_max_samples,
            crate::chart::MIN_SAMPLES
        );

        let mut too_large = customized();
        too_large.chart_max_samples = usize::MAX;
        assert_eq!(
            round_trip(&too_large).chart_max_samples,
            crate::chart::MAX_SAMPLES
        );
    }
}
