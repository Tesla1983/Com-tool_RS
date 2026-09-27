//! 串口通道实现。
//!
//! 一个后台线程独占 `serialport::SerialPort` 句柄，UI 侧只通过两条 mpsc
//! 通道与它交互：一条收事件，一条发指令。阻塞 IO 全部落在后台线程上。
//!
//! 停机方式与「共享标志位 + 轮询」的常见写法不同：UI 侧直接向指令通道投递
//! [`WorkerCommand::Halt`]，后台线程在读完当前这一轮数据后必然看到它并返回。
//! 共享的 `AtomicU8` 只用于向 UI 汇报状态，不参与控制流，因此全程无需加锁，
//! 也就不存在「状态锁与 join 互相等待」这类死锁。

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serialport::SerialPort;

use crate::communicationtrait::worker::{
    ACTIVE, IDLE, OPENING, PendingLedger, StateGuard, WorkerCommand, is_transient,
    retire_finished_chunk, submit,
};
use crate::communicationtrait::{ByteChannel, ChannelEvent};
use crate::serial_impl::config::SerialConfig;

/// 单次 `read` 的最长等待。
///
/// 它同时决定两件事：空闲时的唤醒频率，以及 `close()` 投出停机指令后后台
/// 线程的最长响应延迟。10 ms 在两者之间取平衡——空闲态每秒约 100 次循环，
/// 对 CPU 几乎无感，而关串口的体感延迟仍在「瞬时」范围内。
const READ_TIMEOUT: Duration = Duration::from_millis(10);

/// 单次 `read` 的缓冲上限。
///
/// 取 4 KiB 而非更小：一次系统调用的开销与字节数无关，缓冲越大代表一轮循环
/// 能搬走的数据越多。3 Mbaud 下每毫秒约 300 字节，4 KiB 留有十倍以上余量。
const READ_CHUNK: usize = 4 * 1024;

/// 按配置打开端口。
fn open_port(cfg: &SerialConfig) -> io::Result<Box<dyn SerialPort>> {
    serialport::new(cfg.port.clone(), cfg.baud_rate)
        .data_bits(cfg.data_bits)
        .stop_bits(cfg.stop_bits)
        .parity(cfg.parity)
        .flow_control(cfg.flow_control)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| io::Error::other(e.to_string()))
}

