//! 实时文件日志：把接收区的内容同步落到磁盘文件。
//!
//! 三条设计线（第一条与 [`crate::serial_impl::channel`] 同一套约定）：
//!
//! 1. **写盘在后台线程**。UI 线程只把待写文本投进一条 mpsc 通道，文件句柄由后台
//!    线程独占。磁盘会变慢（网络盘、被其它进程抢占 IO），把 `write` 留在 UI 线程
//!    等于让界面跟着磁盘一起卡。停机同样靠投递 [`LogCommand::Halt`] 而不是共享
//!    标志位轮询；[`LogSink`] 的 `Drop` 会投 `Halt` 再 `join`，保证缓冲区落盘后
//!    才算停。
//! 2. **文件由 UI 线程打开、再交给后台线程**。「开始记录」必须能当场回答成功还是
//!    失败（目录不存在、无权限），而打开文件只是一次本地系统调用。若改成让后台
//!    线程自己打开，界面就只能先乐观地显示「记录中」、失败后再回滚——那正是
//!    `open_port` 现存的问题（`connected` 假阳性），不该复制第二份。
//! 3. **不覆盖已有文件**。用 `create_new` 而不是 `create`，同名时另起一个名字
//!    （见 [`create_log_file`]）。默认模板的精度只到秒，`create` 会静默截断上
//!    一份日志——那是数据丢失。
//!
//! 记录范围：**接收区里看到什么就记什么**——数据、HEX / 字节查看模式下的转义
//! 形式、以及「已打开串口」这类一次性提示都在内；不含发送内容。日志文件**不受**
//! [`ComTool::max_log_chars`] 约束，那个上限只裁内存里的镜像。

use chrono::Local;
use chrono::format::{Item, StrftimeItems};
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use crate::APP_TITLE;
use crate::app::{ComTool, DEFAULT_LOG_FILE_NAME};

/// 写缓冲大小。
///
/// 64 KiB：高速收数据时把「每条数据一次系统调用」合并成「每 64 KiB 一次」，而
/// 低速时文件依然实时可读——队列一排空就会 flush（见 [`worker_main`]）。
const WRITE_BUFFER: usize = 64 * 1024;

/// 同名文件的重试上限。取有限值是必要的：模板可以被用户改成与时间无关的固定
/// 名字，若不设上限，`_1`、`_2`… 会一直找下去。
const MAX_NAME_ATTEMPTS: u32 = 1_000;

/// 待写队列的最大深度（消息条数）。
///
/// 磁盘（尤其是网络盘）跟不上数据速率时，队列会一直涨，而串口工具挂机跑一整天是
/// 常见用法——无上限的队列等于把内存交给磁盘状态决定。这里给一个有限深度，满了
/// 就停下来如实报告（见 [`ComTool::log_append`]），而不是继续吃内存。
///
/// 按单条消息不超过 [`crate::serial_impl::channel`] 的读缓冲（4 KiB，字节查看
/// 模式下转义后最多约 4 倍）估算，队列占用在几 MiB 量级。
const QUEUE_DEPTH: usize = 256;

/// 文件名中会被替换掉的字符（路径分隔符 + Windows 禁止出现在文件名里的字符）。
const UNSAFE_NAME_CHARS: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// UI 侧投给写日志线程的指令。
enum LogCommand {
    /// 追加一段文本。
    Append(String),
    /// 冲刷缓冲区并结束线程。
    Halt,
}

/// 写日志线程向 UI 上报的事件。
#[derive(Debug)]
pub enum LogEvent {
    /// 写盘出错，线程已退出。
    ///
    /// UI 收到后必须关闭「记录中」状态并提示用户——继续显示「记录中」就是在
    /// 骗用户，而用户会以为日志还在长。
    Failed(String),
}

/// 默认日志输出目录：`eframe` 存档目录下的 `logs`。
///
/// 选它而不是进程当前目录：GUI 进程的工作目录取决于快捷方式，用户基本找不到；
/// 存档目录则是平台保证可写的位置，且与「信息」窗口显示的那条路径同源。真的取不到
/// （平台不支持）时退回当前目录——总比没有位置可写要好。
pub fn default_log_dir() -> PathBuf {
    eframe::storage_dir(APP_TITLE)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("logs")
}

