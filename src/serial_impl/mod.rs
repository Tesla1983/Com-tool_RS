//! 串口传输层。
//!
//! 本模块为独立实现：接口划分、并发模型与内部数据结构均按本项目需要重新
//! 设计，不派生自任何第三方串口 GUI 项目。
//!
//! - [`config`]：端口参数与波特率档位。
//! - [`channel`]：后台线程化的串口通道。

pub mod channel;
pub mod config;

pub use channel::{SerialChannel, scan_ports};
pub use config::{
    BAUD_RATES, CUSTOM_BAUD_MAX, CUSTOM_BAUD_MIN, DEFAULT_BAUD_RATE, SerialConfig, clamp_baud,
};
