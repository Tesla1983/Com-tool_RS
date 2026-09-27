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

    /// 待发队列里**还没写出去**的字节数（诊断读数）。
    ///
    /// 回答的是"我点了发送、对端却没收到"这类问题：[`write`](Self::write) 返回
    /// `Err` 时那是"当场被拒"，用户看得到提示；而**队列没满、只是一直写不动**
    /// （对端不读、流控拉住）原先完全没有可见痕迹。界面把这个数显示出来，
    /// "长时间不下降"就成了那个痕迹。
    ///
    /// **不参与任何控制流**，只用于显示——它慢一拍、偶尔读到中间值都不影响正确性，
    /// 所以这里是纯读、不加锁也不做同步。
    ///
    /// **默认实现返回 0**：三个真实通道都覆盖它（各自持有
    /// [`worker::PendingLedger`]）。给默认实现而非强制实现，是因为它对"没有队列的
    /// 替身/未来通道"是**正确**的值（确实是 0），而强制实现只是让每处多写四行
    /// 返回同一个 0。防线放在别处：三个真实实现各有一条测试钉住读数会随预约上涨
    /// （见 `communicationtrait.rs` / `net_impl` 的 `pending_bytes_*` 用例）——
    /// **忘了覆盖**会被那些用例抓住，而不是靠这里编译器报错。
    fn pending_bytes(&self) -> usize {
        0
    }

    /// 把一段字节交给后台线程发出。**不等待写入完成，但当场判定能不能排队**。
    ///
    /// 返回 `Ok` 的含义是"**已经排上队**，后台线程会负责把它写出去"，不是"已经写到
    /// 对端了"——真正的写入仍在后台线程上做（本模块文档第 2 条）。
    ///
    /// 与之相对，返回 `Err` 的两类情况都是**当场就有定论**的：
    ///
    /// * 通道不可用（未打开 / 后台线程已退出）；
    /// * **待发队列已满**（[`worker::MAX_PENDING_BYTES`]）——这一条必须是同步的，
    ///   否则调用方会把"只是排上了队"当成"发出去了"，于是记账虚高、还会把用户
    ///   没发出去的内容清掉。判定落在调用线程里，靠的是 [`worker::PendingLedger`]。
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
/// 用 [`StateGuard`] 保证状态归位、用 [`PendingLedger`] 记账待发容量。这些
/// **不是"看起来像"就该复制的样板**——「线程退出后状态必须归位」「`Halt` 在任何
/// 时刻都要能被看到」「待发数据一件都还没发出去时不能报成功」是三条实现共有的
/// 硬约定（见本模块文档第 3、4 条），复制三份迟早有一份走偏。
pub(crate) mod worker {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

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
    /// 可解释的量级；超出的部分**拒绝并明确报错**，而不是继续吃内存。
    ///
    /// 判定只写一处（[`PendingLedger::reserve`]），三个实现共用同一个值——
    /// 各写一份迟早有一份被改歪，而且偏的那一份不会有测试失败来提醒。
    pub(crate) const MAX_PENDING_BYTES: usize = 1024 * 1024;

    /// 待发队列的**容量账本**：调用线程预约额度，后台线程写完后归还。
    ///
    /// # 为什么需要它（而不是"在队列上判上限"）
    ///
    /// 队列本体归后台线程独占（本模块文档第 2 条：UI 线程不做 IO），所以调用线程
    /// **看不见**它。之前把上限判在后台线程的队列上（`push_bounded`），信息回路是
    /// 单向的：UI 投出去就返回 `Ok`，"排不上队"这件事**回不到**调用线程。用户能
    /// 看见的两个后果都是这么来的：
    ///
    /// * 状态栏的「发送字节数」把根本没发出去的数据也算上；
    /// * 「发送后清空」把用户刚敲下、其实没发出去的内容清掉。
    ///
    /// 把它做成一个**两侧都能读的原子计数**，`write()` 就能在调用线程里当场知道
    /// 排不上队，于是 `write()` 返回 `Err`、调用方既不记账也不清空——两个症状一起
    /// 消失，而且不需要调用方额外判断什么（它本来就是"`Ok` 才记账"）。
    ///
    /// # 它数的是哪一段
    ///
    /// 数的是"**用户已经交出去、但一个字节都还没写出去**"的合计：包括还在 mpsc 通道
    /// 里没被取走的、以及已经在后台队列里等着的。这比"只数队列里的"更贴近用户感知
    /// ——压着没发出去的就是压着。
    ///
    /// # 用完了要还
    ///
    /// 额度在两个地方归还：后台线程把**整段**写完时（[`PendingLedger::release`]），
    /// 以及 `open` 时的 [`PendingLedger::reset`]。后者是必需的：通道对象在类型不变时
    /// 会被**就地复用**（见 `app::ComTool::open_connection`），上次运行结束时可能还
    /// 挂着没写完的额度，不清掉会让重开后的容量凭空少一块。
    #[derive(Clone, Default)]
    pub(crate) struct PendingLedger(Arc<AtomicUsize>);