/// 把渲染结果收紧成一个**纯文件名**；清理后没有可用内容时返回 `None`。
///
/// 规则有三条。前两条是安全要求：路径分隔符与 Windows 非法字符一律替换为 `_`
/// （模板是用户输入的文本，`../` 之类的写法会把日志写到选定目录之外），并去掉
/// 首尾空白与点号（`..` 这种名字在 Windows 上还会被当成上上级目录）。第三条是
/// 可用性要求：结果里必须至少有一个字母或数字——模板写成 `///` 时清理后只剩
/// 分隔符，等于没写，交给调用方退回默认模板。
fn sanitize_file_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if UNSAFE_NAME_CHARS.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    cleaned
        .chars()
        .any(char::is_alphanumeric)
        .then_some(cleaned)
}

/// 展开文件名模板中的时间占位符，返回一个可用的文件名。
///
/// 模板来自用户输入（也可能来自被手改的存档）。`DateTime::format` 在渲染到非法
/// 占位符时会 **panic**，所以先用 [`StrftimeItems`] 逐项检查、有问题就整条退回
/// 默认模板——不能把用户的输入变成崩溃。
pub fn render_log_file_name(template: &str, now: chrono::DateTime<Local>) -> String {
    // 默认模板是内置常量，一定合法；作为一切异常情况的兜底。
    let fallback = || now.format(DEFAULT_LOG_FILE_NAME).to_string();

    let template_ok = !template.is_empty()
        && !StrftimeItems::new(template).any(|item| matches!(item, Item::Error));
    if !template_ok {
        return fallback();
    }
    sanitize_file_name(&now.format(template).to_string()).unwrap_or_else(fallback)
}

/// 把文件名切成「主名 + 后缀」，用于同名时追加序号。
fn split_name(name: &str) -> (String, Option<String>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), Some(ext.to_string()))
        }
        _ => (name.to_string(), None),
    }
}

/// 在 `dir` 下创建名为 `name` 的日志文件；同名文件已存在时改用 `name_1`、`name_2`…
///
/// 用 `create_new` 而不是 `File::create`：默认模板的精度只到秒，同一秒内再次开始
/// 记录、或用户把模板改成不含时间的固定名，都会命中已存在的文件，而 `File::create`
/// 会**静默截断**它。宁可换名新建，也不能悄悄销毁上一份日志。
///
/// 返回实际使用的路径与已打开的文件句柄——句柄随后交给写盘线程独占。
pub fn create_log_file(dir: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    let (stem, ext) = split_name(name);

    for attempt in 0..MAX_NAME_ATTEMPTS {
        let candidate = match (attempt, &ext) {
            (0, _) => name.to_string(),
            (n, Some(ext)) => format!("{stem}_{n}.{ext}"),
            (n, None) => format!("{stem}_{n}"),
        };
        let path = dir.join(&candidate);

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("同名日志文件过多（超过 {MAX_NAME_ATTEMPTS} 个）: {name}"),
    ))
}

/// 写日志线程主体：独占文件句柄，按需追加与冲刷。
fn worker_main(file: File, commands: Receiver<LogCommand>, events: Sender<LogEvent>) {
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER, file);

    'outer: while let Ok(mut cmd) = commands.recv() {
        // 上面的 `recv()` 是阻塞的：空闲时完全不占 CPU，而 `Halt` 一到就能立刻
        // 唤醒——这正是用通道而不是轮询标志位的好处。收到 `Err` 说明 UI 侧把
        // Sender 丢了（sink 正在析构），循环随之结束。
        loop {
            match cmd {
                LogCommand::Append(text) => {
                    if let Err(e) = writer.write_all(text.as_bytes()) {
                        let _ = events.send(LogEvent::Failed(format!("写入日志失败: {e}")));
                        return;
                    }
                }
                LogCommand::Halt => break 'outer,
            }

            // 一次排空队列：高吞吐时把多次写入合并到同一个缓冲区，队列真排空了
            // 再 flush，这样低速场景下文件也能被 tail 之类实时看到。
            match commands.try_recv() {
                Ok(next) => cmd = next,
                Err(TryRecvError::Empty) => {
                    if let Err(e) = writer.flush() {
                        let _ = events.send(LogEvent::Failed(format!("写入日志失败: {e}")));
                        return;
                    }
                    break;
                }
                Err(TryRecvError::Disconnected) => break 'outer,
            }
        }
    }

    // 正常收工：把缓冲区里剩下的内容落盘。写失败路径上 `BufWriter` 的 Drop 也会
    // 尽力冲刷一次，但那里的错误已经没有接收方，只能丢弃。
    let _ = writer.flush();
}

