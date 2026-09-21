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

use crate::communicationtrait::worker::{ACTIVE, IDLE, StateGuard, WorkerCommand, is_transient};
use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::net_impl::config::NetConfig;

/// 单次 `recv` 的最长等待。与其它两个通道同值，理由相同（空闲唤醒频率 +
/// 停机响应延迟）。
const READ_TIMEOUT: Duration = Duration::from_millis(10);

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
) {
    let _guard = StateGuard(Arc::clone(&state));

    let mut buf = vec![0u8; DATAGRAM_MAX];
    // 与其它两个通道同样的待发队列（理由见 `serial_impl::channel`）。
    // UDP 的 `send` 基本不会部分写，但保留同一个结构让三个实现可以逐段对照。
    let mut outgoing: VecDeque<(Vec<u8>, usize)> = VecDeque::new();

    loop {
        // ———— 方向一：对端 → UI ————
        match socket.recv(&mut buf) {
            // **不是 EOF**：收到一个零长度数据报。忽略即可，绝不能当成关闭。
            Ok(0) => {}
            Ok(n) => {
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
        let mut chunk_done = false;
        if let Some((bytes, sent)) = outgoing.front_mut() {
            match socket.send(&bytes[*sent..]) {
                Ok(0) => {}
                Ok(n) => {
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
        if chunk_done {
            outgoing.pop_front();
        }

        // ———— 接收指令 ————
        loop {
            match commands.try_recv() {
                Ok(WorkerCommand::Transmit(bytes)) => outgoing.push_back((bytes, 0)),
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
        socket.set_read_timeout(Some(READ_TIMEOUT))?;

        self.bound = socket.local_addr().ok();

        let state = Arc::clone(&self.state);
        let (commands, orders) = mpsc::channel::<WorkerCommand>();
        self.commands = Some(commands);

        state.store(ACTIVE, Ordering::Release);
        self.worker = Some(thread::spawn(move || {
            worker_main(socket, orders, sink, state);
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

    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "UDP 尚未绑定"));
        };
        commands
            .send(WorkerCommand::Transmit(bytes))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "UDP 后台线程已退出"))
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
        let received = wait_until(
            || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        ChannelEvent::Bytes(data) => got.extend_from_slice(&data),
                        // UDP 无连接，任何情况下都不该出现 Closed
                        ChannelEvent::Closed => panic!("UDP 不会产生 Closed 事件"),
                        ChannelEvent::Failed(msg) => panic!("不该出现失败事件: {msg}"),
                    }
                }
                !got.is_empty()
            },
            Duration::from_secs(5),
        );
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
}
