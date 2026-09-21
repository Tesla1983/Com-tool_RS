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
//! - 接收日志 `receive_text` / `display_spans` / `ascii_spans`：目前没有长度
//!   上限（见开发计划 P3），而 eframe 会周期性自动保存，落盘等于把一个无界
//!   字符串反复写进磁盘；
//! - 模态窗口开关 `show_settings`：不应该在启动瞬间自动弹窗。

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

    // 端口列表必须反映当前系统：插拔设备后重启，看到的不该是旧快照。
    app.ports = scan_ports();
    if app.ports.is_empty() {
        // 一个端口都没有时，留着上次的名字只会让界面显示一个不存在的端口。
        app.selected_port.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Language;
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
    }

    /// 运行时状态必须被丢弃，而不是从存档里带回来。
    #[test]
    fn runtime_state_is_never_restored() {
        let mut saved = customized();
        saved.connected = true;
        saved.show_settings = true;
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
        assert!(restored.receive_text.is_empty(), "接收日志不参与持久化");
        assert!(restored.display_spans.is_empty());
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
    }
}
