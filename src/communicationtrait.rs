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

    /// 通道侧的一条**非致命**消息：只该显示给用户，不改变连接状态。
    ///
    /// 与 `Failed` 的区别是决定性的——`Failed` 的含义是"这条通道已经不能用了"，
    /// 上层据此置 `connected = false`、停掉定时发送、让自动重连接手。而"设置 RTS
    /// 失败"这类**只影响单次操作**的错误若走 `Failed`，用户会看到界面从"已连接"
    /// 跳成"未连接"、定时发送被静默关掉，而实际上数据照样在收发。
    Notice(String),
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

    /// 通道是否**确实已经就绪**——可以收发字节了。
    ///
    /// 与 [`is_active`](Self::is_active) 的差别只在**串口**：串口的 `open()` 只
    /// 负责拉起后台线程，端口要到后台线程里才真正打开，因此 `open()` 返回之后的
    /// 一小段时间里通道是"活动但未就绪"（`OPENING`）。UI 侧据此显示"正在打开"，
    /// 而不是谎报"已连接"——那正是 `FEATURE_GAP_AND_PLAN.md` §4 记的假阳性缺陷。
    ///
    /// **默认实现直接转发 `is_active()`**：TCP / UDP 的 `open()` 在调用线程里就
    /// 把 connect / bind 做完，返回 `Ok` 即已就绪，两者恒等。只有串口会覆盖它。
    /// 这样放默认实现而不是让每个实现各写一遍，是因为"就绪"与"活动"对网络通道
    /// 而言**必须**是同一条判据——各写一份迟早有一份漂开。
    fn is_ready(&self) -> bool {
        self.is_active()
    }

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

    /// 把手动电平请求交给后台线程。`None` 表示"这一根线不动"。
    ///
    /// **默认实现直接拒绝**：只有串口有 RTS / DTR 这两根控制线，网络通道没有。
    /// 与 [`configure`](Self::configure) 同一个套路——"这个实现不提供该能力"必须
    /// 当场说出来，而不是静默忽略；静默忽略会让用户以为线已经拉下去了。
    ///
    /// 用 `Option<bool>` 而不是两个 `bool`：调用方通常只想动其中一根。用两个 `bool`
    /// 表达"不动"，就只能把另一根**按调用方以为的值**重写一遍，而我们其实并不知道
    /// 它现在是什么电平——`serialport` 只提供写、不提供读。
    fn set_line_state(&mut self, rts: Option<bool>, dtr: Option<bool>) -> io::Result<()> {
        let _ = (rts, dtr);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "该通道类型没有 RTS / DTR 控制线",
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
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    /// UI 侧投给后台线程的指令。
    pub(crate) enum WorkerCommand {
        /// 把这段字节发给对端。
        Transmit(Vec<u8>),
        /// 改变控制线的输出电平（`None` = 这一根不动）。只有串口会收到它。
        ///
        /// 它必须走指令队列而不是在 `SerialChannel` 上直接办：端口句柄归后台线程独占，
        /// UI 线程够不着（本模块文档第 2 条）。
        SetLineState {
            rts: Option<bool>,
            dtr: Option<bool>,
        },
        /// 结束工作循环并返回。
        Halt,
    }

    /// `is_active` 的取值。用 `u8` 而不是 `bool`，是为了给**中间态**留位置，
    /// 同时保持无锁读取。这一档现在是 [`OPENING`]，不再是"将来可能"。
    pub(crate) const IDLE: u8 = 0;
    pub(crate) const ACTIVE: u8 = 1;

    /// 「后天线程已拉起，但通道**还没真正就绪**」。
    ///
    /// 只有串口会遇到这一档：`TcpChannel` / `UdpChannel` 的 `open()` 在**调用线程
    /// 里**就把 connect / bind 做完才返回，所以对它们而言 `Ok` 就等于"已经就绪"。
    /// 串口的 `open()` 只是拉起线程，真正的端口打开发生在后台线程上、结果可能失败。
    ///
    /// **为什么不复用 `IDLE` 表示"还没就绪"**：那样 UI 在 `open()` 返回后会看到
    /// "未活动"，于是无法区分"正在打开"与"打开失败/已关闭"——既有的 `connected`
    /// 假阳性缺陷正是这么来的（UI 只能乐观置位）。有了这一档，"等待中"可以明确
    /// 显示出来，也不会被误判成掉线。
    ///
    /// **配置里存的 `u8` 必须是三态之外的值**：这是 `AtomicU8` 的取值集合，不是
    /// 持久化字段。见 `serial_impl::channel` 的 `open()` 注释——顺序
    /// （`UI 置 OPENING` → `spawn` → `worker 置 ACTIVE`）本身就是判据。
    pub(crate) const OPENING: u8 = 2;

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

    /// 待发队列的字节上限。
    ///
    /// **为什么必须有上限**：待发队列曾在两个实现（串口 / TCP）里都是无界的
    /// `VecDeque`。设备侧长期不接受数据（驱动发送缓冲满、硬件流控掐住、对端不读）
    /// 时，用户每点一次「发送」就往后追加一段，队列只增不减，最终把内存吃光。
    /// 「定时发送」按 10 ms 间隔灌数据会把这个过程加速到分钟级。
    ///
    /// **为什么是 1 MiB**：串口工具的现实负载是"几十字节的指令 + 偶发几百 KB 的
    /// 固件块"。1 MiB 足够缓冲一条很长的报文，同时把单次故障的内存占用钉在
    /// 可解释的量级；超出的部分**拒绝入队并明确报错**，而不是继续吃内存。
    ///
    /// 这个值是**共享**的：串口与 TCP 用同一个上限，判定逻辑也只写一处
    /// （[`push_bounded`]）——两个实现各写一份，迟早有一份被改歪。
    pub(crate) const MAX_PENDING_BYTES: usize = 1024 * 1024;

    /// 把一段待发数据追加到队尾，**带字节上限**。
    ///
    /// 返回 `Ok(())` 表示已入队；返回 `Err(pending)` 表示**已拒绝**，实参是拒绝
    /// 时的在队字节数（仅供调用方拼错误信息）。
    ///
    /// 判定只看"在队字节数的合计"，不看条数：一条 4 MiB 的报文与四千条 1 KiB 的
    /// 指令对内存的压力相同，按条数限流会放过前者、误伤后者。
    ///
    /// **满了就拒绝，绝不静默丢弃也不无限增长**。这与 HEX / 转义解析失败时
    /// "不发送但给提示"是同一条原则：数据没出去必须让用户知道，界面上的
    /// "发送字节计数"也才不会记账。
    pub(crate) fn push_bounded(
        outgoing: &mut VecDeque<(Vec<u8>, usize)>,
        bytes: Vec<u8>,
    ) -> Result<(), usize> {
        let pending: usize = outgoing.iter().map(|(b, _)| b.len()).sum();
        if pending + bytes.len() > MAX_PENDING_BYTES {
            return Err(pending);
        }
        outgoing.push_back((bytes, 0));
        Ok(())
    }

    /// 处理一条 [`WorkerCommand::Transmit`]：带背压入队，满了报一条 `Notice`。
    ///
    /// 抽成函数而不是在两个/三个 `worker_main` 里各写一段：入队策略（上限、
    /// 拒绝时的措辞、走 `Notice` 而非 `Failed`）必须**只有一处实现**，
    /// 三个实现各写一份迟早有一份走偏，而且偏的那一份不会有人发现。
    ///
    /// 返回 `Err(pending)` 便于调用方与测试断言"确实是被拒绝了"。
    pub(crate) fn accept_transmit(
        outgoing: &mut VecDeque<(Vec<u8>, usize)>,
        bytes: Vec<u8>,
        events: &std::sync::mpsc::Sender<crate::communicationtrait::ChannelEvent>,
    ) -> Result<(), usize> {
        match push_bounded(outgoing, bytes) {
            Ok(()) => Ok(()),
            Err(pending) => {
                // 走 `Notice` 而非 `Failed`：队列满只说明"这一下没排上队"，
                // 通道本身好好的，判成掉线会让界面从"已连接"跳走、还会停掉定时发送。
                let _ = events.send(crate::communicationtrait::ChannelEvent::Notice(format!(
                    "待发队列已满（在队 {pending} 字节，上限 {MAX_PENDING_BYTES} 字节），本条未发送"
                )));
                Err(pending)
            }
        }
    }
}