/// 一个正在运行的日志文件。
///
/// 句柄、线程与指令通道都归它所有；被丢弃时自动投 `Halt` 并等待写盘线程收工，
/// 因此「停止记录」天然带上一次 flush，不会丢最后一段（`BufWriter` 里还欠着的）
/// 内容。
pub struct LogSink {
    path: PathBuf,
    events: Receiver<LogEvent>,
    commands: Option<SyncSender<LogCommand>>,
    worker: Option<JoinHandle<()>>,
    written: u64,
}

impl LogSink {
    /// 在已打开的文件上拉起写盘线程。
    ///
    /// `file` 由调用方（UI 线程）负责打开，好处见模块文档第 2 条：打开失败能够
    /// 当场回报，而不是先显示成功再回滚。
    pub fn start(path: PathBuf, file: File) -> Self {
        let (command_tx, command_rx) = mpsc::sync_channel::<LogCommand>(QUEUE_DEPTH);
        let (event_tx, event_rx) = mpsc::channel::<LogEvent>();

        let worker = thread::spawn(move || worker_main(file, command_rx, event_tx));

        Self {
            path,
            events: event_rx,
            commands: Some(command_tx),
            worker: Some(worker),
            written: 0,
        }
    }

    /// 日志文件的实际路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 已经交给写盘线程的字节数（不含还留在内存镜像里的内容）。
    pub fn written(&self) -> u64 {
        self.written
    }

    /// 追加一段文本；返回 `false` 表示**待写队列已满**（磁盘明显跟不上）。
    ///
    /// 队列满时这一条会被丢弃，调用方应立即停止记录并告知用户（见
    /// [`ComTool::log_append`]）——丢掉末尾一小段并说清楚，好过让队列无上限增长，
    /// 也好过静默丢弃。
    ///
    /// 投递失败（写盘线程已退出）在这里按「已接受」处理：那种情况会经
    /// [`LogEvent::Failed`] 上报，不必让收数据的热路径去处理磁盘故障。
    pub fn append(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return true;
        }
        let Some(commands) = &self.commands else {
            return true;
        };

        match commands.try_send(LogCommand::Append(text.to_string())) {
            Ok(()) => {
                self.written += text.len() as u64;
                true
            }
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => true,
        }
    }

    /// 取走写盘线程上报的全部事件（非阻塞）。
    pub fn drain_events(&self, out: &mut Vec<LogEvent>) {
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
    }
}

