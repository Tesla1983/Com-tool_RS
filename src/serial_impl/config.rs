//! 串口的可选参数与一次打开所需的完整配置。
//!
//! 这里刻意把「UI 下拉框要展示的档位」与「打开端口时要提交给驱动的参数」
//! 分成两件事：档位是一串普通的 `u32`，UI 直接格式化展示；只有真正打开
//! 端口时才组装成 [`SerialConfig`]。这样 UI 层不需要为了显示而携带
//! 「显示字符串 + 数值」这样的双字段包装。

use serialport::{DataBits, FlowControl, Parity, StopBits};

/// UI 波特率下拉框的档位，数组顺序即展示顺序。
///
/// 覆盖 110 到 3 Mbaud：低位段是传统设备的遗留速率，高位段是常见 USB
/// 转串口桥（CP210x / CH34x / FTDI）与 CDC-ACM 的上限区间。
pub const BAUD_RATES: [u32; 20] = [
    110, 300, 600, 1_200, 2_400, 4_800, 9_600, 14_400, 19_200, 38_400, 57_600, 115_200, 128_000,
    230_400, 256_000, 460_800, 921_600, 1_000_000, 2_000_000, 3_000_000,
];

/// 默认波特率。绝大多数开发板、Bootloader 与 USB 转串口芯片的出厂设置。
pub const DEFAULT_BAUD_RATE: u32 = 115_200;

/// 打开一个串口所需的全部参数。
#[derive(Debug, Clone, PartialEq)]
pub struct SerialConfig {
    /// 系统端口名，Windows 形如 `COM3`，Unix 形如 `/dev/ttyUSB0`。
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,
}

impl Default for SerialConfig {
    /// 8N1 无流控 @115200——覆盖面最广的一档，也是多数串口工具的初始值。
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: DEFAULT_BAUD_RATE,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
        }
    }
}