#[cfg(test)]
mod worker_tests {
    use super::worker::{MAX_PENDING_BYTES, accept_transmit, push_bounded};
    use crate::communicationtrait::ChannelEvent;
    use std::collections::VecDeque;
    use std::sync::mpsc;

    /// 空队列应接受任意合法长度的数据。
    #[test]
    fn push_bounded_accepts_within_capacity() {
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        push_bounded(&mut q, vec![0u8; 1024]).expect("远小于上限应接受");
        assert_eq!(q.len(), 1, "入队后队列应有一条");
    }

    /// **核心判据**：累计字节数越界时必须拒绝，并且**不改动队列**。
    ///
    /// 断言的是"拒绝后队列长度不变"——只看返回值会漏掉"先 push 再报错"这种
    /// 半吊子实现（数据进去了、调用方却以为没进去）。
    #[test]
    fn push_bounded_rejects_over_capacity_without_mutating() {
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        // 先塞满到接近上限。
        push_bounded(&mut q, vec![0u8; MAX_PENDING_BYTES - 10]).expect("刚好不越界应接受");
        let before = q.len();

        let err = push_bounded(&mut q, vec![0u8; 20]).expect_err("越界必须被拒绝");
        assert!(err > 0, "错误信息里应带上在队字节数，实际为 {err}");
        assert_eq!(q.len(), before, "拒绝时不得改动队列");
    }

