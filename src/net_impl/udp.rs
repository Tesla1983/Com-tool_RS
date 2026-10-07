//! UDP 通道。
//!
//! 结构与 [`crate::net_impl::tcp`] 同形（后台线程 + `Halt` 停机 + `StateGuard`
//! 归位），差别全在 UDP 的语义上——这些差别必须体现在**文案与注释**里，
//! 否则用户会以为 UDP 也有握手。
//!
//! # UDP 与 TCP 的四处语义差别
//!
//! 1. **没有连接**。`is_active` 只能表达"本地 socket 已绑定"，因此界面上的
//!    状态词必须是「**已绑定**」而不是「已连接」——说成"已连接"会让用户以为
//!    对端在线、以为掉了线会被发现。事实上 UDP 发出去的数据有没有人收，
//!    本地**无从得知**。
//!
//! 2. **不会产生 [`ChannelEvent::Closed`]**。没有握手就没有"对端正常关闭"这个
//!    事件。本通道只发 `Bytes` 与 `Failed`。
//!
//! 3. **`Ok(0)` 不是 EOF**。`recv` 返回 0 表示收到一个**零长度数据报**，
//!    那是合法数据，不是对端下线（TCP 的 `Ok(0)` 才是 EOF）。
//!
//! 4. **Windows 上"目标没在监听"会被报成本地错误**。对已 `connect` 的 UDP
//!    socket，收到 ICMP port unreachable 时 Windows 会让**下一次 `recv`**
//!    返回 `ConnectionReset`。那是网络层面的正常现象（对端就没开），把它当致命
//!    错误会让用户第一次往没监听的端口发数据就把通道关掉。所以这里把它归入
//!    **可忽略**，见 [`is_ignorable`]。
//!
//! # 为什么 UDP 也改成了非阻塞（与 TCP 的又一处差别）
//!
//! TCP 侧在 2026-09-24 从阻塞写改成了非阻塞写，理由是**对端不读会把本地发送缓冲
//! 灌满**（TCP 的发送缓冲受对端接收窗口节制），`write` 于是永不返回、worker 卡死、
//! 待发上限失效、`close()` 也回不来。详见 `net_impl::tcp` 的模块文档。
//!
//! **UDP 曾经刻意保持阻塞**，理由当时是充分的：它没有流控，对端读不读**不参与**
//! 本地发送缓冲的判定——收不下就是丢掉，发送端照样返回成功。实测（对端 socket
//! 绑好端口后一个字节都不收，发送端 SO_SNDBUF 压到 4096）：连发 4000 个 60 KB
//! 报文，**0 次失败、单次最长 0.1 ms**；同一台机器上换成 TCP 做同样的事，
//! 2 分 05 秒毫无进展。也就是说"对端不读 ⇒ worker 卡死"这条路径在 UDP 上不存在。
//!
//! 2026-10-07 改成非阻塞，收的是**另一条**残余风险：**本机网卡出口饱和**（不是
//! "对端不读"）。出口发不出去时阻塞的 `send` 同样会把 worker 停在原地——`Halt`
//! 看不见、`close()` 回不来、待发上限形同虚设，表现与 TCP 那次一模一样。它罕见，
//! 但 `HANDOVER.md` §5 记的处置办法本来就是"按 TCP 同样的方式改非阻塞"，而代价
//! 只是"空闲时自己睡"这一项，于是做掉。
//!
//! 为什么不用 `SO_SNDTIMEO` 补一个写超时（TCP 那边已实测排除过同一条路）：超时的
//! `send` **拿不到已写字节数**，worker 无从判断这个数据报到底出没出去，重试就会
//! 重复发出去。非阻塞的 `WouldBlock` 没有这个歧义——一个字节都没排队，下一轮再来。
//!
//! 代价与 TCP 相同：非阻塞 fd 上 `set_read_timeout` 失效（阻塞属性是**每个 fd**
//! 整体的），空闲等待改为 worker 自己睡 [`IDLE_SLEEP`]。
//!
//! ⚠️ **这条改动没有、也无法被单元测试证明**：要触发它得先把真实网卡的出口打满，
//! 而门禁里连一个能堵住的回环都造不出来（`cargo test` 里 UDP 发 4000 × 60 KB
//! 给不读的对端是 0 次失败）。下面的测试只覆盖"改完之后收发与 `close()` 仍然正常"
//! ——它们是**防回归**，不是这条改动生效的证据。
//!
//! # 为什么要 `connect` 一个无连接的 socket
//!
//! 调用 `UdpSocket::connect` **不建立任何连接**（不发包、不等响应），它只做两件
//! 事：把目标地址固化进 socket，让 `recv` **只**接收来自该地址的数据报；
//! 以及让后续可以用 `send`/`recv` 而不是 `send_to`/`recv_from`。
//!
//! 对调试工具这正是想要的：用户在界面上指定了一个对端，就不该收到局域网里
//! 别人发来的广播包——那会让接收区出现无法解释的内容。代价是"只收指定对端的
//! 数据"，这个限制必须写在界面上（见发送区/侧边栏的说明）。