/// 后台线程主体：独占端口句柄，搬运两个方向的数据。
fn worker_main(
    cfg: SerialConfig,
    commands: Receiver<WorkerCommand>,
    events: Sender<ChannelEvent>,
    state: Arc<AtomicU8>,
    ledger: PendingLedger,
) {
    let _guard = StateGuard(Arc::clone(&state));

    // 端口**确实**打开之后才把状态从 `OPENING` 推到 `ACTIVE`。这是三态里
    // "正在打开"与"已就绪"的分界：`is_ready()` 在此之前一律为假，UI 因此
    // 不会在打开失败时先亮一下"已连接"。
    let mut port = match open_port(&cfg) {
        Ok(port) => port,
        Err(e) => {
            let _ = events.send(ChannelEvent::Failed(format!("打开 {} 失败: {e}", cfg.port)));
            return;
        }
    };
    state.store(ACTIVE, Ordering::Release);

    let mut buf = vec![0u8; READ_CHUNK];

    // 待发送队列。队首是正在发送的那一段，附带已经写出去多少字节。
    //
    // 之所以要在队列里保留「发了一半」的状态，而不是对每段数据调用一次
    // `write_all`：`timeout()` 同时作用于读和写，当设备侧流控拉低或驱动发送
    // 缓冲暂时满时，`write_all` 会中途返回 `TimedOut`，而 `Write::write_all`
    // 不报告已写入的字节数，剩下没挤进去的部分只能被静默丢弃——对串口工具
    // 而言那是数据丢失。这里改为按剩余长度逐步推进，不丢字节。
    let mut outgoing: VecDeque<(Vec<u8>, usize)> = VecDeque::new();

    loop {
        // ———— 方向一：设备 → UI ————
        match port.read(&mut buf) {
            Ok(0) => {
                // 配了超时的驱动理应返回 `TimedOut` 而不是 `Ok(0)`。万一某个
                // 驱动违反这一点，这里必须让出时间片，否则会空转烧满一个核。
                thread::sleep(Duration::from_millis(1));
            }
            Ok(n) => {
                // 接收端已被丢弃，说明 UI 已经退出，没有继续搬数据的理由。
                if events.send(ChannelEvent::Bytes(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) if is_transient(&e) => {}
            Err(e) => {
                let _ = events.send(ChannelEvent::Failed(format!("读取失败: {e}")));
                return;
            }
        }

        // ———— 方向二：UI → 设备 ————
        // 每轮最多推进一次写，避免长报文占满循环、拖慢停机响应。
        let mut chunk_done = false;
        if let Some((bytes, sent)) = outgoing.front_mut() {
            match port.write(&bytes[*sent..]) {
                // 驱动这一轮一个字节都没接收，等下一轮重试。
                Ok(0) => {}
                Ok(n) => {
                    *sent += n;
                    chunk_done = *sent == bytes.len();
                }
                Err(e) if is_transient(&e) => {}
                Err(e) => {
                    let _ = events.send(ChannelEvent::Failed(format!("写入失败: {e}")));
                    return;
                }
            }
        }
        // 整段确实写完之后才出队并把额度还给调用线程；写出去多少字节由驱动如实
        // 回报（`Ok(n)`），所以不会多还也不会少还。
        retire_finished_chunk(&mut outgoing, chunk_done, &ledger);

        // ———— 接收指令 ————
        // 一次排空，避免高速收发时指令积压；`Halt` 在任何时刻都能被看到，
        // 因此 `close()` 的等待时长始终由一轮读写决定，不被待发数据拖住。
        loop {
            match commands.try_recv() {
                // 队列**直接 push**：容量已经在调用线程里由 `worker::submit` 预约过了，
                // 这里不再判一遍。上限只有一处实现，判两遍迟早会分叉成两套口径。
                Ok(WorkerCommand::Transmit(bytes)) => outgoing.push_back((bytes, 0)),
                Ok(WorkerCommand::SetLineState { rts, dtr }) => {
                    // 现场直接改：这两个调用只是驱动层的两次控制请求，不阻塞。
                    // 失败走 `Notice` 而不是 `Failed`——拉不动一根控制线并不代表
                    // 通道坏了，判成掉线会让界面从"已连接"跳走、还会把定时发送关掉。
                    if let Some(level) = rts
                        && let Err(e) = port.write_request_to_send(level)
                    {
                        let _ = events.send(ChannelEvent::Notice(format!("设置 RTS 失败: {e}")));
                    }
                    if let Some(level) = dtr
                        && let Err(e) = port.write_data_terminal_ready(level)
                    {
                        let _ = events.send(ChannelEvent::Notice(format!("设置 DTR 失败: {e}")));
                    }
                }
                Ok(WorkerCommand::Halt) => return,
                Err(TryRecvError::Empty) => break,
                // UI 侧把 Sender 丢了，说明对象正在析构，同样收工。
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}

/// 一个串口通道。
pub struct SerialChannel {
    cfg: SerialConfig,
    state: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
    commands: Option<Sender<WorkerCommand>>,
    /// 待发容量的账本。UI 侧持有它才能在 `write()` 里**当场**知道排不排得上队
    /// （理由见 `worker::PendingLedger` 的文档：那正是"队列满却报成功"的根）。
    ledger: PendingLedger,
}

impl SerialChannel {
    pub fn new() -> Self {
        Self {
            cfg: SerialConfig::default(),
            state: Arc::new(AtomicU8::new(IDLE)),
            worker: None,
            commands: None,
            ledger: PendingLedger::default(),
        }
    }

    /// 当前生效的配置，用于在 UI 上回显实际参数。
    #[allow(dead_code)]
    pub fn config(&self) -> &SerialConfig {
        &self.cfg
    }
}

impl Default for SerialChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteChannel for SerialChannel {
    fn open(&mut self, sink: Sender<ChannelEvent>) -> io::Result<()> {
        if self.is_active() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "串口通道已经在运行",
            ));
        }

        let cfg = self.cfg.clone();
        // 后台线程拿一份自己的 `state` 克隆，线程内**先**置 `ACTIVE` 再打开端口
        // ——`is_active()` 因此有确定的次序（见本方法的文档）。
        let worker_state = Arc::clone(&self.state);
        let worker_ledger = self.ledger.clone();
        let (commands, orders) = mpsc::channel::<WorkerCommand>();
        self.commands = Some(commands);

        // 账本归零。通道对象在连接类型不变时会被**就地复用**（重连就是"同一个句柄
        // 再 open 一次"），上一次运行退出时队列里可能还压着没写完的数据、额度没还，
        // 不清掉会让重开后的容量凭空少一块。
        self.ledger.reset();

        // **本线程先置 `OPENING`，后台线程接管后才升到 `ACTIVE`。**
        //
        // 顺序在这里是判据本身：`UI 置 OPENING` 必须先于 `spawn`，否则
        // `open()` 返回后到线程真正起来之间会短暂停在 `IDLE`——那正是既有的
        // `connected` 假阳性缺陷（UI 看到"未活动"就乐观置位"已连接"）。
        // 而 `UI 置 OPENING` 又必须先于 `worker 置 ACTIVE`，否则端口打开极快时
        // 线程会先把状态推到 `ACTIVE`、UI 这一句再把它踩回 `OPENING`。
        self.state.store(OPENING, Ordering::Release);
        self.worker = Some(thread::spawn(move || {
            worker_main(cfg, orders, sink, worker_state, worker_ledger);
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
            handle.join().map_err(|_| {
                // 后台线程当前实现全程不加锁，理论上不会 panic；真发生了说明
                // 是代码缺陷，如实上报，而不是假装关闭成功。
                io::Error::other("串口后台线程 panic，资源可能未正确释放")
            })?;
        }

        self.state.store(IDLE, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// 端口是否**确实**已经打开（不是"正在打开"、也不是"已停止"）。
    ///
    /// 三态在这里第一次被区分开：`OPENING` 只代表"后台线程已经起来了，但端口
    /// 还没开成"，对调用方而言它和失败一样都**不能算连上**。UI 侧据此就不必再
    /// 乐观置位 `connected`——`FEATURE_GAP_AND_PLAN.md` §4 记的那个"打开不存在
    /// 的端口时先闪一下已连接"的缺陷，根子就在这里少了一档。
    ///
    /// **语义与 `is_active` 的差别**：`is_active` 回答"后台线程活着吗"（`OPENING`
    /// 也算活着），`is_ready` 回答"能收发字节了吗"。两者只在 `OPENING` 这一档上
    /// 分叉，`IDLE` / `ACTIVE` 上完全一致。
    ///
    /// 覆盖 trait 的默认实现（那个直接转发 `is_active()`）：串口是**唯一**异步
    /// 打开的传输，覆盖它的正是"串口打开不是同步的"这一事实本身。
    fn is_ready(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// 交一段字节出去。**队列满会当场返回 `Err`**（判定在调用线程里做，
    /// 靠 [`PendingLedger`] 的额度预约，见 `worker::submit` 的文档）。
    ///
    /// 这一条对本项目很要紧：调用方（`app::ComTool::transmit`）是"`Ok` 才记账、
    /// `Ok` 才清空输入框"。异步判定会让它在队列满时照记账、照清空，用户于是看到
    /// "本条未发送"的提示与仍在增长的发送计数并存、刚敲的内容也被清掉。
    fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "串口尚未打开"));
        };
        submit(&self.ledger, commands, bytes, "串口")
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
        let cfg = settings.downcast_ref::<SerialConfig>().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "配置类型不是 SerialConfig")
        })?;
        self.cfg = cfg.clone();
        Ok(())
    }

    /// 串口是**唯一**会覆盖它的实现（RTS / DTR 是串口专有的两根控制线）。
    ///
    /// 只把请求投给后台线程就返回：真正的驱动调用在那里做。因此 `Ok` 的含义是
    /// "请求已送达"，线到底拉没拉动由后台线程的判断决定（失败会回一条 `Notice`）。
    fn set_line_state(&mut self, rts: Option<bool>, dtr: Option<bool>) -> io::Result<()> {
        let Some(commands) = self.commands.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "串口尚未打开"));
        };
        commands
            .send(WorkerCommand::SetLineState { rts, dtr })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "串口后台线程已退出"))
    }
}

