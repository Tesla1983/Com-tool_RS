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
//!   即自动开始写文件」不是用户要的行为——上一次会话的记录必须由用户显式停止。
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
    app.serial = None;
    app.event_rx = None;
    app.connected = false;
    app.last_reconnect_attempt = None;

    // 模态窗口不应在启动时自动出现。
    app.show_settings = false;
    app.show_info = false;

    // 实时文件日志：持有文件句柄与写盘线程，必须重建（也就是「不记录」）。
    // 直接赋值即可——`LogSink` 的 Drop 会投 `Halt` 并等待缓冲区落盘。
    app.log = None;

    // 端口列表必须反映当前系统：插拔设备后重启，看到的不该是旧快照。
    app.ports = scan_ports();
    if app.ports.is_empty() {
        // 一个端口都没有时，留着上次的名字只会让界面显示一个不存在的端口。
        app.selected_port.clear();
    }

    normalize_panel_visibility(app);

    // 重绘间隔越界会让界面要么卡顿、要么 CPU 空转；越界值在这里夹回合法区间，
    // 使用处 `app.rs` 也会再夹一次，双保险。
    app.repaint_interval_ms = app.repaint_interval_ms.clamp(16, 1000);
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
    use crate::app::{Language, ThemeChoice};
    use crate::receive::{AnsiStyle, ColoredSpan};
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
            baud_rate: 921_600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            send_text: "AT+RST\n".to_string(),
            hex_view: true,
            color_view: true,
            ascii_view: true,
            sidebar_open: false,
            language: Language::En,
            last_opened_port: Some("COM42".to_string()),
            theme: ThemeChoice::Dark,
            auto_scroll: false,
            max_log_chars: 4_096,
            repaint_interval_ms: 40,
            clear_send_after_send: false,
            default_log_file_name: "MyLog_%Y.txt".to_string(),
            log_dir: "E:/serial-logs".to_string(),
            check_update_on_start: true,
            show_receive_panel: true,
            show_send_panel: false,
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
        assert_eq!(restored.data_bits, original.data_bits);
        assert_eq!(restored.stop_bits, original.stop_bits);
        assert_eq!(restored.parity, original.parity);
        assert_eq!(restored.flow_control, original.flow_control);
        assert_eq!(restored.send_text, original.send_text);
        assert_eq!(restored.hex_view, original.hex_view);
        assert_eq!(restored.color_view, original.color_view);
        assert_eq!(restored.ascii_view, original.ascii_view);
        assert_eq!(restored.sidebar_open, original.sidebar_open);
        assert_eq!(restored.language, original.language);
        assert_eq!(restored.last_opened_port, original.last_opened_port);
        // P2/P3 新增的偏好字段同样要能往返
        assert_eq!(restored.theme, original.theme);
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
        assert_eq!(
            restored.check_update_on_start,
            original.check_update_on_start
        );
        assert_eq!(restored.show_receive_panel, original.show_receive_panel);
        assert_eq!(restored.show_send_panel, original.show_send_panel);
    }

    /// 运行时状态必须被丢弃，而不是从存档里带回来。
    #[test]
    fn runtime_state_is_never_restored() {
        let mut saved = customized();
        saved.connected = true;
        saved.show_settings = true;
        saved.show_info = true;
        saved.receive_text = "上一次会话的日志".to_string();
        saved.display_spans.push(ColoredSpan {
            text: "x".to_string(),
            style: AnsiStyle::default(),
        });

        let restored = round_trip(&saved);

        assert!(!restored.connected, "连接状态不得跨进程延续");
        assert!(restored.serial.is_none(), "串口句柄必须重建");
        assert!(restored.event_rx.is_none(), "事件通道必须重建");
        assert!(!restored.show_settings, "设置窗口不该自动弹出");
        assert!(!restored.show_info, "信息窗口不该自动弹出");
        assert!(restored.log.is_none(), "实时文件日志必须重建");
        assert!(restored.receive_text.is_empty(), "接收日志不参与持久化");
        assert!(restored.display_spans.is_empty());
        assert!(restored.ascii_text.is_empty());
        assert!(restored.ascii_spans.is_empty());
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
        assert!(restored.serial.is_none());
    }

    /// 存档损坏时必须退化为默认值，绝不能 panic——否则用户只要有一个坏存档
    /// 就再也打不开程序。
    #[test]
    fn corrupt_archive_falls_back_to_default() {
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "这不是合法的 RON {{{".to_string());

        let restored = load(Some(&storage));

        assert!(!restored.connected);
        assert!(restored.serial.is_none());
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
        assert!(app.serial.is_none());
        assert!(!app.connected);
        assert!(app.visible_panel_count() > 0);
        assert_eq!(app.max_log_chars, crate::app::DEFAULT_MAX_LOG_CHARS);
        assert!(!app.is_logging(), "默认不处于记录状态");
        assert!(
            app.log_dir.is_empty(),
            "默认输出目录留空 = 用平台默认目录，而不是把本机路径烧进默认值"
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
        saved.sidebar_open = false;

        let restored = round_trip(&saved);

        assert!(restored.show_receive_panel);
        assert!(!restored.show_send_panel);
        assert!(!restored.sidebar_open);
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
}
