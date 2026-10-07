//! TCP 客户端通道。
//!
//! 结构与 `serial_impl::channel.rs` 完全一致：一个后台线程独占 `TcpStream`，
//! UI 侧通过两条 mpsc 通道与它交互。停机走 [`WorkerCommand::Halt`]，
//! 状态归位靠 `StateGuard`（两者都在 `communicationtrait::worker` 里）。
//!
//! # 与串口实现的三处**实质性**差别
//!
//! 1. **对端可以主动下线**：`read` 返回 `Ok(0)` 是 TCP 的 EOF，含义是"对端正常
//!    关闭了连接"，此时发 [`ChannelEvent::Closed`] 而**不是** `Failed`。串口没有
//!    这个概念（USB 拔出在驱动层是读错误），所以那里只会发 `Failed`。
//!    不区分这两者的话，界面上"设备被拔了"和"对端正常关掉了"会长得一模一样。
//!
//! 2. **连接是同步建立的，`open()` 会等它出结果**。这与"阻塞 IO 不上 UI 线程"
//!    的约定有张力，取舍写在下面。
//!
//! 3. **对端未启动时连接会失败**，而失败原因（`ConnectionRefused`）恰恰是 TCP
//!    调试里最常见的一种情况，必须当场说清楚。
//!
//! # 为什么 `connect` 放在 `open()` 里同步做
//!
//! 串口那边是"先乐观置位 `connected = true`，失败等 `Failed` 事件回来再回滚"，
//! 结果是一个**已知缺陷**：`connected` 假阳性，界面会先闪一下"已连接"。
//! 串口上这个闪烁很短（驱动 `open` 很快），但 TCP 的 `connect` 失败要等 SYN
//! 重传，可能持续数秒——照搬乐观写法会让界面**明确地对用户撒谎好几秒**。
//!
//! 另一个方向（把 connect 挪进后台线程、加一个"正在连接"状态）才是教科书答案，
//! 但那要引入第三个连接状态并让界面、字幕、按钮全部跟着分叉，成本不小、收益
//! 只有观感。这里选择第三条路：**同步 connect + 3 秒超时**，代价是
//!
//!   * 连接期间 UI 卡住，最坏情况是超时长度（3 秒）；
//!   * 只发生在用户主动点「连接」时，且失败路径是**有界的**（地址写对时通常
//!     几毫秒返回）。
//!
//! 这个取舍与 `log_file.rs` 是一致的：那里也是"UI 线程 `File::create` 打开文件
//! 后再交给写盘线程"，注释里明说这是**刻意**不同于 `open_connection()` 乐观写法的选择。
//! 换句话说，本项目愿意为"不骗用户"付一次有界的阻塞。
//!
//! # 为什么写侧必须非阻塞（2026-09-24 修）
//!
//! 曾经的写法是：读侧 `set_read_timeout(10 ms)`，写侧保持**阻塞**、没有写超时。
//! 这在"对端一直在读"时看不出问题，但 TCP 的发送缓冲由**对端的接收窗口**决定，
//! 所以对端一停读，本地发送缓冲就会填满——然后 `write` **永不返回**。两条后果：
//!
//! * 后台线程卡在 `write` 上，指令队列再也排不到，**待发上限形同虚设**，
//!   真正无限增长的是那条无界的 mpsc 通道；
//! * `close()` 是"投 `Halt` 后 `join()`"，而 `Halt` 只能在两轮 I/O 之间被看到，
//!   于是点「断开」会一直等下去（实测：对端完全不读时，灌满约 2.75 MB 内核缓冲
//!   后 2 分 36 秒毫无进展）。
//!
//! **最容易想到的修法是错的**：给写侧补一个 `set_write_timeout`。实测（64 KB 一片、
//! 片内含片号、超时 10 ms、对端先停读 2 秒再全速读）表明，Windows 上带
//! `SO_SNDTIMEO` 的阻塞 `send` 超时后**数据可能已经进了内核缓冲并最终发出**，而
//! 调用方只拿到 `WSAETIMEDOUT`、**拿不到已写字节数**——worker 于是按原样重发整片，
//! 同一片在接收端出现了 **126** 遍（`126 × 8192` 个词）。流协议里重复字节比丢字节
//! 难查得多。
//!
//! 正解是让 `write` **永远立刻返回**，并由 `Ok(n)` 如实回报写出去多少：
//! `set_nonblocking(true)` 之后，发送窗口为零只是返回 `WouldBlock`，worker 把这一片
//! 留在队列里、下一轮再试，其它的活（读、排空指令、看 `Halt`）照常进行。
//! 于是上限真正生效、`close()` 立刻返回，而且**一个字节都不会重复**。
//!
//! 代价只有一个：非阻塞 fd 上 `set_read_timeout` 不再起作用（阻塞属性是整体的），
//! 空闲时的等待从"内核里等 10 ms"换成"worker 自己睡 1 ms"。
//!
//! 串口没有这个歧义：它的 `.timeout(10 ms)` 是**驱动层**语义，写超时后驱动会如实
//! 报告已写字节数，所以那里保留阻塞写 + 驱动超时即可，不需要非阻塞。
//!
//! # 第一版不做的两件事
//!
//! * **自动重连**：`ComTool::auto_reconnect` 依赖 `last_opened_port`，是串口语义；
//!   给 TCP 加上会在对端未启动时产生半死不活的 socket 状态，收益不明确。
//! * **TCP Server / 多客户端**：一个 `ByteChannel` 管一个对端，与"多客户端"是
//!   两种形态，属独立课题。

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::communicationtrait::worker::{
    ACTIVE, IDLE, PendingLedger, StateGuard, WorkerCommand, is_transient, retire_finished_chunk,
    submit,
};
use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::net_impl::config::NetConfig;

