//! 网络通道（TCP 客户端 / UDP）。
//!
//! 与 [`crate::serial_impl`] 平级：`serial_impl` 是串口这一种传输的实现，
//! 这里是网络侧的两种。两边的接口完全相同——都实现
//! [`ByteChannel`](crate::communicationtrait::ByteChannel)，因此 `ComTool`
//! 那一侧只认 trait 对象，换传输类型不需要改接收链路。
//!
//! # 文件划分
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `config.rs` | [`NetConfig`]：目标地址 / 端口 / UDP 本地端口 |
//! | `tcp.rs` | [`TcpChannel`]：**同步 connect**、对端关闭走 `ChannelEvent::Closed` |
//! | `udp.rs` | [`UdpChannel`]：只绑定不连接、零长度数据报不是 EOF |
//!
//! 路标（`FUTURE_ROADMAP.md` §5）列的是 `{mod, tcp, udp}` 三个文件，这里多了一个
//! `config.rs`：串口那边就是 `serial_impl/{mod, channel, config}` 的划分，
//! 保持一致比严格照抄文件清单更值钱。
//!
//! # 零新增依赖
//!
//! 只用 `std::net`。这三个实现的难点不在协议，而在"把连接生命周期讲清楚"——
//! 见 `tcp.rs` / `udp.rs` 的模块文档。
//!
//! # 第一版**不做**
//!
//! * **TCP Server / 多客户端**：一个 `ByteChannel` 管一个对端；"一个监听端口管多个
//!   客户端"是另一种形态（要决定"当前对端是谁"、要有客户端列表），属独立课题。
//! * **命名管道**：同上的单通道契约问题，且 Windows 上要多一套 ACL 处理。
//! * **TCP 自动重连**：`auto_reconnect` 依赖 `last_opened_port`，是串口语义；
//!   给 TCP 加上会在对端未启动时产生半死不活的 socket 状态，收益不明确。
//! * **TLS / SSH**：SSH 是 `FUTURE_ROADMAP.md` §7 的 D1（待定开发），
//!   它会复用本模块建立的"连接类型分叉"框架。

mod config;
mod tcp;
mod udp;

pub use config::{DEFAULT_REMOTE_HOST, DEFAULT_REMOTE_PORT, NetConfig};
pub use tcp::TcpChannel;
pub use udp::UdpChannel;
