//! 字节流通信通道的抽象层。
//!
//! 这里只描述「一条由后台线程持有、UI 线程仅通过消息与之交互的双向字节流」
//! 这件事本身，不绑定任何具体传输介质。串口是当前唯一的实现，但 TCP、
//! 命名管道、用于测试的内存通道都可以落在同一组契约上。
//!
//! 契约：
//!
//! 1. 配置必须在 `open` 之前经 `configure` 提交，实现方自行校验参数。
//! 2. 所有阻塞 IO 都发生在后台线程；UI 线程只投递消息，不做 IO。
//! 3. 通道终止的三条路径——对端关闭、发生错误、`close` 被调用——都必须让
//!    `is_active` 回到 `false`，不允许出现「线程已退出但状态仍报活动」。
//! 4. `close` 幂等；对已停止的通道重复调用不得报错，也不得 panic。

use std::io;
use std::sync::mpsc::Sender;

/// 通道在生命周期内向 UI 侧上报的事件。
#[derive(Debug)]
pub enum ChannelEvent {
    /// 一段原始字节到达。分片边界由实现方决定，不保证与设备侧的发送边界一致，
    /// 因此上层必须按流处理，不能假设一个事件等于一条完整报文。
    Bytes(Vec<u8>),

    /// 连接正常终止（对端主动关闭）。
    ///
    /// 串口通道不会产生该事件——USB 拔出、设备断电在驱动层表现为读错误，
    /// 统一走 `Failed`。该分支保留给对端能够主动握手下线的传输（如 TCP）。
    #[allow(dead_code)]
    Closed,

    /// 通道发生错误。字符串用于向用户展示，不保证机器可解析。
    Failed(String),
}

/// 后台线程持有的字节流通道。
///
/// 要求 `Send`：对象本身由 UI 侧持有，真正的 IO 句柄始终留在后台线程里。
pub trait ByteChannel: Send {
    /// 启动后台线程。此后事件经 `sink` 上报，直至通道终止。
    ///
    /// 如果通道已经处于活动状态，返回 [`io::ErrorKind::AlreadyExists`]。
    fn open(&mut self, sink: Sender<ChannelEvent>) -> io::Result<()>;

    /// 停止后台线程并回收其资源。对已经停止的通道调用是幂等的。
    fn close(&mut self) -> io::Result<()>;

    /// 通道当前是否活动。
    ///
    /// 该状态仅用于向 UI 汇报，不参与控制流——停机由 [`close`](Self::close)
    /// 显式驱动，不依赖这个标志位。
    fn is_active(&self) -> bool;

    /// 把一段字节交给后台线程发出。调用方不必等待写入完成。
    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()>;

    /// 在 `open` 之前提交配置。
    ///
    /// 默认实现拒绝一切配置，要求实现方显式覆盖——这样「忘记实现配置入口」
    /// 会在打开时立刻暴露，而不是静默地按默认参数运行。
    fn configure(&mut self, settings: &dyn std::any::Any) -> io::Result<()> {
        let _ = settings;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "该通道类型不接受运行时配置",
        ))
    }
}