/// `connect` 的最长等待。
///
/// 3 秒是"局域网设备其实在线、只是慢了一拍"与"用户已经觉得卡了"之间的折中。
/// 公网地址可能需要更长，但这个工具的现实用法是**点对点调试**（设备直连、
/// 或本机起个 `nc`），3 秒足够；真需要长超时的场景应该由用户自己重试。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 两个方向都无事可做时，一轮循环让出的时间片。
///
/// 本通道的 socket 是**非阻塞**的（见 [`TcpChannel::open`]），所以两个方向都会立刻
/// 返回 `WouldBlock`——没有这一句就是 100% CPU 空转烧满一个核。串口那边靠
/// `.timeout(10 ms)` 让内核帮着等，非阻塞 socket 没有那层保护，只能自己睡。
///
/// 取 1 ms 而不是照搬 10 ms：它同时决定"待发数据的响应延迟"与"`close()` 投出停机
/// 指令后线程的最长响应延迟"。1 ms 下两者都远优于原来的 10 ms，而每秒一千次
/// 唤醒对 CPU 基本无感（`read` + 可选的一次 `write`，都是立即返回的失败调用）。
const IDLE_SLEEP: Duration = Duration::from_millis(1);

/// 单次 `read` 的缓冲上限。TCP 是流式的，一次 `read` 能拿到多少取决于对端
/// 怎么切分 `send`，所以缓冲稍大无妨——它只影响一轮循环搬多少，不改变语义。
const READ_CHUNK: usize = 4 * 1024;