    /// 恰好等于上限应被接受（边界不能差一）。
    #[test]
    fn push_bounded_boundary_is_inclusive() {
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        push_bounded(&mut q, vec![0u8; MAX_PENDING_BYTES])
            .expect("恰好等于上限必须接受，否则容量比标称值少一个字节");
        assert_eq!(q.len(), 1);
    }

    /// 多条小数据累加越界同样要被拒——判定必须看**合计字节**而不是条数。
    #[test]
    fn push_bounded_counts_total_bytes_not_entries() {
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        // 每条 1 KiB，塞到接近上限。
        let chunk = vec![0u8; 1024];
        for _ in 0..(MAX_PENDING_BYTES / 1024) {
            push_bounded(&mut q, chunk.clone()).expect("累加到上限之前都应接受");
        }
        // 再加一条必然越界。
        assert!(
            push_bounded(&mut q, chunk).is_err(),
            "合计字节越界必须被拒绝，与条数无关"
        );
    }

    /// **核心行为判据**：溢出时事件通道里必须收到一条 `Notice`。
    ///
    /// 这是"串口 / TCP / UDP 三个后台线程到底有没有真的报错"的判定点——
    /// 只看 `push_bounded` 的返回值无法证明调用方处理了它。断言落在**事件内容**
    /// 与**非致命**这两件事上：消息要说明是队列满，而且**不能**是 `Failed`
    /// （`Failed` 会让界面从"已连接"跳走、还会停掉定时发送）。
    #[test]
    fn accept_transmit_reports_notice_on_overflow() {
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        let (tx, rx) = mpsc::channel();

        // 先塞满到上限。
        accept_transmit(&mut q, vec![0u8; MAX_PENDING_BYTES], &tx).expect("恰好等于上限应接受");
        assert!(rx.try_recv().is_err(), "未溢出时不应产生任何事件");

        // 再投一条，必然溢出。
        let refused = accept_transmit(&mut q, vec![1u8; 8], &tx);
        assert!(refused.is_err(), "越界必须返回 Err 供调用方判断");

        match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(ChannelEvent::Notice(msg)) => {
                assert!(
                    msg.contains("待发队列已满"),
                    "提示应说清是队列满，实际为: {msg}"
                );
            }
            Ok(ChannelEvent::Failed(msg)) => {
                panic!("队列满不是掉线，绝不能走 Failed（会让界面跳成未连接），实际为: {msg}");
            }
            Ok(other) => panic!("溢出应上报 Notice，实际收到: {other:?}"),
            Err(e) => panic!("溢出必须上报事件，但没收到: {e}"),
        }
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
        /// `open()` 时是否**当场就绪**。
        ///
        /// 替身默认同步就绪（`open()` 返回即算通），与 TCP / UDP 一致。置为真时
        /// 它会扮串口：`open()` 后 `is_active()` 仍为假，直到测试显式
        /// [`FakeHandle::signal_ready`]——这正是"串口 `open()` 只拉起线程、端口
        /// 在后台线程上才打开"的模型。
        ///
        /// 没有这一档就没法在门禁里验证 `ComTool::open_connection` 对串口的判定：
        /// 替身一律同步就绪的话，串口与 TCP 的行为不可区分，"UI 不得乐观置位"
        /// 这条约定永远测不到。
        async_ready: bool,
        reject_writes: bool,
        reject_line_state: bool,
        /// 每一次 `set_line_state` 的实参，按发生顺序。
        line_state_calls: Vec<(Option<bool>, Option<bool>)>,
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