impl Drop for LogSink {
    fn drop(&mut self) {
        // 先取出 Sender 再发停机指令，随后立刻 drop 它：即便 `Halt` 因为某种原因
        // 没被读到，写盘线程也会观察到 `Disconnected` 而收工（与串口通道一致）。
        //
        // 这里必须用 `try_send`：队列可能是满的，而 `send` 在队列满时会**阻塞**
        // 到有消费者取走为止——一个卡住的磁盘就能让「停止记录」把 UI 线程永久
        // 挂住。投不进去也不要紧，紧接着 drop 掉 Sender 同样能让线程收工。
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(LogCommand::Halt);
        }
        // `join` 会等到 flush 完成才返回。付出的是「停止记录」可能短暂阻塞 UI 线程
        // 的代价，换来的是最后一小段内容一定落盘——进程随后就可能退出，不能把
        // 缓冲区的责任留给析构顺序。
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ComTool {
    /// 当前是否正在记录日志。
    ///
    /// 以 [`ComTool::log`] 是否为 `Some` 作为唯一事实来源，不另设 `bool` 标志位——
    /// 两个字段表达同一件事迟早会不一致。
    pub fn is_logging(&self) -> bool {
        self.log.is_some()
    }

    /// 正在记录的日志文件路径。
    pub fn log_path(&self) -> Option<&Path> {
        self.log.as_ref().map(LogSink::path)
    }

    /// 本次记录已写入的字节数。
    pub fn log_written_bytes(&self) -> u64 {
        self.log.as_ref().map_or(0, LogSink::written)
    }

    /// 实际生效的日志输出目录（偏好为空时用 [`default_log_dir`]）。
    pub fn effective_log_dir(&self) -> PathBuf {
        if self.log_dir.trim().is_empty() {
            default_log_dir()
        } else {
            PathBuf::from(self.log_dir.trim())
        }
    }

    /// 把一段文本追加进接收镜像，并同步写进实时文件日志。
    ///
    /// 「已打开串口」这类一次性提示也走这里：日志文件因此自带上下文，而不只是一串
    /// 裸数据——排查问题时能看出「这段数据是在哪次连接之后收到的」。
    pub(crate) fn push_receive_note(&mut self, text: String) {
        self.receive_text.push_str(&text);
        self.log_append(&text);
    }

    /// 只投递给实时文件日志（未开启时为空操作）。
    ///
    /// 队列满 = 磁盘跟不上数据速率（网络盘掉线、盘满、机械盘被别的进程抢占 IO）。
    /// 此时**主动停下来并把原因说清楚**：继续投递会让内存无上限增长，而串口工具
    /// 挂机跑一整天是常见用法；静默丢弃则会让用户拿着一份缺了中段的日志去排查问题。
    pub(crate) fn log_append(&mut self, text: &str) {
        let backlogged = match &mut self.log {
            Some(sink) => !sink.append(text),
            None => return,
        };

        if backlogged {
            self.stop_logging();
            self.push_receive_note(
                "实时文件日志已停止：写入速度跟不上数据速率（检查磁盘是否可用）\n".to_string(),
            );
        }
    }

    /// 开始与停止之间切换（菜单项与侧边栏按钮共用）。
    pub fn toggle_logging(&mut self) {
        if self.is_logging() {
            self.stop_logging();
        } else {
            self.start_logging();
        }
    }

    /// 开始记录日志。
    ///
    /// 目录创建与文件打开都在这里同步完成，失败就直接把原因写进接收区、不进入
    /// 「记录中」状态——宁可让用户当场看到失败，也不要显示一个假的成功状态。
    pub fn start_logging(&mut self) {
        if self.is_logging() {
            return;
        }

        let dir = self.effective_log_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.push_receive_note(format!("创建日志目录失败（{}）: {e}\n", dir.display()));
            return;
        }

        let name = render_log_file_name(&self.default_log_file_name, Local::now());
        let (path, file) = match create_log_file(&dir, &name) {
            Ok(created) => created,
            Err(e) => {
                self.push_receive_note(format!("创建日志文件失败（{}）: {e}\n", dir.display()));
                return;
            }
        };

        self.log = Some(LogSink::start(path.clone(), file));
        // 这条提示会被写进日志文件本身，于是文件开头就说明了它是什么。
        self.push_receive_note(format!("开始记录日志: {}\n", path.display()));
    }

    /// 停止记录日志（幂等：没在记录时什么也不做）。
    pub fn stop_logging(&mut self) {
        if self.log.is_none() {
            return;
        }

        // 先给日志文件补一行收尾提示，再停机：文件最后一行能说明「记录到此结束」，
        // 而不是留下一个无法判断是否被中途截断的结尾。
        self.log_append("已停止记录日志\n");

        let Some(sink) = self.log.take() else {
            return;
        };
        let path = sink.path().to_path_buf();
        // 显式 drop 而不是等字段析构：`join` 返回时缓冲区已经落盘。
        drop(sink);

        // 这一条只进接收镜像（sink 已丢弃），不会重复写进文件。
        self.push_receive_note(format!("已停止记录日志: {}\n", path.display()));
    }

    /// 选择日志输出目录（`None` 表示用户取消，保持原值）。
    pub fn pick_log_dir(&mut self) {
        let mut dialog =
            rfd::FileDialog::new().set_title(self.language.strings().pick_log_dir_title);
        let current = self.effective_log_dir();
        if current.is_dir() {
            dialog = dialog.set_directory(&current);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.log_dir = dir.display().to_string();
        }
    }

    /// 恢复默认输出目录。
    pub fn reset_log_dir(&mut self) {
        self.log_dir.clear();
    }