/// 后台线程主体：独占已连接好的 `TcpStream`，搬运两个方向的数据。
fn worker_main(
    mut stream: TcpStream,
    commands: Receiver<WorkerCommand>,
    events: Sender<ChannelEvent>,
    state: Arc<AtomicU8>,
    ledger: PendingLedger,
) {
    let _guard = StateGuard(Arc::clone(&state));

    let mut buf = vec![0u8; READ_CHUNK];
    // 待发送队列，队首是正在发送的那一段 + 已写出多少字节。
    //
    // 与串口同样的理由：`write` 可能只吃掉一部分（发送窗口满），而
    // `write_all` 在部分写之后失败时不报告已写字节数，会静默丢数据。
    // TCP 上还要多一层：对端关闭后 `write` 会返回 `BrokenPipe` /
    // `ConnectionReset`，那是真故障，走 `Failed` 上报。
    let mut outgoing: VecDeque<(Vec<u8>, usize)> = VecDeque::new();

    // 上一轮是否什么都没做成（读、写、指令三处都没进展）。用于决定要不要睡——
    // 非阻塞 socket 下"什么都没有"是常态，不睡就是空转。
    let mut idle = false;

    loop {
        if idle {
            thread::sleep(IDLE_SLEEP);
        }
        idle = true;

        // ———— 方向一：对端 → UI ————
        match stream.read(&mut buf) {
            // TCP 的 EOF：对端**正常**关闭了连接。这是与串口最本质的差别。
            Ok(0) => {
                let _ = events.send(ChannelEvent::Closed);
                return;
            }
            Ok(n) => {
                idle = false;
                // 接收端已被丢弃，说明 UI 已经退出，没有继续搬数据的理由。
                if events.send(ChannelEvent::Bytes(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) if is_transient(&e) => {}
            Err(e) => {
                let _ = events.send(ChannelEvent::Failed(format!("接收失败: {e}")));
                return;
            }
        }

        // ———— 方向二：UI → 对端 ————
        // 每轮最多推进一次写，避免长报文占满循环、拖慢停机响应。
        let mut chunk_done = false;
        if let Some((bytes, sent)) = outgoing.front_mut() {
            // 非阻塞下的三种结果都**立即**返回，没有一种是"等对端腾地方"：
            // `Ok(n)` 是挤进去了 n 个字节，`WouldBlock` 是这次一个都没挤进去
            // （发送窗口为零，或对端彻底停读），下一轮再来。所以对端不读**不会**
            // 把这一轮拖住，`Halt` 也就能按时被看到。
            match stream.write(&bytes[*sent..]) {
                // 非阻塞下不该出现，保留以对应 `Write` 契约里的"写了 0 个字节"。
                Ok(0) => {}
                Ok(n) => {
                    idle = false;
                    // 只推进"确实写出去的"那么多：进度由驱动如实回报，因此不会
                    // 重发、也不会丢——这是"加一个写超时"那条路走不通的地方
                    // （超时的 `send` 可能已经把数据送出去了，调用方却拿不到计数）。
                    *sent += n;
                    chunk_done = *sent == bytes.len();
                }
                Err(e) if is_transient(&e) => {}
                Err(e) => {
                    let _ = events.send(ChannelEvent::Failed(format!("发送失败: {e}")));
                    return;
                }
            }
        }
        // 整段确实写完之后才出队并把额度还给调用线程；进度由 `Ok(n)` 如实回报，
        // 所以不会多还也不会少还。
        retire_finished_chunk(&mut outgoing, chunk_done, &ledger);

        // ———— 接收指令 ————
        // 一次排空，避免高速收发时指令积压；`Halt` 在任何时刻都能被看到
        // （最迟在下一轮循环后），因此 `close()` 的等待时长始终由一轮读写决定
        // ——**这里曾经不成立**：写侧是阻塞 socket 时对端一停读就永久卡在
        // `write` 上，`Halt` 再也看不到，点「断开」会一直等下去。
        loop {
            match commands.try_recv() {
                // 队列**直接 push**：容量已经在调用线程里由 `worker::submit` 预约过了，
                // 这里不再判一遍（上限只有一处实现）。
                Ok(WorkerCommand::Transmit(bytes)) => {
                    idle = false;
                    outgoing.push_back((bytes, 0));
                }
                // 网络通道没有 RTS / DTR：`ByteChannel::set_line_state` 的默认实现
                // 直接报 `Unsupported`，所以这条指令到不了这里。写出来只是让 match
                // 穷尽——顺带说明"三个实现共用同一个指令枚举"是编译期强制同步的。
                Ok(WorkerCommand::SetLineState { .. }) => {}
                Ok(WorkerCommand::Halt) => return,
                Err(TryRecvError::Empty) => break,
                // UI 侧把 Sender 丢了，说明对象正在析构，同样收工。
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}

/// 一个 TCP 客户端通道。
pub struct TcpChannel {
    cfg: NetConfig,
    state: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
    commands: Option<Sender<WorkerCommand>>,
    /// 待发容量的账本。见 [`worker::PendingLedger`] 的文档。
    ledger: PendingLedger,
}

impl TcpChannel {
    pub fn new() -> Self {
        Self {
            cfg: NetConfig::default(),
            state: Arc::new(AtomicU8::new(IDLE)),
            worker: None,
            commands: None,
            ledger: PendingLedger::default(),
        }
    }
}

impl Default for TcpChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteChannel for TcpChannel {
    fn open(&mut self, sink: Sender<ChannelEvent>) -> io::Result<()> {
        if self.is_active() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "TCP 通道已经在运行",
            ));
        }

        // 地址解析与 connect 都在这里同步完成：`open()` 返回 `Ok` 就意味着
        // "确实连上了"，而不是"线程起来了"（见模块文档里的取舍说明）。
        let addr = self.cfg.remote_addr()?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| io::Error::new(e.kind(), format!("连接 {addr} 失败: {e}")))?;

        // 交互式调试工具不该为了攒包而等待：小报文（几字节的指令）在 Nagle
        // 下会被压到 40 ms 才出去，而串口工具的用户已经习惯了"点一下就走"。
        stream.set_nodelay(true)?;

        // **两个方向都设为非阻塞**（而不是只给读设一个超时）。
        //
        // 这是本通道唯一的"防卡死"手段，理由在模块文档「为什么写侧必须非阻塞」那节。
        // 一句话：阻塞写的超时**给不出已写字节数**，于是"超时重试"会把同一段数据
        // 重发出去；而非阻塞写由 `Ok(n)` 如实回报进度，既有界、又不重复。
        //
        // 代价是读侧也不能再靠 `set_read_timeout` 让内核帮着等（同一个 fd 的阻塞
        // 属性是整体的），改为在 [`worker_main`] 里空闲时自己睡 [`IDLE_SLEEP`]。
        stream.set_nonblocking(true)?;

        let state = Arc::clone(&self.state);
        let worker_ledger = self.ledger.clone();
        let (commands, orders) = mpsc::channel::<WorkerCommand>();
        self.commands = Some(commands);

        // 账本归零。通道对象在连接类型不变时会被**就地复用**（重连就是"同一个句柄
        // 再 open 一次"），上一次退出时队列里可能还压着没写完的数据、额度没还。
        self.ledger.reset();

        state.store(ACTIVE, Ordering::Release);
        self.worker = Some(thread::spawn(move || {
            worker_main(stream, orders, sink, state, worker_ledger);
        }));
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        // 先取出 Sender 再发停机指令，随后立刻 drop 它。这样即便 `Halt`
        // 因为某种原因没被读到，后台线程也会观察到 `Disconnected` 而退出。
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(WorkerCommand::Halt);
        }

        if let Some(handle) = self.worker.take() {
            handle
                .join()
                .map_err(|_| io::Error::other("TCP 后台线程 panic，资源可能未正确释放"))?;
        }

        self.state.store(IDLE, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// 交一段字节出去。**队列满会当场返回 `Err`**（判定在调用线程里做，
    /// 靠 [`PendingLedger`] 的额度预约，见 `worker::submit` 的文档）。
    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "TCP 尚未连接"));
        };
        submit(&self.ledger, commands, bytes, "TCP")
    }

    /// 待发队列里还没写出去的字节数（诊断读数，显示在状态栏）。
    ///
    /// 读的是 [`PendingLedger`]：额度由**调用线程**在投递前预约、由后台线程整段写完后
    /// 归还，所以这个数就是"已经答应要发、但还没送出去"的字节。界面用它来回答
    /// "发送看起来成功、对端却没收到"——`write` 被拒时有提示，而**没满却一直写不动**
    /// 原先没有任何可见痕迹（`HANDOVER.md` §5「写路径瞬态错误静默重试」）。
    fn pending_bytes(&self) -> usize {
        self.ledger.bytes()
    }

    fn configure(&mut self, settings: &dyn std::any::Any) -> io::Result<()> {
        let cfg = settings
            .downcast_ref::<NetConfig>()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "配置类型不是 NetConfig"))?;
        self.cfg = cfg.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communicationtrait::worker::MAX_PENDING_BYTES;
    use std::net::TcpListener;
    use std::time::Instant;

    /// 一个基本不可能有人监听的端口（1024 以下的特权端口，本进程不是
    /// root/管理员时也绑不上），用于稳定触发「连接被拒绝」。
    const CLOSED_PORT: u16 = 1;

    fn config_to(port: u16) -> NetConfig {
        NetConfig {
            remote_host: "127.0.0.1".to_string(),
            remote_port: port,
            local_port: 0,
        }
    }

    /// 等到 `predicate` 成立或超时。用它代替固定的 `sleep`：CI 与开发机上
    /// 的调度延迟差别很大，固定睡眠不是太慢就是不稳。
    fn wait_until(mut predicate: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        predicate()
    }

    #[test]
    fn new_channel_is_idle() {
        assert!(!TcpChannel::new().is_active(), "新建通道不应处于活动状态");
    }

    /// 需求：`close` 幂等——从未打开过、或重复调用都不得报错。
    #[test]
    fn close_is_idempotent_on_unopened_channel() {
        let mut ch = TcpChannel::new();
        ch.close().expect("对从未打开的通道调用 close 应成功");
        ch.close().expect("重复调用 close 应成功");
        assert!(!ch.is_active());
    }

    /// 需求：未打开时写入必须明确失败，不能 panic，也不能静默丢弃。
    #[test]
    fn write_before_open_is_rejected() {
        let mut ch = TcpChannel::new();
        let err = ch
            .write(vec![0xA5, 0x5A])
            .expect_err("未连接时写入必须返回错误");
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    /// 需求：`configure` 只接受本通道认识的配置类型。
    #[test]
    fn configure_rejects_foreign_type() {
        let mut ch = TcpChannel::new();
        let err = ch
            .configure(&42u32)
            .expect_err("非 NetConfig 类型必须被拒绝");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // 串口配置也必须被拒绝：两个 `&dyn Any` 入口长得很像，转型失败是最容易
        // 悄悄发生的错误（传错了就会"用串口参数去连 TCP"）。
        let serial_cfg = crate::serial_impl::SerialConfig::default();
        assert!(ch.configure(&serial_cfg).is_err());
    }

    /// 需求：连不上时必须由 `open()` **当场**报错（不是先 Ok 再回滚）。
    ///
    /// 这条正是本实现与串口实现的关键差异：`open` 返回 `Ok` 就等于"确实连上了"，
    /// 因此 UI 侧不需要靠一个"乐观置位 + 事后回滚"来猜。
    #[test]
    fn refused_connection_is_reported_by_open_itself() {
        let mut ch = TcpChannel::new();
        ch.configure(&config_to(CLOSED_PORT)).expect("配置应被接受");

        let (tx, _rx) = mpsc::channel();
        let err = ch.open(tx).expect_err("连不上的端口必须让 open 直接失败");
        assert!(
            err.to_string().contains("连接"),
            "错误信息应说清是连接失败，实际为: {err}"
        );
        assert!(!ch.is_active(), "open 失败后不能处于活动状态");
        // 失败也要保持幂等关闭
        ch.close().expect("关闭一个从未成功打开的通道应成功");
    }

    /// 地址非法时同样由 `open()` 报错（走 `NetConfig::validate`）。
    #[test]
    fn invalid_config_is_rejected_by_open() {
        let mut ch = TcpChannel::new();
        ch.configure(&NetConfig {
            remote_host: String::new(),
            ..NetConfig::default()
        })
        .expect("配置本身应被接受（校验发生在 open 时）");
        let (tx, _rx) = mpsc::channel();
        let err = ch.open(tx).expect_err("空地址必须让 open 失败");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// **主链路集成测试**：连上 → 双向收发 → 对端关闭 → 状态复位。
    ///
    /// 全程只用回环地址，因此不需要任何硬件，可以直接进 `cargo test`——
    /// TCP 是本计划里唯一能在门禁内被自动验完的网络功能。
    #[test]
    fn connects_exchanges_bytes_and_resets_on_peer_close() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环上应能监听");
        let port = listener.local_addr().expect("应能取到监听地址").port();

        // 对端：接受一次连接，收一条、回一条，然后**主动关闭**。
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("应能接受连接");
            let mut buf = [0u8; 16];
            let n = stream.read(&mut buf).expect("应能读到数据");
            let received = buf[..n].to_vec();
            stream.write_all(b"pong").expect("应能把回包写出去");
            stream.flush().expect("flush 应成功");
            // 这里 drop(stream) —— 对端正常关闭，客户端应收到 Closed 而不是 Failed
            received
        });

        let mut ch = TcpChannel::new();
        ch.configure(&config_to(port)).expect("配置应被接受");

        let (tx, rx) = mpsc::channel();
        ch.open(tx).expect("回环连接应立即成功");
        assert!(ch.is_active(), "连接成功后应处于活动状态");

        ch.write(b"ping".to_vec()).expect("写入应被接受");

        // 一次把事件排空到底：`pong` 与 `Closed` 可能落在同一轮或相邻两轮
        // （对端写完就 drop，两个事件之间只隔一个 `read`）。
        let mut got: Vec<u8> = Vec::new();
        let mut saw_close = false;
        let mut failure: Option<String> = None;
        let mut notice: Option<String> = None;

        wait_until(
            || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        ChannelEvent::Bytes(data) => got.extend_from_slice(&data),
                        ChannelEvent::Closed => saw_close = true,
                        // 对端正常关闭必须走 Closed。收成 Failed 就是实现错了，
                        // 记下来在断言里说清楚，而不是在闭包里 panic。
                        ChannelEvent::Failed(msg) => failure = Some(msg),
                        // TCP 没有 RTS/DTR，也不该产生别的非致命通知。真收到了
                        // 说明事件路由串了，同样记下来交给断言。
                        ChannelEvent::Notice(msg) => notice = Some(msg),
                    }
                }
                saw_close || failure.is_some() || notice.is_some()
            },
            Duration::from_secs(5),
        );

        assert_eq!(failure, None, "对端正常关闭必须上报 Closed，而不是 Failed");
        assert_eq!(notice, None, "TCP 通道不应产生非致命通知（Notice）");
        assert!(saw_close, "对端关闭后必须上报 Closed");
        assert_eq!(got, b"pong", "对端回包的字节应原样送达");
        assert!(
            wait_until(|| !ch.is_active(), Duration::from_secs(5)),
            "后台线程退出后 is_active 必须回到 false"
        );

        ch.close().expect("关闭已自行退出的通道应成功");
        assert!(!ch.is_active());

        let peer_received = peer.join().expect("对端线程不应 panic");
        assert_eq!(peer_received, b"ping", "对端应收到我们写的字节");
    }

    /// 已经打开的通道再次 `open` 必须被拒绝（而不是悄悄拉起第二个线程）。
    #[test]
    fn open_twice_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环上应能监听");
        let port = listener.local_addr().expect("应能取到监听地址").port();
        let peer = thread::spawn(move || {
            // 接受并保持连接，直到客户端 close
            let _accepted = listener.accept();
            thread::sleep(Duration::from_millis(200));
        });

        let mut ch = TcpChannel::new();
        ch.configure(&config_to(port)).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx.clone()).expect("首次连接应成功");

        let err = ch.open(tx).expect_err("重复 open 必须被拒绝");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);

        ch.close().expect("关闭应成功");
        peer.join().expect("对端线程不应 panic");
    }

    /// ⭐ **背压的判据落在调用线程上**：一条自己就超过上限的负载必须**当场**被拒。
    ///
    /// 2026-09-24 之前，上限判在后台线程的队列上，`write()` 一律返回 `Ok`，于是
    /// `app::ComTool::transmit` 照常记账、照常清空输入框——用户会看到"本条未发送"
    /// 的提示与仍在增长的发送计数并存。这条钉的就是那个缺口已经堵上。
    ///
    /// 附带断言"被拒的那一条不占额度"：不然后续每一次发送都会被误拒，而用户看到的
    /// 只是"没发多少就报队列满"。
    #[test]
    fn oversized_payload_is_refused_at_the_call_site() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环上应能监听");
        let port = listener.local_addr().expect("应能取到监听地址").port();
        let peer = thread::spawn(move || {
            // 接受连接后什么也不做：队列排得多快全看我们自己。
            let _accepted = listener.accept();
            thread::sleep(Duration::from_millis(300));
        });

        let mut ch = TcpChannel::new();
        ch.configure(&config_to(port)).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("回环连接应立即成功");

        let err = ch
            .write(vec![0u8; MAX_PENDING_BYTES + 1])
            .expect_err("单条就超过上限的负载必须当场被拒");
        assert!(
            err.to_string().contains("待发队列已满"),
            "错误信息应说清是队列满（调用方会原样显示给用户），实际为: {err}"
        );

        ch.write(vec![1u8; 16])
            .expect("被拒绝的那一条不该占着容量，后续小负载应仍能排上队");

        ch.close().expect("关闭应成功");
        peer.join().expect("对端线程不应 panic");
    }

    /// ⭐⭐ **对端不读时，TCP 也必须能立刻断开**（2026-09-24 修的核心判据）。
    ///
    /// 判据选在 `close()` 上，因为它一次暴露老实现的**两个后果**：写侧一旦阻塞，
    /// worker 就再也看不到 `Halt`，`join()` 会一直等下去（实测 2 分 36 秒毫无进展）；
    /// 同时指令队列不再被排空，待发上限也就形同虚设。
    ///
    /// 之所以把 `close()` 放进独立线程再 `recv_timeout`：万一这条回归了，它要报
    /// "超时失败"，而不是把整个测试套件一起挂住——挂住的套件比失败的用例难查得多。
    #[test]
    fn close_returns_promptly_even_when_the_peer_never_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环上应能监听");
        let port = listener.local_addr().expect("应能取到监听地址").port();

        // 对端接受连接后**一个字节都不读**，正好把我们的发送窗口掐死。
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            let _accepted = listener.accept().expect("应能接受连接");
            let _ = accepted_tx.send(());
            // 关键：accept 之后**一个字节都不读**，直到测试结束（连接随之关闭）。
            //
            // ⚠️ 这里**没有**去压对端的接收窗口：`std::net::TcpStream` 不提供
            // `SO_RCVBUF` 的 setter（那是 `socket2` 才有的），为它引入一个依赖不值得。
            // 代价是"多少字节才灌得满"仍然由机器决定——**正因为如此，下面才必须用
            // 「停滞」当判据，而不能用任何绝对字节数**（详见那里）。
            let _ = stop_rx.recv_timeout(Duration::from_secs(30));
        });

        let mut ch = TcpChannel::new();
        ch.configure(&config_to(port)).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("回环连接应立即成功");
        // 连上之后再等对端"我已经接受了"的信号：反过来的话会等在一个永远不会发生的
        // 事件上（accept 要等 connect 先到）。
        accepted_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("对端应已接受连接");

        // 先把**内核发送缓冲**灌满——这一步不能省。账本满只说明"有 1 MiB 压着还没
        // 写出去"，而对端不读时真正会让 `write` 卡住的是内核缓冲；只把账本填满就
        // 收手的话，内核可能还有空间把积压全吃掉，worker 于是根本没被卡住，
        // 这条测试就成了空转（第一版就是这么假绿的）。
        //
        // ⭐ 判据是"**写不动了**"，**不是**"投出去多少字节"。内核缓冲有多大随系统而
        // 异，拿一个绝对字节数当门槛，换台机器就会红——远程 CI 正是红在这里
        // （原来要求 2 MiB，而 windows-latest 的 runner 两秒只投出 1 507 328 字节）。
        // 这条测试要守的东西（`close()` 不被写卡住）与缓冲多大没有关系，判据就该盯
        // 前者而不是后者。
        //
        // "写不动"的可观测证据是**停滞**：连续一段时间里一次都没能成功入队，说明
        // worker 一个字节都挤不出去了。它在任何机器上都是同一个现象。
        const STALL: Duration = Duration::from_millis(500);
        const GIVE_UP: Duration = Duration::from_secs(12);
        let chunk = vec![0u8; 64 * 1024];
        let deadline = Instant::now() + GIVE_UP;
        let mut pushed = 0usize;
        let mut refusals = 0usize;
        let mut last_progress = Instant::now();
        while Instant::now() < deadline {
            match ch.write(chunk.clone()) {
                Ok(()) => {
                    pushed += chunk.len();
                    last_progress = Instant::now();
                }
                Err(e) if e.to_string().contains("待发队列已满") => {
                    refusals += 1;
                    // 让 worker 有机会把缓冲里的数据挤出去，再把额度填回来。
                    thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("不该出现其它写错误: {e}"),
            }
            // 已经停滞就收手，不必耗满 12 秒；`refusals > 0` 保证不是"一次都没投成"。
            if refusals > 0 && last_progress.elapsed() >= STALL {
                break;
            }
        }
        assert!(
            refusals > 0,
            "对端一个字节都不读，投到这个量必然排不上队；一次没轮到说明上限根本没生效"
        );
        assert!(
            last_progress.elapsed() >= STALL,
            "收手时仍在成功入队（共 {pushed} 字节）⇒ 内核缓冲还没满，这条测试没测到\
             要测的东西"
        );
        assert!(
            ch.pending_bytes() >= MAX_PENDING_BYTES / 2,
            "停滞时应有大半额度压在队列里，实际只有 {} 字节",
            ch.pending_bytes()
        );

        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = ch.close();
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(3)).is_ok(),
            "对端不读时 close() 也必须立刻返回：阻塞写会让后台线程永远卡在 write 上，\
             点「断开」就再也回不来了"
        );

        let _ = stop_tx.send(());
    }

    /// ⭐ **`open()` 必须把账本归零**，否则重连后容量会凭空少一块。
    ///
    /// 为什么这条单独要测：通道对象在连接类型不变时会被**就地复用**（重连就是
    /// "同一个句柄再 `open` 一次"，见 `app::ComTool::open_connection`），而 `close()`
    /// 时队列里那些**确实没写出去**的数据的额度是不会被归还的（它们确实没出去）。
    /// 不清的话，用户第二次连上以后会发现"什么都没发就报队列满"。
    ///
    /// 做法是直接把账本压满来模拟"上次留下的额度"——本模块内的测试摸得到通道自己的
    /// 账本，不必真的去造一个"写到一半就断电"的场景。
    #[test]
    fn reopening_the_channel_clears_the_stale_quota() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环上应能监听");
        let port = listener.local_addr().expect("应能取到监听地址").port();
        let peer = thread::spawn(move || {
            // 接受两次连接：第一次用完就断，第二次是重连。
            let _first = listener.accept();
            let _second = listener.accept();
            thread::sleep(Duration::from_millis(200));
        });

        let mut ch = TcpChannel::new();
        ch.configure(&config_to(port)).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("首次连接应成功");
        ch.close().expect("关闭应成功");

        // 模拟上一次运行留下的、没还回来的额度。
        ch.ledger
            .reserve(MAX_PENDING_BYTES)
            .expect("额度充足时应能占满");
        assert!(ch.ledger.reserve(1).is_err(), "占满之后就该拒绝了");

        let (tx2, _rx2) = mpsc::channel();
        ch.open(tx2).expect("重连应成功");
        assert_eq!(
            ch.ledger.bytes(),
            0,
            "open 必须把上一次留下的额度清掉，否则重连后的容量凭空少一块"
        );
        ch.write(vec![1u8; 8])
            .expect("容量已经回来了，小负载应该能排上队");

        ch.close().expect("关闭应成功");
        peer.join().expect("对端线程不应 panic");
    }
    /// ⭐ **待发读数必须真的接上账本**（`HANDOVER.md` §5「写路径瞬态错误静默重试」的接入测试）。
    ///
    /// 缺陷的原样是：`write` 被拒时用户有提示，但**队列没满却一直写不动**时完全
    /// 没有可见痕迹——界面上唯一能显示它的地方就是这条读数。
    ///
    /// 为什么必须**通过 trait 方法**断言（而不是直接读 `ch.ledger.bytes()`）：
    /// 账本本身早就有别的用例覆盖，界面拿到的那个数走的是
    /// `Box<dyn ByteChannel>` 上的 `pending_bytes()`。忘了覆盖它时 trait 的默认
    /// 实现会**一直返回 0**——只写 `ledger.bytes()` 的断言照样全绿，那正是空转。
    #[test]
    fn pending_bytes_reports_what_the_ledger_holds() {
        let ch = TcpChannel::new();
        assert_eq!(ByteChannel::pending_bytes(&ch), 0, "没排队时读数应当是 0");

        ch.ledger.reserve(4096).expect("额度充足，应当预约成功");

        // 换成 trait 方法读（界面拿到的就是 `Box<dyn ByteChannel>` 上的这一条）。
        let channel: &dyn ByteChannel = &ch;
        assert_eq!(
            channel.pending_bytes(),
            4096,
            "账本里明明压着 4096 字节，待发读数却是 0 —— 界面上那个数会是死的"
        );
    }
}