        /// 让替身扮**串口**：`open()` 后不会当场就绪。
        ///
        /// 必须在 `open()` **之前**调用（`open()` 会读它决定初始的 `active`）。
        pub(crate) fn defer_ready(&self) {
            self.shared.lock().expect("锁未被毒化").async_ready = true;
        }

        /// 模拟"后台线程把端口打开了"：把状态推到已就绪。
        ///
        /// 与 [`FakeHandle::defer_ready`] 配对使用——调用者要在 `open_connection()`
        /// 之后、收到 `Failed` 之前调用它，才能表达"这次真的开成了"。
        pub(crate) fn signal_ready(&self) {
            self.shared.lock().expect("锁未被毒化").active = true;
        }

        /// 让后续 `write` 一律失败，用来验证"发送失败"分支。
        pub(crate) fn reject_writes(&self, reject: bool) {
            self.shared.lock().expect("锁未被毒化").reject_writes = reject;
        }

        /// 让后续 `set_line_state` 一律失败，用来验证"线状态设置失败"走的是
        /// **非致命**提示（`Notice`）而不是掉线（`Failed`）。
        pub(crate) fn reject_line_state(&self, reject: bool) {
            self.shared.lock().expect("锁未被毒化").reject_line_state = reject;
        }

        /// 到目前为止每一次线状态请求的实参。
        pub(crate) fn line_state_calls(&self) -> Vec<(Option<bool>, Option<bool>)> {
            self.shared
                .lock()
                .expect("锁未被毒化")
                .line_state_calls
                .clone()
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
            // 同步模式（默认，扮 TCP/UDP）：`open` 返回即就绪。
            // 异步模式（扮串口）：状态留给测试用 `signal_ready` 推。
            shared.active = !shared.async_ready;
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

        fn is_ready(&self) -> bool {
            // 替身不区分"活动"与"就绪"：它的 `active` 本来就由测试显式摆布
            // （`defer_ready` / `signal_ready`），这就是它扮串口三态的方式。
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

        /// 记下每一次请求。**不接受** trait 的默认实现（那个一律报 `Unsupported`），
        /// 否则"界面上勾一下 RTS 到底有没有传到通道"就没法在门禁里验。
        fn set_line_state(&mut self, rts: Option<bool>, dtr: Option<bool>) -> io::Result<()> {
            let mut shared = self.shared.lock().expect("锁未被毒化");
            if shared.reject_line_state {
                return Err(io::Error::other("替身通道被要求拒绝线状态操作"));
            }
            shared.line_state_calls.push((rts, dtr));
            Ok(())
        }
    }
}
