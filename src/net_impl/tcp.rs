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

use crate::communicationtrait::worker::{ACTIVE, IDLE, StateGuard, WorkerCommand, is_transient};
use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::net_impl::config::NetConfig;

/// `connect` 的最长等待。
///
/// 3 秒是"局域网设备其实在线、只是慢了一拍"与"用户已经觉得卡了"之间的折中。
/// 公网地址可能需要更长，但这个工具的现实用法是**点对点调试**（设备直连、
/// 或本机起个 `nc`），3 秒足够；真需要长超时的场景应该由用户自己重试。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 单次 `read` 的最长等待。
///
/// 与串口取同一个值（10 ms），理由相同：它同时决定空闲时的唤醒频率与
/// `close()` 投出停机指令后线程的最长响应延迟。
const READ_TIMEOUT: Duration = Duration::from_millis(10);

/// 单次 `read` 的缓冲上限。TCP 是流式的，一次 `read` 能拿到多少取决于对端
/// 怎么切分 `send`，所以缓冲稍大无妨——它只影响一轮循环搬多少，不改变语义。
const READ_CHUNK: usize = 4 * 1024;

/// 后台线程主体：独占已连接好的 `TcpStream`，搬运两个方向的数据。
fn worker_main(
    mut stream: TcpStream,
    commands: Receiver<WorkerCommand>,
    events: Sender<ChannelEvent>,
    state: Arc<AtomicU8>,
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

    loop {
        // ———— 方向一：对端 → UI ————
        match stream.read(&mut buf) {
            // TCP 的 EOF：对端**正常**关闭了连接。这是与串口最本质的差别。
            Ok(0) => {
                let _ = events.send(ChannelEvent::Closed);
                return;
            }
            Ok(n) => {
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
            match stream.write(&bytes[*sent..]) {
                // 这一轮一个字节都没挤进去，等下一轮重试。
                Ok(0) => {}
                Ok(n) => {
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
        if chunk_done {
            outgoing.pop_front();
        }

        // ———— 接收指令 ————
        // 一次排空，避免高速收发时指令积压；`Halt` 在任何时刻都能被看到
        // （最迟在下一轮 `read` 超时后），因此 `close()` 的等待时长始终由
        // 一轮读写决定，不被待发数据拖住。
        loop {
            match commands.try_recv() {
                Ok(WorkerCommand::Transmit(bytes)) => outgoing.push_back((bytes, 0)),
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
}

impl TcpChannel {
    pub fn new() -> Self {
        Self {
            cfg: NetConfig::default(),
            state: Arc::new(AtomicU8::new(IDLE)),
            worker: None,
            commands: None,
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
        stream.set_read_timeout(Some(READ_TIMEOUT))?;

        let state = Arc::clone(&self.state);
        let (commands, orders) = mpsc::channel::<WorkerCommand>();
        self.commands = Some(commands);

        state.store(ACTIVE, Ordering::Release);
        self.worker = Some(thread::spawn(move || {
            worker_main(stream, orders, sink, state);
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

    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "TCP 尚未连接"));
        };
        commands
            .send(WorkerCommand::Transmit(bytes))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "TCP 后台线程已退出"))
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
}