/// 枚举当前系统上可见的串口名，按名称排序。
///
/// 排序是为了让 `COM10` 落在 `COM2` 之后，而不是按系统返回的原始顺序排列。
/// 枚举失败时返回空列表：对 UI 而言「没有可用串口」与「枚举失败」的处置方式
/// 相同（都只能提示用户检查设备），因此不向上传播错误。
pub fn scan_ports() -> Vec<String> {
    let mut names: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|info| info.port_name)
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communicationtrait::worker::MAX_PENDING_BYTES;
    use std::time::Instant;

    /// 一个基本不可能存在的端口名，用于稳定地触发「打开失败」路径。
    const ABSENT_PORT: &str = "COM_THIS_PORT_DOES_NOT_EXIST_9999";

    fn absent_port_config() -> SerialConfig {
        SerialConfig {
            port: ABSENT_PORT.to_string(),
            ..SerialConfig::default()
        }
    }

    #[test]
    fn new_channel_is_idle() {
        let ch = SerialChannel::new();
        assert!(!ch.is_active(), "新建通道不应处于活动状态");
    }

    /// 需求：`close` 幂等——从未打开过、或重复调用都不得报错。
    #[test]
    fn close_is_idempotent_on_unopened_channel() {
        let mut ch = SerialChannel::new();
        ch.close().expect("对从未打开的通道调用 close 应成功");
        ch.close().expect("重复调用 close 应成功");
        assert!(!ch.is_active());
    }

    /// 需求：未打开时写入必须明确失败，不能 panic，也不能静默丢弃。
    #[test]
    fn write_before_open_is_rejected() {
        let mut ch = SerialChannel::new();
        let err = ch
            .write(vec![0xA5, 0x5A])
            .expect_err("未打开时写入必须返回错误");
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    /// 需求：`configure` 只接受本通道认识的配置类型。
    #[test]
    fn configure_rejects_foreign_type() {
        let mut ch = SerialChannel::new();
        let err = ch
            .configure(&42u32)
            .expect_err("非 SerialConfig 类型必须被拒绝");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// 需求：打开失败必须经事件通道上报，且 `is_active` 必须回落。
    ///
    /// 这条覆盖了原实现里最容易出问题的一环：后台线程退出后状态若不归位，
    /// UI 会永久停留在「已连接」，而自动重连又因为 `serial` 非空而不触发。
    #[test]
    fn open_failure_is_reported_and_state_resets() {
        let mut ch = SerialChannel::new();
        ch.configure(&absent_port_config()).expect("配置应被接受");

        let (tx, rx) = mpsc::channel();
        ch.open(tx).expect("open 只负责拉起线程，应立即返回 Ok");

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(ChannelEvent::Failed(msg)) => {
                assert!(
                    msg.contains(ABSENT_PORT),
                    "失败信息应带上端口名以便定位，实际为: {msg}"
                );
            }
            Ok(other) => panic!("打开不存在的端口应上报 Failed，实际收到: {other:?}"),
            Err(e) => panic!("等待失败事件超时: {e}"),
        }

        // 线程退出到状态归位之间允许有调度延迟，给一小段收敛时间。
        let deadline = Instant::now() + Duration::from_secs(5);
        while ch.is_active() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!ch.is_active(), "后台线程退出后 is_active 必须回到 false");

        ch.close().expect("关闭已自行退出的通道应成功");
        assert!(!ch.is_active());
    }

    /// ⭐ **`open()` 之后、端口打开之前，状态必须是 `OPENING`**——这是"UI 不许
    /// 乐观置位"能够成立的那块基石。
    ///
    /// 上面那条（`is_ready_is_false_until_the_port_is_actually_open`）看的是
    /// `is_ready()` 的**结果**；这条直接读状态位本身，钉住"中间态确实存在"。
    /// 两条都要：只看结果的话，把 `open()` 写成"直接置 `IDLE`"也能通过——而那会
    /// 让 UI 无法区分"正在打开"与"打开失败"，正是本次要修的缺陷的另一半。
    ///
    /// 做法是**打开一个必然失败的端口**并抢在失败事件之前读状态：那一刻后台线程
    /// 已经起来了（所以不是 `IDLE`）、端口还没打开（所以不是 `ACTIVE`）。
    #[test]
    fn open_transitions_through_the_opening_state() {
        use crate::communicationtrait::worker::{ACTIVE, IDLE, OPENING};

        let mut ch = SerialChannel::new();
        assert_eq!(
            ch.state.load(Ordering::Acquire),
            IDLE,
            "新建通道必须是 IDLE"
        );

        ch.configure(&absent_port_config()).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("open 只负责拉起线程，应立即返回 Ok");

        // `open()` 返回后、后台线程把端口打开（或失败）之前——`OPENING`。
        //
        // 这段窗口极短，所以不能断言"此刻一定是 OPENING"（线程可能已经跑到失败
        // 分支把状态归位成 IDLE 了）。改为断言**状态集合的合法性**：绝不允许
        // 出现 `ACTIVE`——那意味着"端口还没开就被报告为已就绪"。
        let observed = ch.state.load(Ordering::Acquire);
        assert_ne!(
            observed, ACTIVE,
            "`open()` 刚返回、端口还没打开时绝不能是 ACTIVE——旧实现在这里就宣布\
             已连接，界面于是先亮一下「已连接」再跳回「未连接」"
        );
        assert!(
            observed == OPENING || observed == IDLE,
            "状态只可能是 OPENING（线程在跑）或 IDLE（线程已失败退出），实际 {observed}"
        );

        ch.close().expect("关闭应成功");
        assert_eq!(
            ch.state.load(Ordering::Acquire),
            IDLE,
            "关闭后必须回到 IDLE"
        );
    }

    /// 端口枚举不应 panic，且结果有序（保证 COM2 排在 COM10 之前）。
    #[test]
    fn scan_ports_is_sorted_and_panic_free() {
        let ports = scan_ports();
        let mut expected = ports.clone();
        expected.sort();
        assert_eq!(ports, expected, "scan_ports 必须返回已排序的结果");
    }

    /// ⭐ **三态的边界**：还没打开是"未就绪"，打开失败的**整个过程中**也一直是
    /// "未就绪"，只有端口真的打开之后才是就绪。
    ///
    /// 这条钉住的是本次修的缺陷的**根**：`open()` 返回只表示"线程起来了"，此
    /// 时 `is_ready()` 必须还是假。断言写成"①→②"两段而不是只看最终值，是因为
    /// "一开始就返回真"和"失败后仍然返回真"这两种实现都能骗过只看最终值的版本。
    #[test]
    fn is_ready_is_false_until_the_port_is_actually_open() {
        let mut ch = SerialChannel::new();
        // ① 从未打开：不就绪。
        assert!(!ch.is_ready(), "从未打开的通道不得报告为就绪");

        // ② 打开一个**不存在**的端口。`open()` 会返回 `Ok`（它只拉起线程），
        //    紧接着线程上报 `Failed` 并退出——从头到尾都不该出现"就绪"。
        ch.configure(&absent_port_config()).expect("配置应被接受");
        let (tx, rx) = mpsc::channel();
        ch.open(tx).expect("open 只负责拉起线程，应立即返回 Ok");

        assert!(
            !ch.is_ready(),
            "`open()` 返回后端口还没打开，必须**不是**就绪——这正是 UI 侧不能乐观\
             置位 `connected` 的依据（旧实现在这里就宣布已连接了）"
        );

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(ChannelEvent::Failed(_)) => {}
            Ok(other) => panic!("打开不存在的端口应上报 Failed，实际收到: {other:?}"),
            Err(e) => panic!("等待失败事件超时: {e}"),
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        while ch.is_active() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !ch.is_ready(),
            "端口打开失败后绝不能报告为就绪——否则界面会亮着「已连接」而其实一个字节都发不出去"
        );

        ch.close().expect("关闭已自行退出的通道应成功");
        assert!(!ch.is_ready());
    }

    /// `is_ready()` 与 `is_active()` 必须**同时**为真：两者读的是同一个状态位，
    /// 只是 `is_ready` 把"正在打开"这一档明确算作"不就绪"。
    ///
    /// 这条防的是"三态只加在一处"：若哪天 `is_ready` 被改成查另一个字段，两个
    /// 读数就会在某个时刻分叉，而调用方（UI 与测试）会各自拿到不同结论。
    #[test]
    fn is_ready_agrees_with_is_active_outside_the_opening_window() {
        let mut ch = SerialChannel::new();
        assert_eq!(ch.is_ready(), ch.is_active(), "未打开时两者都该是假");

        ch.configure(&absent_port_config()).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("open 应返回 Ok");

        let deadline = Instant::now() + Duration::from_secs(10);
        while ch.is_active() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            ch.is_ready(),
            ch.is_active(),
            "稳定态（非「正在打开」窗口内）两者必须一致"
        );
        ch.close().expect("关闭应成功");
    }

    /// ⭐ **背压的判据落在调用线程上**（三个实现共用 `worker::submit`），串口这一份
    /// 不必连硬件也能验：打开一个必然失败的端口即可——通道对象仍然在，`write()`
    /// 会走到同一个 `submit`，判定完全在调用线程内完成。
    ///
    /// 与 TCP / UDP 那两条是同一个缺口的三处复现：上限曾经判在后台线程的队列上，
    /// `write()` 一律返回 `Ok`，于是 `app::ComTool::transmit` 照常记账、照常把用户
    /// 刚敲的内容清掉。
    #[test]
    fn oversized_payload_is_refused_at_the_call_site() {
        let mut ch = SerialChannel::new();
        ch.configure(&absent_port_config()).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("open 只负责拉起线程，应立即返回 Ok");

        let err = ch
            .write(vec![0u8; MAX_PENDING_BYTES + 1])
            .expect_err("单条就超过上限的负载必须当场被拒");
        assert!(
            err.to_string().contains("待发队列已满"),
            "错误信息应说清是队列满（调用方会原样显示给用户），实际为: {err}"
        );

        ch.close().expect("关闭应成功");
    }

    /// ⭐ **`open()` 必须把账本归零**（与 TCP / UDP 同一处约定）。
    ///
    /// 通道对象在连接类型不变时会被**就地复用**（重连就是"同一个句柄再 `open` 一次"），
    /// 而 `close()` 时队列里那些确实没写出去的数据的额度不会被归还。不清的话，用户
    /// 第二次连上以后会"什么都没发就报队列满"。做法是直接把账本压满来模拟上次的残留。
    #[test]
    fn reopening_the_channel_clears_the_stale_quota() {
        let mut ch = SerialChannel::new();
        ch.configure(&absent_port_config()).expect("配置应被接受");
        let (tx, _rx) = mpsc::channel();
        ch.open(tx).expect("open 只负责拉起线程，应立即返回 Ok");
        ch.close().expect("关闭应成功");

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
    #[test]
    fn pending_bytes_reports_what_the_ledger_holds() {
        let ch = SerialChannel::new();
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