    impl PendingLedger {
        /// 预约 `len` 字节的额度；满了返回 `Err(在队字节数)`（供调用方拼错误信息）。
        ///
        /// 用 CAS 循环而不是"读一次、判一次、写一次"：判定与占用必须是**一个原子
        /// 步骤**，否则两个调用线程可以同时通过判定、把合计推过上限。眼下只有 UI
        /// 一个生产者，但那是调用方的巧合，不该由这里来假设。
        ///
        /// 边界是**闭**的：恰好等于 [`MAX_PENDING_BYTES`] 接受，多一个字节拒绝。
        /// 容量比标称值少一个字节是那种"说明和实现对不上"的小毛病，专门有一条测试钉住。
        pub(crate) fn reserve(&self, len: usize) -> Result<(), usize> {
            let mut cur = self.0.load(Ordering::Acquire);
            loop {
                if cur + len > MAX_PENDING_BYTES {
                    return Err(cur);
                }
                match self.0.compare_exchange_weak(
                    cur,
                    cur + len,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Ok(()),
                    // 别的线程抢先改了，拿新值重判——不能直接放弃，否则会误拒。
                    Err(actual) => cur = actual,
                }
            }
        }

        /// 归还 `len` 字节额度。后台线程**确实把整段写完之后**才调用。
        ///
        /// 不做"写到一半就按比例还"：额度记账的粒度就是"条"，一条写完了才算出去。
        /// 按字节比例还会引入一个更难解释的口径（用户看到的"在队字节数"到底指什么）。
        pub(crate) fn release(&self, len: usize) {
            self.0.fetch_sub(len, Ordering::AcqRel);
        }

        /// 归零。每次 `open` 都要做——理由见本类型的文档「用完了要还」。
        pub(crate) fn reset(&self) {
            self.0.store(0, Ordering::Release);
        }

        /// 当前在队字节数。
        ///
        /// 这是**诊断读数**，经 [`ByteChannel::pending_bytes`] 上报给界面。
        /// 原先这里写着"界面暂未展示它"、还挂着 `#[cfg(test)]`——那正是
        /// `HANDOVER.md` §5「写路径瞬态错误静默重试」那条缺陷的另一半：
        /// **队列已满**有提示，可**队列没满而长期写不动**（设备不读、流控拉住）
        /// 原先没有任何可见痕迹。2026-09-27 起状态栏显示这个数，
        /// "数字停住不下降"就是那个痕迹。
        pub(crate) fn bytes(&self) -> usize {
            self.0.load(Ordering::Acquire)
        }
    }

    /// 队首那一段写完了就出队，并把额度还给调用线程。
    ///
    /// 三个 `worker_main` 共用它。这两件事**必须一起做**：只出队不归还 ⇒ 容量只减
    /// 不增（用户发上一会儿就永远报"队列满"，而通道其实好得很）；只归还不出队 ⇒
    /// 同一段数据被反复重发。合成一处就不会有一份实现只做了其中一半——而那种漏法
    /// 恰恰不会有测试失败来提醒（真通道上的写入要硬件才能驱动）。
    pub(crate) fn retire_finished_chunk(
        outgoing: &mut VecDeque<(Vec<u8>, usize)>,
        finished: bool,
        ledger: &PendingLedger,
    ) {
        if !finished {
            return;
        }
        if let Some((done, _)) = outgoing.pop_front() {
            ledger.release(done.len());
        }
    }