use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
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

/// 两个方向都无事可做时，一轮循环让出的时间片。
///
/// 本通道的 socket 是**非阻塞**的（见 [`UdpChannel::open`]），所以 `recv` / `send`
/// 都会立刻返回 `WouldBlock`——没有这一句就是 100% CPU 空转烧满一个核。串口那边靠
/// `.timeout(10 ms)` 让内核帮着等，非阻塞 socket 没有那层保护，只能自己睡。
///
/// 取 1 ms（与 TCP 同值、同理）：它同时决定"待发数据的响应延迟"与"`close()` 投出
/// 停机指令后线程的最长响应延迟"，而每秒一千次唤醒对 CPU 基本无感。注意只有在
/// **读写都没做成**、且**没有新指令**时才睡——有数据要发时循环不睡，吞吐不受它限制。
const IDLE_SLEEP: Duration = Duration::from_millis(1);

/// 单个数据报的接收缓冲上限。
///
/// UDP 的**一个**数据报超过这个长度会被**截断**（多出来的部分直接丢弃，这是
/// UDP 的固有行为，不是本实现的缺陷）。65535 是 UDP 报文的理论上限，取 64 KiB
/// 覆盖到它、同时避免为每个通道预留过多内存。
const DATAGRAM_MAX: usize = 64 * 1024;

/// 除三类通用可忽略错误之外，UDP 还要额外忽略的两种。
///
/// `ConnectionReset` / `ConnectionRefused`：对已 `connect` 的 UDP socket，
/// 目标主机回 ICMP port unreachable 时 Windows 会在**下一次 `recv`/`send`**
/// 上报这两种错误。那不是本地故障——对端就是没在监听，而这恰恰是 UDP 调试里
/// 最常见的一种情况。当成致命错误会让通道莫名其妙地自己关掉。
fn is_ignorable(err: &io::Error) -> bool {
    is_transient(err)
        || matches!(
            err.kind(),
            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused
        )
}

