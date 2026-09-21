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
    /// 统一走 `Failed`。该事件由**对端能够主动握手/优雅下线**的传输发出
    /// （TCP 的 `read` 返回 0 就是它）。UDP 无连接，同样不会产生。
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

    /// 通道实际绑定到的本地地址；这类概念对它不成立、或实现方拿不到时返回 `None`。
    ///
    /// 目前只有 UDP 会实现：UDP 的本地端口允许配 0（由系统分配），而用户**必须**
    /// 知道最终分到了哪个端口，否则不知道该让设备往哪回包。串口没有"本地地址"，
    /// TCP 的本地端口由内核在 `connect` 时挑、对调试也少有用处——默认实现返回
    /// `None`，免得每个实现都要编一个假值出来。
    fn local_addr(&self) -> Option<std::net::SocketAddr> {
        None
    }

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

/// 后台线程实现方共用的脚手架。
///
/// 三个实现（串口 / TCP / UDP）的后台线程结构是同一个：独占一个 IO 句柄，
/// 每轮「读一次 → 推进一次写 → 排空指令队列」，用 [`WorkerCommand`] 停机、
/// 用 [`StateGuard`] 保证状态归位。这些**不是"看起来像"就该复制的样板**——
/// 「线程退出后状态必须归位」「`Halt` 在任何时刻都要能被看到」是三条实现共有的
/// 硬约定（见本模块文档第 3、4 条），复制三份迟早有一份走偏。
pub(crate) mod worker {
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    /// UI 侧投给后台线程的指令。
    pub(crate) enum WorkerCommand {
        /// 把这段字节发给对端。
        Transmit(Vec<u8>),
        /// 结束工作循环并返回。
        Halt,
    }

    /// `is_active` 的取值。用 `u8` 而不是 `bool`，是为了给将来可能出现的中间态
    /// （例如「正在打开」）留位置，同时保持无锁读取。
    pub(crate) const IDLE: u8 = 0;
    pub(crate) const ACTIVE: u8 = 1;

    /// 后台线程退出时统一把状态归位。
    ///
    /// 用 Drop 守卫而不是在每个 `return` 前手写一次 `store`：新增退出分支时
    /// 不会漏掉置位，`is_active` 也就不可能停留在「线程已死但仍报活动」。
    pub(crate) struct StateGuard(pub(crate) Arc<AtomicU8>);

    impl Drop for StateGuard {
        fn drop(&mut self) {
            self.0.store(IDLE, Ordering::Release);
        }
    }

    /// 驱动在「暂时无数据 / 暂时无法收发」时返回的错误种类。这些不是故障。
    ///
    /// 只列**所有传输共有**的三种。某个传输特有的可忽略错误（例如 Windows 上
    /// connected UDP socket 把 ICMP port unreachable 报成 `ConnectionReset`）
    /// 由该实现自己补充，不要把特例塞进来——否则串口那边也会跟着忽略一个真故障。
    pub(crate) fn is_transient(err: &io::Error) -> bool {
        matches!(
            err.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
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
        ///
        /// `Failed` / `Closed` 会顺带把替身置回**非活动**：真实的实现里后台线程退出
        /// 时 `StateGuard` 必然把 `is_active` 归位（见本模块文档第 3 条），替身不跟着
        /// 做就会在"掉线后重新 `open`"这条路径上给出假的 `AlreadyExists` 错误。
        pub(crate) fn emit(&self, event: ChannelEvent) {
            let mut shared = self.shared.lock().expect("锁未被毒化");
            if matches!(event, ChannelEvent::Failed(_) | ChannelEvent::Closed) {
                shared.active = false;
            }
            if let Some(sink) = shared.sink.clone() {
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
        /// 拒绝配置会让 `open_connection` 走失败分支，测不到后面的链路。
        fn configure(&mut self, _settings: &dyn std::any::Any) -> io::Result<()> {
            Ok(())
        }
    }
}