    /// 把一段字节交给后台线程：**先预约额度，再投递；投递失败要退还**。
    ///
    /// 三个实现共用它。这三步里漏掉任何一步都会产生一个静默偏差——漏预约等于上限
    /// 失效，漏退还等于重开后的容量凭空变小——而且都不会有测试失败来提醒。各写一份
    /// 迟早有一份漏，所以只留这一处。
    ///
    /// `noun` 是传输名（"串口" / "TCP" / "UDP"），只用于拼"后台线程已退出"这句
    /// 错误信息，让用户在接收区里一眼看出是哪条通道出的问题。
    pub(crate) fn submit(
        ledger: &PendingLedger,
        commands: &std::sync::mpsc::Sender<WorkerCommand>,
        bytes: Vec<u8>,
        noun: &str,
    ) -> io::Result<()> {
        let len = bytes.len();
        if let Err(pending) = ledger.reserve(len) {
            // 文案与后台线程那条 `Notice` 保持一致（同一个判据，不该长出两种说法）。
            // 不同的是：现在它是**同步**返回的错误，用户当场就能看到，而不是过一会儿
            // 从接收区里冒出来。
            return Err(io::Error::other(format!(
                "待发队列已满（在队 {pending} 字节，上限 {MAX_PENDING_BYTES} 字节），本条未发送"
            )));
        }
        if commands.send(WorkerCommand::Transmit(bytes)).is_err() {
            // 后台线程已经退出，这段数据没有去处——把额度还回去。不还的话，通道在
            // 类型不变时会被就地复用，下一次 `open` 之前这段"幽灵额度"会让容量变少。
            ledger.release(len);
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("{noun}后台线程已退出"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod worker_tests {
    use super::worker::{
        MAX_PENDING_BYTES, PendingLedger, WorkerCommand, retire_finished_chunk, submit,
    };
    use std::collections::VecDeque;
    use std::sync::mpsc;

    /// 额度从空一直给到上限，**恰好等于上限要接受**（边界是闭的）。
    ///
    /// 断言写成"MAX 接受、再要一个字节就拒绝"两段：容量比标称值少一个字节是那种
    /// "说明和实现对不上"的小毛病，只测一侧两边都测不出来。
    #[test]
    fn ledger_hands_out_quota_up_to_the_cap() {
        let ledger = PendingLedger::default();
        ledger
            .reserve(MAX_PENDING_BYTES)
            .expect("恰好等于上限必须接受，否则容量比标称值少一个字节");
        assert_eq!(
            ledger.bytes(),
            MAX_PENDING_BYTES,
            "预约之后账本要如实记下在队量"
        );

        assert_eq!(
            ledger.reserve(1).expect_err("已经满额时必须拒绝"),
            MAX_PENDING_BYTES,
            "拒绝时要把当前在队字节数带出来，供调用方拼错误信息"
        );
        assert_eq!(ledger.bytes(), MAX_PENDING_BYTES, "被拒的预约不得改动账本");
    }

    /// 多条小负载累加同样会顶到上限——判定看的是**合计字节**而不是条数。
    ///
    /// 一条 4 MiB 的报文与四千条 1 KiB 的指令对内存的压力相同；按条数限流会放过
    /// 前者、误伤后者。
    #[test]
    fn ledger_counts_total_bytes_not_entries() {
        let ledger = PendingLedger::default();
        for _ in 0..(MAX_PENDING_BYTES / 1024) {
            ledger.reserve(1024).expect("累加到上限之前都应接受");
        }
        assert!(
            ledger.reserve(1024).is_err(),
            "合计字节越界必须被拒绝，与条数无关"
        );
    }

    /// 归还之后额度要能重新用——写出去的数据不该永久占着容量。
    #[test]
    fn ledger_release_returns_the_quota() {
        let ledger = PendingLedger::default();
        ledger.reserve(MAX_PENDING_BYTES).expect("应接受");
        assert!(ledger.reserve(1).is_err(), "用满后应拒绝");

        ledger.release(64);
        assert_eq!(ledger.bytes(), MAX_PENDING_BYTES - 64, "归还多少就少多少");
        ledger.reserve(64).expect("归还回来的额度应能立刻再用");
        assert!(ledger.reserve(1).is_err(), "再要一个字节又该拒绝了");
    }

    /// ⭐ **归零**：通道对象在类型不变时会被**就地复用**（见
    /// `app::ComTool::open_connection`），所以每次 `open` 都得清一次账本。
    /// 不清的话，上一次运行留下的"幽灵额度"会让重开后的容量凭空少一块——用户
    /// 会看到"明明没发多少就报队列满"。
    #[test]
    fn ledger_reset_clears_the_quota() {
        let ledger = PendingLedger::default();
        ledger.reserve(MAX_PENDING_BYTES).expect("应接受");
        ledger.reset();
        assert_eq!(ledger.bytes(), 0, "归零后账本必须为空");
        ledger
            .reserve(MAX_PENDING_BYTES)
            .expect("归零后应能重新用满一整份容量");
    }

    /// ⭐ **出队与归还必须同时发生**。
    ///
    /// 只出队不归还 ⇒ 容量只减不增（用户发上一会儿就永远报队列满，而通道其实好好
    /// 的）；只归还不出队 ⇒ 同一段数据被反复重发。两条都是那种在真通道上才暴露、
    /// 而真通道要硬件才能驱动的漏法，所以在这里一次钉住。
    #[test]
    fn retiring_a_finished_chunk_pops_it_and_returns_the_quota() {
        let ledger = PendingLedger::default();
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        ledger.reserve(300).expect("额度充足");
        q.push_back((vec![0u8; 100], 100));
        q.push_back((vec![0u8; 200], 200));

        // 还没写完：队列与账本都不许动。
        retire_finished_chunk(&mut q, false, &ledger);
        assert_eq!(q.len(), 2, "没写完就不该出队");
        assert_eq!(ledger.bytes(), 300, "没写完就不该归还额度");

        // 写完了：出队 + 归还。
        retire_finished_chunk(&mut q, true, &ledger);
        assert_eq!(q.len(), 1, "写完的那一段必须出队，否则会一直重发同一段");
        assert_eq!(ledger.bytes(), 200, "写完的那一段要把额度还回去");
    }

    /// 空队列上"写完了"也不得 panic，并且账本不许被减成负数（`usize` 会绕成一个
    /// 巨大值，那会让后续的每一次 `reserve` 都被误拒）。
    #[test]
    fn retiring_on_an_empty_queue_is_a_no_op() {
        let ledger = PendingLedger::default();
        let mut q: VecDeque<(Vec<u8>, usize)> = VecDeque::new();
        retire_finished_chunk(&mut q, true, &ledger);
        assert_eq!(ledger.bytes(), 0);
        ledger.reserve(1).expect("账本不该被减坏");
    }

    /// ⭐ **核心判据**：满了必须**在调用线程里当场**返回错误，而且**一个字节都没交出去**。
    ///
    /// 断言"接收端一条命令都没收到"是关键——只看返回值的版本漏得掉"先投递再报错"
    /// 这种半吊子实现（数据进去了、调用方却以为没进去）。那正是本次要修的缺陷的
    /// 原始形态：上限挂在后台线程上，调用方无从知道。
    #[test]
    fn submit_refuses_when_the_ledger_is_full_and_queues_nothing() {
        let ledger = PendingLedger::default();
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        ledger.reserve(MAX_PENDING_BYTES).expect("先把额度用光");

        let err = submit(&ledger, &tx, vec![1u8; 8], "TCP").expect_err("满了必须当场报错");
        assert!(
            err.to_string().contains("待发队列已满"),
            "错误信息应说清是队列满，实际为: {err}"
        );
        assert!(
            err.to_string().contains(&MAX_PENDING_BYTES.to_string()),
            "错误信息应带上上限，用户才判断得出要等多久，实际为: {err}"
        );
        assert!(
            rx.try_recv().is_err(),
            "被拒绝的数据绝不能已经投给后台线程——那等于上限根本没生效"
        );
    }

    /// 正常路径：预约成功后数据确实到了后台线程手上，账本也如实记着。
    #[test]
    fn submit_hands_the_bytes_to_the_worker_and_charges_the_ledger() {
        let ledger = PendingLedger::default();
        let (tx, rx) = mpsc::channel::<WorkerCommand>();

        submit(&ledger, &tx, vec![7u8; 512], "串口").expect("额度充足时应接受");
        assert_eq!(ledger.bytes(), 512, "接受的字节要记进账本");

        match rx.try_recv().expect("应收到一条 Transmit") {
            WorkerCommand::Transmit(bytes) => assert_eq!(bytes, vec![7u8; 512]),
            _ => panic!("投给后台线程的应当是 Transmit"),
        }
    }

    /// 后台线程已经退出时投递会失败——此时**必须把刚预约的额度还回去**。
    ///
    /// 不还的后果很难查：通道被就地复用，重开之后容量少了一块，而用户看到的只是
    /// "没发多少就报队列满"。
    #[test]
    fn submit_returns_the_quota_when_the_worker_is_gone() {
        let ledger = PendingLedger::default();
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        drop(rx); // 后台线程退出 ⇒ 指令通道断开

        let err = submit(&ledger, &tx, vec![0u8; 4096], "UDP").expect_err("投不出去必须报错");
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
        assert!(
            err.to_string().contains("UDP"),
            "错误信息应指出是哪条通道退出了，实际为: {err}"
        );
        assert_eq!(
            ledger.bytes(),
            0,
            "投递失败必须退还额度，否则重开后的容量会变少"
        );
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
        /// "队列里还压着 N 字节没写出去"的读数，由测试用
        /// [`FakeHandle::set_pending`] 摆布——真实通道这个数来自 `PendingLedger`，
        /// 替身没有后台线程、也就没有会自己变化的队列，只能这样表达。
        pending: usize,
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

        /// 摆出"队列里还压着 `n` 字节"的状态，用来验证状态栏那条待发读数。
        pub(crate) fn set_pending(&self, n: usize) {
            self.shared.lock().expect("锁未被毒化").pending = n;
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

        /// 替身不排队（`write` 直接落到 `written` 里），所以这个数只能由测试摆布。
        fn pending_bytes(&self) -> usize {
            self.shared.lock().expect("锁未被毒化").pending
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
