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

/// 测试用的通道替身。
///
/// 存在的理由：**发送链路与连接生命周期可以在 `cargo test` 里被完整验证**，
/// 不需要串口硬件、不需要打开窗口。它记下每一次 `write` 的字节，并允许测试
/// 主动往事件通道里灌 `Failed` / `Closed`——否则"通道报错时要自动停掉定时发送"
/// 这类约定只能靠人工拔线去验。
#[cfg(test)]
pub(crate) mod fake {
    use super::{ByteChannel, ChannelEvent};
    use std::io;
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Shared {
        written: Vec<Vec<u8>>,
        active: bool,
        reject_writes: bool,
        sink: Option<Sender<ChannelEvent>>,
    }

    /// 交给被测代码的通道本体。
    pub(crate) struct FakeChannel {
        shared: Arc<Mutex<Shared>>,
    }

    /// 留在测试手里的观察窗。
    #[derive(Clone)]
    pub(crate) struct FakeHandle {
        shared: Arc<Mutex<Shared>>,
    }

    impl FakeChannel {
        /// 返回「交给被测代码的通道」与「测试自己留着的观察窗」。
        pub(crate) fn new() -> (Self, FakeHandle) {
            let shared = Arc::new(Mutex::new(Shared::default()));
            (
                Self {
                    shared: Arc::clone(&shared),
                },
                FakeHandle { shared },
            )
        }
    }

    impl FakeHandle {
        /// 到目前为止每一次写入的字节，按发生顺序。
        pub(crate) fn written(&self) -> Vec<Vec<u8>> {
            self.shared.lock().expect("锁未被毒化").written.clone()
        }

        /// 所有写入拼起来的一份字节流——断言"到底发出去了什么"时比逐次比对好用。
        pub(crate) fn written_bytes(&self) -> Vec<u8> {
            self.written().concat()
        }

        pub(crate) fn write_count(&self) -> usize {
            self.shared.lock().expect("锁未被毒化").written.len()
        }

        /// 让后续 `write` 一律失败，用来验证"发送失败"分支。
        pub(crate) fn reject_writes(&self, reject: bool) {
            self.shared.lock().expect("锁未被毒化").reject_writes = reject;
        }

        /// 往被测代码的事件通道里灌一个事件（通道未打开时什么也不做）。
        pub(crate) fn emit(&self, event: ChannelEvent) {
            let sink = self.shared.lock().expect("锁未被毒化").sink.clone();
            if let Some(sink) = sink {
                let _ = sink.send(event);
            }
        }
    }

    impl ByteChannel for FakeChannel {
        fn open(&mut self, sink: Sender<ChannelEvent>) -> io::Result<()> {
            let mut shared = self.shared.lock().expect("锁未被毒化");
            if shared.active {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "通道已在运行"));
            }
            shared.active = true;
            shared.sink = Some(sink);
            Ok(())
        }

        fn close(&mut self) -> io::Result<()> {
            let mut shared = self.shared.lock().expect("锁未被毒化");
            shared.active = false;
            shared.sink = None;
            Ok(())
        }

        fn is_active(&self) -> bool {
            self.shared.lock().expect("锁未被毒化").active
        }

        fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
            let mut shared = self.shared.lock().expect("锁未被毒化");
            if shared.reject_writes {
                return Err(io::Error::other("替身通道被要求拒绝写入"));
            }
            shared.written.push(bytes);
            Ok(())
        }

        /// 替身接受任何配置：被测代码传进来的是串口参数，而替身不关心内容——
        /// 拒绝配置会让 `open_port` 走失败分支，测不到后面的链路。
        fn configure(&mut self, _settings: &dyn std::any::Any) -> io::Result<()> {
            Ok(())
        }
    }
}