    /// 每帧检查写盘线程的上报。
    ///
    /// 磁盘故障（盘满、可移动盘被拔）不会让 UI 线程有任何感知，唯一通道就是这里；
    /// 收到失败后立即关闭记录状态，否则界面会一直显示「记录中」而文件早已停止增长。
    pub fn poll_log_events(&mut self) {
        let mut events = Vec::new();
        if let Some(sink) = &self.log {
            sink.drain_events(&mut events);
        }
        if events.is_empty() {
            return;
        }

        for event in events {
            match event {
                LogEvent::Failed(msg) => {
                    // 先停机再提示：提示会被写进文件，而文件已经写不动了。
                    self.stop_logging();
                    self.push_receive_note(format!("实时文件日志已中断: {msg}\n"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communicationtrait::ChannelEvent;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 固定时刻，让文件名断言与当前时间无关。
    fn fixed_now() -> chrono::DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 9, 21, 10, 54, 20)
            .single()
            .expect("本地时间应唯一")
    }

    static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

    /// 每次调用都得到一个全新的空目录，避免测试之间互相干扰。
    ///
    /// 不用 `tempfile` crate：为一个测试目录引入新依赖，成本大于收益。
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("comtool-log-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录应可创建");
        dir
    }

    #[test]
    fn default_template_is_expanded() {
        assert_eq!(
            render_log_file_name(DEFAULT_LOG_FILE_NAME, fixed_now()),
            "LogFile_2026-09-21_10-54-20.txt"
        );
    }

    /// 模板是用户可编辑的文本，也是可以被手改的存档内容：非法占位符必须退回默认
    /// 模板，而不是让 `DateTime::format` panic 把程序带崩。
    #[test]
    fn invalid_template_falls_back_instead_of_panicking() {
        let expected = render_log_file_name(DEFAULT_LOG_FILE_NAME, fixed_now());

        for bad in ["%Q", "", "%", "abc%Qdef", "%Y-%"] {
            assert_eq!(
                render_log_file_name(bad, fixed_now()),
                expected,
                "模板 {bad:?} 应退回默认模板"
            );
        }
    }

    /// 模板里的路径分隔符不得残留，否则日志会被写到选定目录之外。
    #[test]
    fn template_cannot_escape_the_target_directory() {
        for bad in ["../evil.txt", r"..\..\evil.txt", "sub/dir.txt", "a:b.txt"] {
            let name = render_log_file_name(bad, fixed_now());
            assert!(
                !name.contains(['/', '\\', ':']),
                "模板 {bad:?} 渲染出了含路径分隔符的名字: {name}"
            );
        }

        // 全是路径分隔符 / 全空的模板退回默认名字，而不是产生一个空文件名
        assert_eq!(
            render_log_file_name("///", fixed_now()),
            render_log_file_name(DEFAULT_LOG_FILE_NAME, fixed_now())
        );
    }

    /// 大小写与纯文本模板（不含时间占位符）应当原样保留。
    #[test]
    fn literal_template_is_kept_verbatim() {
        assert_eq!(
            render_log_file_name("session.log", fixed_now()),
            "session.log"
        );
    }

    /// 同名文件绝不能截断：必须换名，且旧文件内容完整。
    #[test]
    fn create_log_file_never_truncates_an_existing_file() {
        let dir = unique_temp_dir("collision");

        let (first, file) = create_log_file(&dir, "LogFile_x.txt").expect("首次创建应成功");
        drop(file);
        std::fs::write(&first, b"first run").expect("写入应成功");

        let (second, file) = create_log_file(&dir, "LogFile_x.txt").expect("重名时应换名创建");
        drop(file);

        assert_ne!(first, second, "重名时必须换一个名字");
        assert_eq!(second.file_name().unwrap(), "LogFile_x_1.txt");
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            "first run",
            "已有日志的内容不得被清空"
        );

        // 第三次再撞名，序号继续往后走
        let (third, file) = create_log_file(&dir, "LogFile_x.txt").expect("重名时应换名创建");
        drop(file);
        assert_eq!(third.file_name().unwrap(), "LogFile_x_2.txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目录不存在时必须如实报错，而不是悄悄写到别处。
    #[test]
    fn create_log_file_reports_missing_directory() {
        let dir = unique_temp_dir("missing");
        let absent = dir.join("no_such_subdir");

        let err = create_log_file(&absent, "a.txt").expect_err("父目录不存在时应报错");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写盘线程的端到端行为：追加的内容必须在 sink 析构后完整落在文件里。
    ///
    /// 这条覆盖的是「`Halt` → flush → join」这条链路。若停机不 flush，最后一段
    /// 还欠在 `BufWriter` 里的内容就会随进程退出一起消失，而进程退出恰恰是
    /// 「停止记录」之后马上会发生的事。
    #[test]
    fn sink_flushes_everything_on_drop() {
        let dir = unique_temp_dir("flush");
        let (path, file) = create_log_file(&dir, "run.txt").expect("创建应成功");

        // 16 KiB 的定长内容（不含换行，便于按长度与内容核对）
        let chunk = "0123456789abcdef".repeat(1024);
        assert_eq!(chunk.len(), 16 * 1024);

        {
            let mut sink = LogSink::start(path.clone(), file);
            // 10 条 × 16 KiB：远超 64 KiB 写缓冲，因此中途必然触发过缓冲写满的
            // 自动落盘；条数又远小于队列深度，不会受「队列满」影响，断言可重复。
            for _ in 0..10 {
                assert!(sink.append(&chunk), "队列未满时投递必须成功");
            }
            assert_eq!(sink.written(), (chunk.len() * 10) as u64);
        }

        let written = std::fs::read_to_string(&path).expect("日志文件应可读");
        assert_eq!(written.len(), chunk.len() * 10, "一条都不能少");
        assert_eq!(written, chunk.repeat(10));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 队列满时必须如实回报，而不是继续堆积（内存无上限增长）。
    ///
    /// 直接构造一个「深度 1、没有消费者」的 sink 来制造满队列：真去卡住磁盘不现实，
    /// 而这里要验证的正是「满」这一分支本身。
    #[test]
    fn append_reports_backlog_instead_of_growing_memory() {
        let (tx, rx) = mpsc::sync_channel(QUEUE_DEPTH);
        let (_event_tx, event_rx) = mpsc::channel();
        let mut sink = LogSink {
            path: PathBuf::from("unused.txt"),
            events: event_rx,
            commands: Some(tx),
            worker: None,
            written: 0,
        };

        // 填满队列（没有消费者，因此永远排不空）
        for _ in 0..QUEUE_DEPTH {
            assert!(sink.append("x"), "队列还有空位时应投递成功");
        }
        assert!(
            !sink.append("溢出的这一条"),
            "队列已满时必须返回 false，由调用方停下来说明原因"
        );
        assert_eq!(
            sink.written(),
            QUEUE_DEPTH as u64,
            "被拒绝的条目不计入字节数"
        );

        // 析构绝不能阻塞：队列是满的，`send` 式的停机在这里会把线程永久挂住。
        drop(sink);
        drop(rx);
    }

    /// 空文本不需要进通道（收数据热路径上会频繁出现）。
    #[test]
    fn appending_empty_text_is_a_noop() {
        let dir = unique_temp_dir("empty");
        let (path, file) = create_log_file(&dir, "run.txt").expect("创建应成功");

        let mut sink = LogSink::start(path.clone(), file);
        assert!(sink.append(""), "空文本应被接受但什么也不投递");
        assert_eq!(sink.written(), 0);
        drop(sink);

        assert!(std::fs::read_to_string(&path).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 未开启记录时，投递路径必须完全静默（不能 panic，也不能造出文件）。
    #[test]
    fn logging_off_is_a_noop() {
        let mut app = ComTool::default();
        assert!(!app.is_logging());
        assert!(app.log_path().is_none());
        assert_eq!(app.log_written_bytes(), 0);

        app.log_append("谁都不该看到这行");
        app.stop_logging();

        assert!(app.receive_text.is_empty());
        assert!(!app.is_logging());
    }

    /// 「日志文件内容 = 接收区内容」这条承诺要有测试兜住：日志是在接收镜像写入的
    /// 同一处投递的，漏掉任何一个分支都会让日志悄悄少一段数据。
    ///
    /// 数据经事件通道注入——`receive_data` 是唯一的数据入口，因此这里覆盖的正是
    /// 真实串口走的同一条链路（不需要串口硬件）。
    #[test]
    fn log_file_mirrors_the_receive_pane() {
        let dir = unique_temp_dir("mirror");
        let (path, file) = create_log_file(&dir, "run.txt").expect("创建应成功");

        let mut app = ComTool {
            log: Some(LogSink::start(path.clone(), file)),
            ..ComTool::default()
        };
        assert!(app.is_logging());

        let (tx, rx) = mpsc::channel();
        app.event_rx = Some(rx);
        tx.send(ChannelEvent::Bytes(b"device output\n".to_vec()))
            .expect("投递应成功");

        app.receive_data();
        assert!(
            app.receive_text.contains("device output\n"),
            "接收区应先拿到数据，实际为: {}",
            app.receive_text
        );

        // 一次性提示也属于「接收区内容」，同样要进日志
        app.push_receive_note("已打开串口\n".to_string());
        app.stop_logging();

        assert!(!app.is_logging());
        assert!(app.log_path().is_none());

        let written = std::fs::read_to_string(&path).expect("日志文件应可读");
        assert_eq!(
            written, "device output\n已打开串口\n已停止记录日志\n",
            "日志文件必须与接收区同步，且末尾带上收尾行"
        );

        // 接收镜像包含上述全部内容（收尾提示措辞略不同，故只核对前缀）
        assert!(app.receive_text.starts_with("device output\n已打开串口\n"));
        assert!(app.receive_text.contains("已停止记录日志: "));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 走完整条「开始 → 收数据 → 停止」链路的端到端测试。
    ///
    /// 起点是 `start_logging`（含目录创建、模板渲染、文件打开），终点是磁盘上的
    /// 文件内容——不碰串口，也不需要 GUI。
    #[test]
    fn start_and_stop_logging_writes_the_file() {
        let dir = unique_temp_dir("roundtrip");
        let mut app = ComTool {
            log_dir: dir.display().to_string(),
            default_log_file_name: "cap_%Y%m%d.txt".to_string(),
            ..ComTool::default()
        };

        app.start_logging();
        assert!(app.is_logging(), "开始记录后应处于记录状态");
        let path = app.log_path().expect("应有路径").to_path_buf();
        assert_eq!(path.parent(), Some(dir.as_path()));
        // 不核对具体日期：`%Y%m%d` 由当前时间展开，断言里写死日期会让测试明天就红。
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("cap_")
                && name.ends_with(".txt")
                && name.len() == "cap_20260921.txt".len(),
            "文件名应来自用户模板，实际为: {name}"
        );

        app.log_append("hello\n");
        assert!(app.log_written_bytes() >= "hello\n".len() as u64);

        app.stop_logging();

        let written = std::fs::read_to_string(&path).expect("日志文件应可读");
        assert!(written.contains("hello\n"));
        assert!(written.ends_with("已停止记录日志\n"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目录建不出来时必须如实报错，且不得进入「记录中」状态——假成功比失败更糟。
    #[test]
    fn start_logging_reports_failure_without_faking_success() {
        let dir = unique_temp_dir("bad-dir");
        // 把输出目录指到一个**文件**上：`create_dir_all` 必然失败。
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"x").expect("应能建出占位文件");

        let mut app = ComTool {
            log_dir: blocker.display().to_string(),
            ..ComTool::default()
        };
        app.start_logging();

        assert!(!app.is_logging(), "目录创建失败时不得显示「记录中」");
        assert!(app.log_path().is_none());
        assert!(
            app.receive_text.contains("创建日志目录失败"),
            "失败原因要写进接收区，实际为: {}",
            app.receive_text
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 偏好为空时用默认目录，设置了就用设置的。
    #[test]
    fn effective_log_dir_prefers_the_user_choice() {
        let mut app = ComTool::default();
        assert_eq!(app.effective_log_dir(), default_log_dir());

        app.log_dir = "  E:/tmp/serial-logs  ".to_string();
        assert_eq!(app.effective_log_dir(), PathBuf::from("E:/tmp/serial-logs"));

        app.log_dir = "   ".to_string();
        assert_eq!(app.effective_log_dir(), default_log_dir());
    }
}