/// 后台线程主体：独占已绑定的 `UdpSocket`，搬运两个方向的数据。
fn worker_main(
    socket: UdpSocket,
    commands: Receiver<WorkerCommand>,
    events: Sender<ChannelEvent>,
    state: Arc<AtomicU8>,
    ledger: PendingLedger,
) {
    let _guard = StateGuard(Arc::clone(&state));

    let mut buf = vec![0u8; DATAGRAM_MAX];
    // 与其它两个通道同样的待发队列（理由见 `serial_impl::channel`）。
    // UDP 的 `send` 基本不会部分写，但保留同一个结构让三个实现可以逐段对照。
    let mut outgoing: VecDeque<(Vec<u8>, usize)> = VecDeque::new();

    // 上一轮是否什么都没做成（读、写、指令三处都没进展）。非阻塞 socket 下"什么都
    // 没有"是常态，不据此睡觉就是空转；有进展时一刻也不睡，吞吐不受影响。
    let mut idle = false;

    loop {
        if idle {
            thread::sleep(IDLE_SLEEP);
        }
        idle = true;

        // ———— 方向一：对端 → UI ————
        match socket.recv(&mut buf) {
            // **不是 EOF**：收到一个零长度数据报。忽略即可，绝不能当成关闭。
            Ok(0) => {}
            Ok(n) => {
                idle = false;
                if events.send(ChannelEvent::Bytes(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) if is_ignorable(&e) => {}
            Err(e) => {
                let _ = events.send(ChannelEvent::Failed(format!("接收失败: {e}")));
                return;
            }
        }

        // ———— 方向二：UI → 对端 ————
        // 每轮最多推进一次写：长报文不该占满循环、拖慢停机响应。
        let mut chunk_done = false;
        if let Some((bytes, sent)) = outgoing.front_mut() {
            // 非阻塞下的三种结果都**立即**返回，没有一种是"等网卡腾地方"：
            // `Ok(n)` 是排队了 n 个字节，`WouldBlock` 是这次一个都没排进去（出口
            // 饱和），下一轮再来。所以出口堵住**不会**把这一轮拖住，`Halt` 也就能
            // 按时被看到——这正是本次从阻塞改过来的全部理由。
            match socket.send(&bytes[*sent..]) {
                Ok(0) => {}
                Ok(n) => {
                    idle = false;
                    *sent += n;
                    chunk_done = *sent == bytes.len();
                }
                Err(e) if is_ignorable(&e) => {}
                Err(e) => {
                    let _ = events.send(ChannelEvent::Failed(format!("发送失败: {e}")));
                    return;
                }
            }
        }
        // 整段确实发出去之后才出队并把额度还给调用线程。
        retire_finished_chunk(&mut outgoing, chunk_done, &ledger);

        // ———— 接收指令 ————
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
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}

/// 一个 UDP 通道。
pub struct UdpChannel {
    cfg: NetConfig,
    state: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
    commands: Option<Sender<WorkerCommand>>,
    /// 待发容量的账本。见 [`worker::PendingLedger`] 的文档。
    ledger: PendingLedger,
    /// 实际绑定到的本地地址（`local_port` 配 0 时由系统分配）。
    /// 是**运行时**信息，随 `close` 清掉。
    bound: Option<SocketAddr>,
}

impl UdpChannel {
    pub fn new() -> Self {
        Self {
            cfg: NetConfig::default(),
            state: Arc::new(AtomicU8::new(IDLE)),
            worker: None,
            commands: None,
            ledger: PendingLedger::default(),
            bound: None,
        }
    }
}

impl Default for UdpChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteChannel for UdpChannel {
    fn open(&mut self, sink: Sender<ChannelEvent>) -> io::Result<()> {
        if self.is_active() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "UDP 通道已经在运行",
            ));
        }

        let remote = self.cfg.remote_addr()?;

        // 绑定 `0.0.0.0`（所有网卡）而不是 `127.0.0.1`：调试目标通常是局域网里的
        // 设备，绑回环会导致既发不出去也收不到。绑定在**所有**接口上也是各类
        // 网络调试工具的默认行为。
        let bind_addr = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            self.cfg.local_port,
        ));
        let socket = UdpSocket::bind(bind_addr).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("绑定本地端口 {} 失败: {e}", self.cfg.local_port),
            )
        })?;

        // 见模块文档：UDP 的 `connect` 不发包、不握手，只固化对端并过滤来源。
        socket.connect(remote)?;

        // **两个方向都设为非阻塞**，与 `net_impl::tcp` 同一条理由（见模块文档
        // 「为什么 UDP 也改成了非阻塞」）：出口饱和时阻塞的 `send` 会让 worker 停在
        // 原地，`Halt` 看不见、`close()` 回不来。而"补一个写超时"那条路走不通——
        // 超时的 `send` 拿不到已写字节数，worker 无法判断这个数据报出没出去。
        //
        // 代价是读侧也不能再靠 `set_read_timeout` 让内核帮着等（同一个 fd 的阻塞
        // 属性是整体的），改为在 [`worker_main`] 里空闲时自己睡 [`IDLE_SLEEP`]。
        socket.set_nonblocking(true)?;

        self.bound = socket.local_addr().ok();

        let state = Arc::clone(&self.state);
        let worker_ledger = self.ledger.clone();
        let (commands, orders) = mpsc::channel::<WorkerCommand>();
        self.commands = Some(commands);

        // 账本归零。通道对象在连接类型不变时会被**就地复用**（重连就是"同一个句柄
        // 再 open 一次"），上一次退出时队列里可能还压着没发出去的数据、额度没还。
        self.ledger.reset();

        state.store(ACTIVE, Ordering::Release);
        self.worker = Some(thread::spawn(move || {
            worker_main(socket, orders, sink, state, worker_ledger);
        }));
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(WorkerCommand::Halt);
        }

        if let Some(handle) = self.worker.take() {
            handle
                .join()
                .map_err(|_| io::Error::other("UDP 后台线程 panic，资源可能未正确释放"))?;
        }

        self.bound = None;
        self.state.store(IDLE, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        // 语义澄清：这里的"活动"只表示**本地 socket 已绑定**。UDP 没有连接，
        // 因此这个值**不**说明对端存在或可达。
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// 实际绑定到的本地地址。`local_port` 配 0 时这里能取到系统分配到的端口——
    /// 界面上要显示它，否则用户不知道设备该往哪个端口回包。
    fn local_addr(&self) -> Option<SocketAddr> {
        self.bound
    }

    /// 交一段字节出去。**队列满会当场返回 `Err`**（判定在调用线程里做，
    /// 靠 [`PendingLedger`] 的额度预约，见 `worker::submit` 的文档）。
    ///
    /// 三个通道的写侧现在一致：判定都发生在**调用线程**上（`worker::submit`），
    /// 后台线程那一侧永远不会因为"写不动"而停住——UDP 这边的 socket 自 2026-10-07
    /// 起也是非阻塞的，理由见本文件顶部「为什么 UDP 也改成了非阻塞」。
    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "UDP 尚未绑定"));
        };
        submit(&self.ledger, commands, bytes, "UDP")
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
    use crate::net_impl::DEFAULT_REMOTE_PORT;
    use std::time::Instant;

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

    /// 起一个本地 UDP socket 当作"对端"，返回它与它的地址。
    fn peer_socket() -> (UdpSocket, u16) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("回环上应能绑定");
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("设置超时应成功");
        let port = socket.local_addr().expect("应能取到地址").port();
        (socket, port)
    }

    fn config_to(port: u16, local_port: u16) -> NetConfig {
        NetConfig {
            remote_host: "127.0.0.1".to_string(),
            remote_port: port,
            local_port,
        }
    }

    #[test]
    fn new_channel_is_idle() {
        assert!(!UdpChannel::new().is_active());
    }

    #[test]
    fn close_is_idempotent_on_unopened_channel() {
        let mut ch = UdpChannel::new();
        ch.close().expect("对从未绑定的通道调用 close 应成功");
        ch.close().expect("重复调用 close 应成功");
        assert!(!ch.is_active());
    }

    #[test]
    fn write_before_open_is_rejected() {
        let mut ch = UdpChannel::new();
        let err = ch.write(vec![0x01]).expect_err("未绑定时写入必须返回错误");
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn configure_rejects_foreign_type() {
        let mut ch = UdpChannel::new();
        assert!(ch.configure(&42u32).is_err());
        assert!(
            ch.configure(&crate::serial_impl::SerialConfig::default())
                .is_err()
        );
    }

    /// **主链路集成测试**：绑定 → 收 → 发，全程回环，不需要硬件。
    ///
    /// 注意测试**只看得见**"数据发出去了、对方收到了"，以及"对方发来的收到了"——
    /// UDP 发出去有没有人收，本地无从得知，这一点不写进断言（写不出来）。
    #[test]
    fn binds_and_exchanges_datagrams() {
        let (peer, peer_port) = peer_socket();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(peer_port, 0))
            .expect("配置应被接受");

        let (tx, rx) = mpsc::channel();
        ch.open(tx).expect("UDP 绑定应立即成功");
        assert!(ch.is_active(), "绑定成功后应处于活动状态");

        let bound = ch.local_addr().expect("应记录实际绑定的本地地址");
        assert_ne!(bound.port(), 0, "local_port 传 0 应由系统分配一个真实端口");

        // 我们 → 对端
        ch.write(b"hello".to_vec()).expect("写入应被接受");
        let mut buf = [0u8; 32];
        let n = peer.recv(&mut buf).expect("对端应能收到数据报");
        assert_eq!(&buf[..n], b"hello");

        // 对端 → 我们
        peer.send_to(b"world", bound).expect("对端应能发回数据报");

        let mut got: Vec<u8> = Vec::new();
        let mut notice: Option<String> = None;
        let received = wait_until(
            || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        ChannelEvent::Bytes(data) => got.extend_from_slice(&data),
                        // UDP 无连接，任何情况下都不该出现 Closed
                        ChannelEvent::Closed => panic!("UDP 不会产生 Closed 事件"),
                        ChannelEvent::Failed(msg) => panic!("不该出现失败事件: {msg}"),
                        // UDP 同样没有 RTS/DTR，非致命通知不该出现。不 panic
                        // （它按设计不是致命事件），改为记下由断言说清楚。
                        ChannelEvent::Notice(msg) => notice = Some(msg),
                    }
                }
                !got.is_empty() || notice.is_some()
            },
            Duration::from_secs(5),
        );
        assert_eq!(notice, None, "UDP 通道不应产生非致命通知（Notice）");
        assert!(received, "对端发来的数据报应送达");
        assert_eq!(got, b"world");

        ch.close().expect("关闭应成功");
        assert!(!ch.is_active());
        assert!(
            ch.local_addr().is_none(),
            "关闭后不该再报告「已绑定到某地址」"
        );
    }

    /// 本地端口被占用时，`open` 必须**当场**报错并带上端口号——
    /// 与 TCP 侧同一条原则：`open` 返回 `Ok` 就是"确实可用"。
    ///
    /// 占位 socket 必须绑在与 `open` **完全相同的地址**（`0.0.0.0`）上：Windows 的
    /// UDP 允许"特定地址"与"通配地址"占用同一端口号共存（先绑 `127.0.0.1:P`、
    /// 再绑 `0.0.0.0:P` 会成功），拿 `127.0.0.1` 去占位永远测不到冲突。
    #[test]
    fn occupied_local_port_is_reported_by_open() {
        let holder = UdpSocket::bind("0.0.0.0:0").expect("通配地址上应能绑定");
        let busy_port = holder.local_addr().expect("应能取到地址").port();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(DEFAULT_REMOTE_PORT, busy_port))
            .expect("配置应被接受");

        let (tx, _rx) = mpsc::channel();
        let err = ch.open(tx).expect_err("端口被占用时必须失败");
        assert!(
            err.to_string().contains(&busy_port.to_string()),
            "错误信息应带上被占用的端口号，实际为: {err}"
        );
        assert!(!ch.is_active());
    }

    /// 目标地址为空/端口为 0 时同样由 `open` 报错（走 `NetConfig::validate`）。
    #[test]
    fn invalid_config_is_rejected_by_open() {
        let mut ch = UdpChannel::new();
        ch.configure(&NetConfig {
            remote_port: 0,
            ..NetConfig::default()
        })
        .expect("配置本身应被接受");
        let (tx, _rx) = mpsc::channel();
        let err = ch.open(tx).expect_err("端口为 0 必须失败");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// ⭐ **背压的判据落在调用线程上**——三个实现共用 `worker::submit`，UDP 这一份
    /// 同样要当场拒绝、并且不能占着额度不放。
    ///
    /// 与 TCP 那份是同一个缺口的两处复现：上限曾经判在后台线程的队列上，`write()`
    /// 一律返回 `Ok`，于是 `app::ComTool::transmit` 照常记账、照常清空输入框。
    #[test]
    fn oversized_payload_is_refused_at_the_call_site() {
        let (_peer, peer_port) = peer_socket();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(peer_port, 0))
            .expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("UDP 绑定应立即成功");

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
    }

    /// ⭐ **`open()` 必须把账本归零**（与串口 / TCP 同一处约定）。
    ///
    /// 通道对象在连接类型不变时会被**就地复用**（重连就是"同一个句柄再 `bind` 一次"），
    /// 而 `close()` 时队列里那些确实没发出去的数据的额度不会被归还。不清的话，用户
    /// 第二次连上以后会"什么都没发就报队列满"。做法是直接把账本压满来模拟上次的残留。
    #[test]
    fn reopening_the_channel_clears_the_stale_quota() {
        let (_peer, peer_port) = peer_socket();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(peer_port, 0))
            .expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("UDP 绑定应立即成功");
        ch.close().expect("关闭应成功");

        ch.ledger
            .reserve(MAX_PENDING_BYTES)
            .expect("额度充足时应能占满");
        assert!(ch.ledger.reserve(1).is_err(), "占满之后就该拒绝了");

        let (tx2, _rx2) = mpsc::channel();
        ch.open(tx2).expect("重新绑定应成功");
        assert_eq!(
            ch.ledger.bytes(),
            0,
            "open 必须把上一次留下的额度清掉，否则重连后的容量凭空少一块"
        );

        ch.close().expect("关闭应成功");
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
    /// ⭐⭐ **空闲时的发送吞吐不该被 `recv` 的等待时间卡住**（2026-10-07 改非阻塞的
    /// 判据，也是这条改动**唯一能被自动验证**的一面）。
    ///
    /// 判据为什么挑在这里：改之前 socket 是阻塞的、读侧靠 `set_read_timeout(10 ms)`
    /// 让内核帮着等，而 worker 一轮循环里 `recv` **排在 `send` 前面**——对端什么都不
    /// 发时，每一轮都要在 `recv` 里白白待满 10 ms 才轮到发一个数据报，于是发送吞吐
    /// 被钉死在约 **100 个/秒**。改成非阻塞后 `recv` 立刻返回 `WouldBlock`，同样的
    /// 场景下一轮能走完整个循环，吞吐只受 syscall 速度限制。
    ///
    /// 所以这条测的是**能区分两版实现的量**：2000 个数据报在旧实现下要 20 秒，
    /// 新实现下应当远快于 3 秒。它证明不了"出口饱和时不阻塞"（那需要先把真实网卡
    /// 打满，门禁里造不出来），但它能证明"非阻塞确实生效了、且空闲时没有退化成
    /// 每轮等 10 ms"。
    ///
    /// 判据用"对端收到全部 2000 个"而不是"我们写了多少"：`write()` 只是把数据排进
    /// 队列（`submit` 当场返回），真正发出去没有只有对端能作证。
    #[test]
    fn idle_socket_does_not_throttle_send_throughput() {
        const DATAGRAMS: usize = 2_000;
        const DEADLINE: Duration = Duration::from_secs(3);

        let peer = UdpSocket::bind("127.0.0.1:0").expect("回环上应能绑定");
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .expect("设置超时应成功");
        let peer_port = peer.local_addr().expect("应能取到地址").port();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(peer_port, 0))
            .expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("UDP 绑定应立即成功");

        // 对端**只读不写**：这正是"worker 的 recv 每轮都要空等"的那个场景。
        let counter = thread::spawn(move || {
            let mut buf = [0u8; 16];
            let mut seen = 0usize;
            let deadline = Instant::now() + DEADLINE;
            while seen < DATAGRAMS && Instant::now() < deadline {
                // 超时（对端还没发完）与 WouldBlock 都不算失败，继续等。
                if peer.recv(&mut buf).is_ok() {
                    seen += 1;
                }
            }
            seen
        });

        for i in 0..DATAGRAMS {
            ch.write(format!("{i:08}").into_bytes())
                .expect("16 KB 的负载远在待发上限之下，不该被拒绝");
        }

        let seen = counter.join().expect("对端线程不应 panic");
        ch.close().expect("关闭应成功");
        assert_eq!(
            seen, DATAGRAMS,
            "{DEADLINE:?} 内对端只收到 {seen}/{DATAGRAMS} 个数据报——发送吞吐被每轮的\
             读等待卡住了（阻塞读 10 ms 时这里只有约 100 个/秒）"
        );
    }

    /// **队列压满时 `close()` 必须立刻返回**（与 TCP 那份同一条判据）。
    ///
    /// ⚠️ 诚实边界：这条是**防回归**，不是"出口饱和时不阻塞"的证据——门禁里造不出
    /// 会堵住的 UDP 出口。它保证的是无论写侧怎么实现，点「断开」都不会把 UI 卡住。
    #[test]
    fn close_returns_promptly_with_a_backlog_queued() {
        let (_peer, peer_port) = peer_socket();

        let mut ch = UdpChannel::new();
        ch.configure(&config_to(peer_port, 0))
            .expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("UDP 绑定应立即成功");

        // 排 512 个 1 KiB 数据报（合计 512 KiB，在 1 MiB 待发上限之内，因此不会被拒）。
        // 单个数据报刻意不超过 65507 字节——那是 UDP 的报文上限，超了会得到
        // `MessageTooLong`，worker 会当真故障上报并退出，测的就不是积压场景了。
        for i in 0..512 {
            ch.write(vec![i as u8; 1024])
                .expect("512 KiB 远在上限之下，写入不该被拒绝");
        }

        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = ch.close();
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(3)).is_ok(),
            "手里压着待发数据时 close() 也必须立刻返回——卡住就等于点「断开」没反应"
        );
    }

    #[test]
    fn pending_bytes_reports_what_the_ledger_holds() {
        let ch = UdpChannel::new();
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
