# comtool-rs 开发参考文档

> 本项目是一个基于 `eframe`/`egui` 的串口调试工具。本文档描述当前项目的代码组织方式、功能开发的完整流程，以及各层职责划分，作为新增功能的参考。

---

## 0. ⛔ 运行禁令（最高优先级，动手前先读）

**未经项目所有者在本轮对话中逐次、明确批准，禁止以任何形式运行本项目，禁止以任何方式占用或打断 COM3 串口。**

- **禁止运行**：`cargo run`（含 `--release`）、直接执行 `target/**/comtool-rs.exe`、
  `cargo install` 后运行、`start`/`Start-Process`/后台任务/计划任务等一切拉起本程序的方式；
  也包括任何会间接启动本程序本体的脚本、测试或工具调用。
- **禁止触碰 COM3**：不得打开、连接、读写、配置、探测 COM3（`mode COM3`、`echo > COM3`、
  `pyserial`、`[System.IO.Ports.SerialPort]`、第三方串口助手等一律禁止）；不得把 COM3 写进
  默认值、配置、测试或脚本；不得停止/杀掉/禁用任何可能占用 COM3 的进程、服务或驱动。
  对 COM3 只能"看"（不打开端口的只读枚举，且需先说明理由），不能"碰"。
- **默认允许**：`cargo fmt` / `cargo check --all-targets` / `cargo test` / `cargo clippy` /
  `cargo build` 等不启动 GUI、不触碰串口的命令。230 条测试全部不需要串口硬件，
  所有验收一律以这些命令为准。
- **确实需要运行时**：停下来说明原因并请求批准，由项目所有者自行运行；不得"先跑再报"。
- 一次批准只对当次生效，不构成对后续轮次的授权。

> 完整条文见 `AGENTS.md` 顶部「最高优先级红线」一节，两者冲突时以 `AGENTS.md` 为准。
> `AGENTS.md` **已纳入版本控制**（原先被 `.gitignore` 排除，2026-09-21 起取消），
> 它记录的两条拦截规则因此随仓库传播；**守卫的行为自测也跟着走**——
> `test_guard.py` 已在版本控制中（`.gitignore` 里为它开了例外，是 `.workbuddy/` 下唯一放行的文件）。
> 但**被测试的那个脚本本体仍不在仓库里**：`comtool-run-guard.sh` 与批准脚本在用户级目录
> `~/.workbuddy/hooks/`。所以新克隆的仓库能读到"规则是什么"、也能读到"该怎么自测"，
> 却没有可测的对象——换机器或重装后必须单独重建这两个脚本，
> 否则条文还在、机器级拦截会静默失效。

### 0.1 已配置机器级硬拦截（PreToolUse hook）

上面的禁令不只是文档约定，已经装了钩子：**命中即拒绝工具调用**。

| 项 | 位置 |
|---|---|
| 拦截逻辑 | `~/.workbuddy/hooks/comtool-run-guard.sh` |
| 注册位置 | `~/.workbuddy/settings.json`、`~/.codebuddy/settings.json` → `hooks.PreToolUse`，`matcher = Bash/PowerShell/Write/Edit` |
| 自测 | `python3 .workbuddy/hooks/test_guard.py`（39 条断言；不启动程序、不碰串口）。**已在版本控制中** |
| 批准 | `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（**用户本人在 agent 之外执行**；`0` 撤销） |

- 批准文件 `<配置目录>/combot-run-approval` 第一行是过期 Unix 秒，不存在/过期/损坏 → 全部拒绝；
  **批准只放行"运行本项目"，COM3 操作永远拒绝**。
- 批准文件与批准脚本 AI 无权触碰（会被守卫自己拦掉）。
- 守卫 fail-closed：payload 解析失败或 stdin 为空一律拒绝。
- **已知误拦（改用别的工具即可）**：命令**文本**里出现 `cargo run` / `COM3` / `comtool-rs.exe`
  字面量即被拦。例如 `grep "cargo run" AGENTS.md` 会被拒，请用 Grep 工具；把该短语写进
  commit message 也会被拒，可先 Write 一个消息文件再 `git commit -F <file>`。
  写日志/文档时若要提到批准脚本，**别写它的文件名**——那个名字本身在拦截名单上，
  会让"记录这次改动"的命令自己被拦掉。
- **`Write` / `Edit` 只检查 `file_path`，不看内容**（守卫脚本里 `CMD="${FPATH:-}"`）：
  所以把 `cargo run` / `COM3` 这些字面量写进文档或源码是允许的，也正是本文件能存在的原因。
  反过来说，**这条拦截拦不住"通过写文件去改守卫"以外的任何内容特征**——它防的是"动手"，
  不是"写字"。当心的例外是 `file_path` 字段解析失败时会退化为扫描整段 payload（fail-closed）。
- ⚠️ **跑自测脚本会临时创建一个真实的批准文件**：`test_guard.py` 在"批准文件有效"那一档
  真的往 `<配置目录>` 写一个 +30 分钟的批准时刻，跑到最后才删掉。**若中途中断，会留下一个
  约 30 分钟的预授权**，那段时间里守卫放行"运行本项目"。所以别在自测跑到一半时打断它；
  跑完若不放心的，自己确认该文件已被删除。

---

## 1. 项目架构概览

本项目采用 **egui 即时模式（immediate mode）UI**，全部状态集中在一个核心结构体 `ComTool` 上，逻辑按职责分到几个模块：

| 层 | 位置 | 职责 | 典型例子 |
|---|---|---|---|
| **状态层** | `struct ComTool`（`src/app.rs`） | 定义功能所需的跨帧/跨调用状态字段 | `connected`、`display_spans`、`color_view` |
| **初始化层** | `impl Default for ComTool`（`src/app.rs`） | 给所有状态字段设默认值（漏了会编译报错） | `color_view: false`、`baud_rate: DEFAULT_BAUD_RATE` |
| **业务逻辑层** | `impl ComTool`（`src/app.rs` / `src/receive.rs` / `src/log_file.rs` / `src/chart.rs`） | 连接生命周期（串口 / TCP / UDP）、数据解析、布局构建、日志写盘、数值流解析 | `open_connection()`、`receive_data()`、`build_layout()`、`start_logging()` |
| **UI 层** | `impl eframe::App for ComTool`（`src/app.rs`）+ `show_sidebar()`（`src/sidebar.rs`） | 绘制控件并把 UI 事件接到业务方法上 | `ui.button("发送").clicked()` |
| **持久化层** | `src/config.rs` | 决定哪些字段落盘、哪些在启动时重建 | `load()`、`save()`、`restore_runtime_state()` |

### 模块一览

| 文件 | 职责 |
|---|---|
| `src/main.rs` | 入口；`APP_TITLE` 常量；字体回退链配置；`eframe::run_native`；把 `app_icon::generate()` 挂进 `ViewportBuilder` |
| `src/app_icon.rs` | 应用图标：256×256 圆角方波图标，代码绘制、零资源文件、11 条单元测试，不依赖 GUI |
| `src/app.rs` | `ComTool` 状态、`Default`、`Language` / `ThemeChoice` / `ConnectionKind`、连接生命周期（`open_connection()` 按类型分叉）、视图级辅助方法、`eframe::App` 顶层编排；F5 起还有控制线（`apply_line_state`）、发送计数、端口热插拔（`rescan_ports` / `note_focus`）与常用数据区（`render_send_presets`） |
| `src/receive.rs` | 接收链路：四份镜像状态、`AnsiDecoder`（含 `TextEncoding` 增量解码）、字节级可视化、可选时间戳前缀（`TimestampPrefix`）、三种视图渲染、`max_log_chars` FIFO 淘汰；`ChannelEvent::Notice` 只写提示、不改连接状态 |
| `src/send.rs` | 发送侧纯逻辑：`parse_hex()`、`parse_escapes()`、`encode_text()` / `build_payload()`、`SendFormat`（文本 / HEX / 转义）、`SendHistory` / `HistoryNav`、`SendPresets`、定时发送间隔钳制；**不依赖 `egui` 与串口**，因此可直接单测 |
| `src/strings.rs` | 界面文案表：`Strings` + `ZH` / `EN` + `Language::strings()`；`fill()` 填充 `{}` 模板；源码级守卫测试（UI 源码不许硬编码中文） |
| `src/log_file.rs` | 实时文件日志：后台写盘线程（`LogSink`）、文件名模板渲染、同名防截断、待写队列上限 |
| `src/chart.rs` | 实时图表数据层：原始字节 → 多通道数值序列（`ChartData`）、跨包分片、样本 FIFO、非数值计数；不依赖 `egui` |
| `src/sidebar.rs` | 右侧边栏；返回 `SidebarAction` 而不是在绘制过程中改状态；连接类型单选 + 「串口参数 / 网络参数」双分支（F4）；F5 起含发送字节计数、波特率自由输入与控制线 RTS/DTR 一行 |
| `src/panels/mod.rs` | 面板模块声明 + 无头 UI 冒烟测试（含英文界面、已连接侧边栏、常用数据行） |
| `src/panels/menu_bar.rs` | 顶部菜单栏（文件 / 设置 / 视图 / 文本 / 发送）；返回 `MenuAction`；各档单选（主题 / 时间戳 / 编码 / 发送格式与行尾）共用一处定义 |
| `src/panels/settings.rs` | 设置窗口：把用户偏好接到真正的控件上 |
| `src/panels/info.rs` | 信息窗口：版本、技术栈、存档路径、快捷键 |
| `src/panels/chart.rs` | 实时图表面板：`egui_plot` 绘制、图例、绘制点抽样（`points_for_drawing`） |
| `src/config.rs` | 持久化策略 + 启动时不变量修复 |
| `src/communicationtrait.rs` | `ByteChannel` trait（`open` / `close` / `is_active` / `write` / `configure` / `set_line_state` / `local_addr`）与 `ChannelEvent`（`Bytes` / `Failed` / `Closed` / `Notice`）；三者共用的后台线程脚手架 `worker`（`WorkerCommand` / `StateGuard` / `is_transient`）；附 `#[cfg(test)] mod fake` 通道替身 |
| `src/serial_impl/channel.rs` | 串口实现：后台线程 + 命令/事件双通道；唯一覆盖 `set_line_state` 的实现 |
| `src/serial_impl/config.rs` | `SerialConfig` 与波特率档位；任意波特率的区间与 `clamp_baud()` |
| `src/net_impl/config.rs` | `NetConfig`：远端地址 / 端口 / UDP 本地端口 + `validate()` / `remote_addr()` |
| `src/net_impl/tcp.rs` | `TcpChannel`：**同步 connect**、`set_nodelay`、对端关闭发 `ChannelEvent::Closed` |
| `src/net_impl/udp.rs` | `UdpChannel`：绑 `0.0.0.0:local_port` 后 `connect` 固化对端；零长度数据报不是 EOF |

---

## 2. 数据流总览

```
串口后台线程 --(ChannelEvent::Bytes)--> ComTool.event_rx
        │
        ▼
receive_data()                     ← 业务层（src/receive.rs）：拉取原始字节
   ├─ 原始字节 → chart.push_bytes()  → 多通道数值序列（与显示模式无关，见 §2.2）
   ├─ HEX 模式    → 逐字节转文本         ┐
   ├─ 字节查看    → ascii_spans_from_bytes ├→ 片段 → apply_timestamp_prefix()（§2.3）
   └─ 文本模式    → decoder.feed() 解析 ANSI 颜色 ┘
        │        （三条分支的片段走同一条公共尾部：加时间戳 → 写片段镜像）
        │        （同一份文本再 append 到 receive_text，并经 log_append() 投给文件日志，见 §2.1）
        ▼
display_spans / ascii_spans         ← 状态层：片段缓存（内存）
        │
        ▼
build_layout() / build_ascii_layout() ← 业务层：转成 egui 的 LayoutJob
        │
        ▼
egui::Label::new(job).wrap()        ← UI 层：渲染多色文本
```

### 2.1 实时文件日志的旁路（P4）

接收链路有第二条出海口，与渲染无关：

```
receive_data() ─┬─ 四个内存镜像（显示用，受 max_log_chars 截断）
                └─ log_append(text) ──sync_channel──> 写盘线程 ──BufWriter──> 日志文件
                                                      （队列一排空就 flush）
```

- 投递口只有两个：`ComTool::log_append()`（只写日志）与 `ComTool::push_receive_note()`
  （同时写 `receive_text` 与日志）。**不要在 `receive_data` 里绕开它们直接 `push_str`**。
- 日志文件**不受** `max_log_chars` 约束，也不会被「清空日志」截断——那两个只作用于内存镜像。
- 写盘失败或队列满都由 `poll_log_events()` 在每帧开头检查，命中即自动停止记录并写一条提示。

### 2.2 实时图表的旁路（P5）

图表**不是第五份镜像**：它有独立的解析规则、独立的上限、独立的清空入口
（`ComTool::clear_chart()`），只是恰好也从 `receive_data` 拿到同一批原始字节。

```
receive_data() ──原始字节──> ChartData::push_bytes() ──> 每通道 VecDeque<[x, y]>
                                                            │
                       panels/chart.rs::render_chart_panel() ◄┘
                       （每帧抽样到 ≤4000 点/通道，交给 egui_plot）
```

- **喂数据的位置在三种显示模式的分支之前**，所以切到 HEX / 字节查看时曲线照常推进。
  不要把它挪进任何一个模式分支。
- **输入格式：一行一组样本，字段用逗号分隔**（`\n` 或 `\r\n` 结束一组）。第 *k* 个字段
  永远对应第 *k* 条曲线；同一组的字段共享同一个 x（**组序号**，不是时间戳）。
- 解析上的几条硬约定（改动前先读 `src/chart.rs` 的模块文档）：
  - 整行没有数值 → **不产出样本，也不推进 x**（否则文本噪声会在曲线上打出空洞）；
  - 非数值字段 → 计入 `ignored_fields` 并在界面上提示，绝不静默丢弃；
  - 某行字段数少于已建立的通道数 → 该组这些通道**不记样本**（曲线断开），
    **不许补 0**；补 0 会画出从未到达的数据；
  - 半行超过 `MAX_PENDING_BYTES` → **整行丢弃**，该行剩余字节一直丢到换行符为止
    （否则被腰斩的尾部会变成一个假样本）；
  - 通道数只增不减（由最宽的行决定），上限 `MAX_CHANNELS`（16）；每通道样本数
    受 `chart_max_samples` 约束，超出时从最旧的开始淘汰。
- 图表数据是纯运行时状态（`#[serde(skip)]`），跨会话不保留；`chart_max_samples`
  在 `restore_runtime_state` 里夹到合法区间。

### 2.3 时间戳前缀（P3）

可选的显示注解，默认**关闭**（关闭时接收链路与没有这个功能时逐字节相同）：

```
TimestampPrefix::{Off, PerLine, PerBatch}      ← 设置窗口「接收」区 / 视图菜单
        │
        ▼
apply_timestamp_prefix(spans)                  ← 必须是本批写入镜像**之前**读行首状态
        ├─ 前缀文本 = format_timestamp(Local::now())   每批只取一次时间
        └─ timestamped_spans(spans, mode, prefix, at_line_start)
                 │  按 `\n` 切行，在每个行首插入一个灰色临时片段
                 ▼
        片段镜像（display_spans / ascii_spans）→ 拼接出 receive_text → 导出 / 复制 / 文件日志
```

- **在片段层插，不在文本层插**：`receive_text` 由片段拼接而来，两处各插一遍迟早走偏。
  只插一次，四份镜像自然一致——这也是它不需要新增镜像的原因。
- **它是注解，不是数据**：只进显示文本（含导出与文件日志），**绝不**进 `chart.push_bytes()`
  的那份原始字节。两条路在 `receive_data` 里是分开的。
- **行首状态不另设字段**：`at_receive_line_start()` 从 `receive_text` 的尾部推导
  （空串或结尾是 `\n`）。三种显示模式、程序提示、`clear_receive()` 都会写这个镜像，
  任何并行状态字段都会在某个分支上与它不一致。
- **没有内容就不加前缀**（判据是"有非空片段"，不是"片段数组非空"）：文本模式收到
  半个多字节序列时片段是空的，此时加前缀会凭空造出一行。
- 行边界就是 `\n`。HEX 视图每批一行，于是"每行"在那里自然退化成"每批"——这是设计如此。
- 镜像落点必须与分支顺序一致：HEX 优先于字节查看，所以落点是
  `!hex_view && ascii_view`，而不是 `ascii_view`。

发送方向：

```
ui.button("发送") → send_data() → ByteChannel::write(bytes)
                                        │
                                        ▼
                              命令通道 → 后台线程 → VecDeque 待发队列
                                        │ 每轮循环只推进一次 write
                                        ▼
                                     串口
```

> **同一份接收数据维护四份镜像**：`receive_text`（纯文本，供导出与普通视图）、`display_spans`（彩色片段）、`ascii_text` / `ascii_spans`（字节可视化）。四者必须同步清空、同步追加——`ComTool::clear_receive()` 是唯一的复位入口。裁剪只作用于这四份内存镜像；磁盘上的实时日志文件是另一条旁路（见 §2.1）。开启时间戳前缀时（§2.3），前缀是这些镜像内容的一部分，因此导出文件与文件日志里也会带上它。

---

## 3. 功能开发流程（核心）

新增一个功能的完整开发步骤如下：

```
┌────────────────────────────────────────────────────────────┐
│ ① 需求分析：这个功能需要"跨帧/跨调用共享的状态"吗？           │
│    └── 需要持久状态？                                       │
│         ├─ 是 → 进入 ②（加字段 + 初始化）                    │
│         └─ 否 → 跳过 ②③，直接进入 ④（纯计算/纯展示）        │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ② 在 struct ComTool 里加状态字段（src/app.rs）              │
│      如：pub rx_byte_count: u64,                          │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ③ 在 impl Default for ComTool 里给新字段设默认值           │
│      如：rx_byte_count: 0,                                 │
│      ⚠ 漏了这步 → 编译报错（E0061 字段未初始化）             │
│                                                            │
│      ⚠⚠ 决定它是否落盘：                                   │
│        · 是用户显式选过、希望下次启动仍生效的偏好 → 保持可序列化 │
│        · 是句柄/连接状态/日志/缓存 → 加 #[serde(skip)]       │
│          并同步在 config::restore_runtime_state 里重建       │
│      ⚠ 漏了这步 → 要么把运行时状态写进存档，要么存档读不回来   │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ④ 在 impl ComTool 里写业务逻辑方法                         │
│      放对模块：接收相关 → receive.rs；串口生命周期 → app.rs  │
│                文件日志相关 → log_file.rs                    │
│                数值流解析 → chart.rs                         │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ⑤ 在 impl eframe::App for ComTool 的 ui() 里添加控件       │
│      （按钮 / 滚动区 / TextEdit / ComboBox …）              │
│      侧边栏内的控件 → sidebar.rs，通过 SidebarAction 上报   │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ⑥ 把 UI 事件接到业务方法（数据流闭环）                      │
│      clicked() → self.业务方法()                            │
│      → 状态字段变化 → 下一帧 ui() 重新渲染                   │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ⑦ cargo fmt && cargo check && cargo test                  │
└────────────────────────────────────────────────────────────┘
```

> **⑤ 的补充（F3 起）**：控件文案**不许**硬编码。取 `let s = self.language.strings();`
> 之后用 `s.字段名`；带占位符的文案用 `strings::fill(s.字段名, &[&实参])`
> （`format!` 不接受变量作格式串）。新增文案在 `src/strings.rs` 的 `strings!` 宏里
> **同时写中英两列**——宏的形态决定了写不出单列。接收区 / 日志里的**提示行不翻译**，
> 详见注意事项第 31 条。

---

## 4. 实例演示：添加「接收字节计数」

走一遍上面的流程，假设要实现"统计并显示已接收字节数"：

```rust
// ① 需要跨帧显示计数 → 需要状态字段

// ② struct ComTool 中加字段（src/app.rs）
pub struct ComTool {
    // ...原有字段...
    pub rx_byte_count: u64,      // 新增：已接收字节数
}

// ③ impl Default 中初始化
impl Default for ComTool {
    fn default() -> Self {
        Self {
            // ...原有字段...
            rx_byte_count: 0,    // 新增：初始为 0
        }
    }
}
// ③′ 这是个运行时计数器，不该跨进程延续 → 需要 #[serde(skip)]

// ④ impl ComTool 中写业务逻辑：在接收数据时累加（src/receive.rs）
impl ComTool {
    pub fn receive_data(&mut self) {
        // ...原有逻辑...
        if !received.is_empty() {
            self.rx_byte_count += received.len() as u64;  // 新增
            // ...
        }
    }
}

// ⑤⑥ impl eframe::App 的 ui() 中显示，并触发刷新
impl eframe::App for ComTool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 原有渲染...
        ui.label(format!("已接收 {} 字节", self.rx_byte_count)); // ⑤ 显示
        ui.ctx().request_repaint_after(Duration::from_millis(30)); // ⑥ 持续刷新
    }
}

// ⑦ cargo fmt && cargo check && cargo test
```

---

## 5. 现有功能与代码位置的对应关系

| 功能 | 状态字段 | 业务方法 | UI 入口 |
|---|---|---|---|
| 建立/断开连接 | `connected`、`channel`、`channel_kind`、`event_rx`、`last_opened_port` | `open_connection()` / `close_connection()` / `teardown_channel()`（`app.rs`） | 侧边栏「打开串口 / 连接 / 绑定」→「关闭串口 / 断开」按钮 |
| 连接类型（串口 / TCP / UDP） | `connection_kind` | `ConnectionKind`（`app.rs`）；参数控件分叉在 `sidebar.rs` | 侧边栏「连接类型」单选（**连接中锁定**） |
| 网络目标与本地端口 | `net_remote_host`、`net_remote_port`、`net_local_port` | `net_config()` / `net_peer_label()`（`app.rs`） | 侧边栏「远端地址 / 远端端口」；UDP 另加「本地端口」 |
| 自动重连 | `last_reconnect_attempt` | `auto_reconnect()`（`app.rs`，**只对串口生效**） | 无（每帧自动尝试，500 ms 节流） |
| 接收数据显示 | `display_spans`、`receive_text`、`decoder` | `receive_data()`、`build_layout()`（`receive.rs`） | 接收区滚动区 + 「颜色」勾选框 |
| 字节查看视图 | `ascii_text`、`ascii_spans` | `render_ascii_receive()`（`receive.rs`） | 「字节查看」勾选框（与「HEX 接收」互斥，见 §6 第 28 条） |
| HEX 模式 | `hex_view` | `receive_data()` 的 HEX 分支 | 「HEX / 文本」勾选框；与「字节查看」互斥，规则在 `resolve_display_mode()` |
| 清空接收 | 全部四份镜像 | `clear_receive()`（`receive.rs`） | 侧边栏「清空日志」按钮 |
| 导出接收数据 | `receive_text` | `export_receive_text()`（`app.rs`） | 侧边栏「导出日志」按钮 |
| 发送数据 | `send_text` | `send_data()`（`app.rs`） | 「发送」按钮 / Enter 键 |
| 端口枚举 | `ports`、`selected_port` | `refresh_ports()`（`app.rs`）、`scan_ports()`（`serial_impl`） | 侧边栏「刷新」按钮 |
| 侧边栏显示/收起 | `sidebar_open` | `show_sidebar()`（`sidebar.rs`） | 「☰ 侧边栏」开关 / 侧边栏「>>」按钮 |
| 状态持久化 | 全部偏好字段 | `config::load()` / `save()` | `eframe::App::save()` 自动触发 |
| 顶部菜单栏 | — | `show_menu_bar()`（`panels/menu_bar.rs`） | 窗口顶部：文件 / 设置 / 视图 |
| 主题切换 | `theme` | `apply_theme()`（`app.rs`） | 设置菜单与设置窗口里的「主题」单选 |
| 设置窗口 | `show_settings` 与各偏好字段 | `show_settings_window()`（`panels/settings.rs`） | 设置菜单 / 「⚙ 设置」按钮 |
| 日志长度上限（FIFO 淘汰） | `max_log_chars` | `trim_receive_logs()`（`receive.rs`） | 设置窗口「日志长度上限」 |
| 自动滚动 | `auto_scroll` | 三个接收视图的 `stick_to_bottom` 参数 | 设置窗口 / 视图菜单 |
| 时间戳前缀 | `timestamp_prefix` | `apply_timestamp_prefix()` / `timestamped_spans()` / `format_timestamp()`（`receive.rs`） | 设置窗口「接收」区 / 视图菜单；侧边栏「接收统计」显示当前档位 |
| 重绘间隔 | `repaint_interval_ms` | `ui()` 里的 `request_repaint_after` | 设置窗口「重绘间隔」 |
| 发送后清空 | `clear_send_after_send` | `send_data()`（`app.rs`） | 设置窗口 |
| 面板显隐（至少留一个） | `sidebar_open` / `show_receive_panel` / `show_chart_panel` / `show_send_panel` | `visible_panel_count()`（`app.rs`） | 视图菜单「面板」 |
| 信息窗口 | `show_info` / `APP_TITLE` | `show_info_window()`（`panels/info.rs`） | 文件菜单「信息」 |
| 实时文件日志 | `log`（`Some` = 记录中）、`log_dir`、`default_log_file_name` | `start_logging()` / `stop_logging()` / `toggle_logging()` / `log_append()`（`log_file.rs`） | 侧边栏「文件日志」区 / 文件菜单「开始-停止记录日志」 |
| 日志写盘故障上报 | — | `poll_log_events()`（`log_file.rs`，每帧在 `receive_data()` 之后调用） | 无（自动停止并在接收区提示） |
| 日志输出目录选择 | `log_dir` | `pick_log_dir()` / `reset_log_dir()` / `effective_log_dir()`（`log_file.rs`） | 设置窗口「选择目录」/「恢复默认」，侧边栏「选择目录」 |
| 实时图表显示 | `show_chart_panel`、`chart`（运行时） | `render_chart_panel()`（`panels/chart.rs`） | 视图菜单「面板 → 实时图表」/ 设置窗口 |
| 数值流解析（多通道） | `chart`、`chart_max_samples` | `ChartData::push_bytes()`（`chart.rs`，在 `receive_data()` 里对原始字节调用） | 无（自动） |
| 图表样本上限 / 自动缩放 | `chart_max_samples`、`chart_auto_bounds` | `ChartData::push_bytes` 的淘汰逻辑 / `Plot::auto_bounds` | 设置窗口「实时图表」区 |
| 清空图表 | `chart` | `clear_chart()`（`chart.rs`） | 图表面板「清空图表」/ 设置窗口 |
| 应用图标 | — | `app_icon::generate()`（`app_icon.rs`） | 编译期生成；`main.rs` 挂进 `eframe::NativeOptions::viewport` |
| release 编译优化级别 | — | `[profile.release] opt-level = 2`（`Cargo.toml`） | 仅影响 `cargo build --release` / `cargo run --release` |
| 发送格式（文本 / HEX） | `send_format` | `parse_hex()` / `build_payload()`（`send.rs`） | 发送区 / 发送菜单 / 设置窗口三处共用 `send_format_radios()` |
| 发送行尾（无 / LF / CRLF） | `send_line_ending` | `build_payload()`（`send.rs`，**字节层**追加） | 发送区 / 发送菜单 / 设置窗口三处共用 `send_line_ending_radios()` |
| 定时发送 | `repeat_send`（运行时，不落盘）、`repeat_interval_ms` | `tick_repeat_send()` / `stop_repeat_send()`（`app.rs`）；间隔钳制在 `send.rs` | 发送区 / 发送菜单的「定时发送」勾选框 + 间隔框 |
| 发送历史与 ↑/↓ 浏览 | `send_history`（落盘）、`send_history_nav`、`send_cursor_index`（运行时） | `SendHistory` / `HistoryNav` / `on_first_line()` / `on_last_line()`（`send.rs`） | 发送区「发送历史」下拉 / ↑↓ 键 / 设置窗口「清空历史」 |
| 收发字符编码 | `text_encoding` | `TextEncoding`（`receive.rs`）、`encode_text()`（`send.rs`） | 文本菜单 / 设置窗口「文本编码」区，两处共用 `text_encoding_radios()` |
| 界面语言 | `language` | `Language::strings()`（`strings.rs`） | 设置窗口「语言」下拉 |
| 界面文案表 | — | `Strings` / `ZH` / `EN` / `fill()`（`strings.rs`） | 所有 UI 控件文案（接收区/日志提示行除外，见 §6 第 31 条） |

---

## 6. 开发注意事项

1. **不是所有功能都要加字段**：只有需要"跨帧/跨调用共享"的数据才进结构体。纯计算/纯展示（如格式化字符串）直接写方法或局部变量即可，避免污染状态。

2. **新增字段务必同步 `impl Default`**：egui 结构体大多通过 `Default::default()` 创建，漏初始化会直接编译失败。

3. **新增字段务必同步持久化策略**：见 §3 步骤 ③。`ComTool` 带容器级 `#[serde(default)]`，所以新增字段本身不会导致旧存档读不出来；但如果它是运行时状态却没有 `#[serde(skip)]`，就会把一个无意义的快照写进磁盘，并在下次启动时带着过期值出现。运行时字段还必须加入 `config::restore_runtime_state` 的重建列表。

   反过来，**删除**一个持久化字段是安全的：反序列化会忽略存档里已经不认识的那些键（`config::tests::archive_with_a_removed_field_still_loads` 把这一点钉住了）。别让那条测试失效——例如给 `ComTool` 加上 `#[serde(deny_unknown_fields)]`，那会让所有旧存档直接读不出来。P6 决定不做版本检查时，就是按这条把 `check_update_on_start` 连同它的设置项一起删掉的：界面里留一个永远不生效的开关，比没有这个功能更糟。

4. **UI 事件与业务逻辑分离**：把数据处理写在 `impl ComTool` 里，`ui()` 只负责画控件和调用方法。侧边栏尤其注意——它在 `sidebar.rs` 里只记录「用户按了什么」，真正的状态变更由 `app.rs` 统一派发。

5. **渲染用即时数据更新**：`ui()` 开头调用 `self.receive_data()` 拉取最新数据（或由 `render_color_receive` 内部调用），配合 `request_repaint_after` 实现持续刷新。

6. **一次性提示不要每帧追加**：类似"串口已打开/已关闭"这类提示，应在事件发生（如 `open_connection`/`close_connection`）时追加到 `display_spans`，而不是在渲染函数里按 `self.connected` 每帧写入，避免日志里不断重复。

7. **三种接收视图共用 `SCROLL_AREA_ID`**：普通文本、彩色、字节查看的 `ScrollArea` 使用同一个 `id_salt`，切换模式时滚动位置保持延续；`clear_receive()` 也用它重置滚动状态。

8. **串口读写绝不能在 UI 线程**：一切 IO 都在 `SerialChannel` 的后台线程里；UI 侧只投递消息。停机靠向命令通道发 `Halt`，不要改成共享标志位轮询——那正是先前版本死锁风险的来源。

9. **不要用 `write_all` 发送**：`timeout()` 同时作用于写操作，`write_all` 失败时不报告已写字节数，会静默截断长报文。发送走 `VecDeque` 增量推进（见 `channel.rs` 的 `worker_main`）。

10. **开发全程不得运行本项目，不得触碰 COM3**：见 §0。开发阶段的验证一律用
    `cargo fmt && cargo check --all-targets && cargo test`，绝不用 `cargo run` 启动 GUI
    来"看一眼效果"——那会占用串口并可能打断 COM3 上正在运行的其他程序。

11. **日志长度上限只有一处实现**：`ComTool::trim_receive_logs()`，在 `receive_data()` 末尾无条件调用。四份镜像**各自**按同一字符预算裁剪——三种模式写入的镜像并不相同（HEX/文本模式写 `display_spans`，字节查看模式写 `ascii_spans`，`receive_text` 三种都写），它们之间没有下标对应关系，不要试图"按 `receive_text` 裁好再同步"。裁剪必须按**字符**边界（不是字节下标），并且要能把边界上的 `ColoredSpan` **从中间切开**且保留其 `AnsiStyle`。

12. **面板显隐有一条不变量**：`sidebar_open` / `show_receive_panel` / `show_chart_panel` / `show_send_panel` 至少要有一个为真（`visible_panel_count() >= 1`）。三处需要同时维护：`visible_panel_count()`、视图菜单的勾选框（唯一可见项置灰）、`config::restore_runtime_state` 里的兜底修复。新增面板时三处都要加。

13. **`ComTool::ui()` 的绘制顺序是有约束的**：菜单栏 → 侧边栏 → 中央区（`CentralPanel` 必须最后）；中央区内部是 接收区 → 图表 → 发送区。egui 的面板按添加顺序抢占空间，顺序错了菜单栏就不会横跨整幅宽度。

14. **主题偏好存自有枚举 `ThemeChoice`，不要存 `egui::ThemePreference`**：外部枚举的取值一旦被上游调整，旧存档就读不出来了。转换只发生在 `apply_theme()` 里，且仅在与当前生效值不同时才下发（避免每帧 `options_mut` 导致无谓重绘）。

15. **启动期的不变量修复放 `config::restore_runtime_state`**：存档是可以被手改的（也可能来自更早的版本），重绘间隔越界、面板全隐藏这类情况统一在那里兜底，并配单元测试——`restore_runtime_state` 不依赖 `eframe::CreationContext`，正是为了可测试才拆出来的。

16. **UI 冒烟测试用 `egui::__run_test_ui`**：`src/panels/mod.rs` 底部已有先例（菜单栏、设置/信息窗口、三种接收视图、侧边栏、图表面板）。它不需要窗口、不需要串口，能覆盖"面板嵌套顺序、控件 ID 冲突、布局调用"这类只有运行期才暴露的问题，但也**只能**证明"画得出来不 panic"，外观仍需人工确认。

17. **实时文件日志的写盘只能在后台线程**（`log_file.rs::worker_main`）：UI 线程不许对日志文件做任何 `write`。停机靠投 `Halt`，且 `Drop` 里必须用 **`try_send`**——队列满时 `send` 会阻塞，一个卡住的磁盘就能让"停止记录"把 UI 线程永久挂住。文件由 UI 线程 `File::create` 打开后再交给线程：这是刻意不同于 `open_connection()` 那种**乐观置位**写法的（串口那条「先乐观置位、失败再回滚」的假阳性缺陷还没修；网络侧靠同步 connect 绕开了它），不要改回去。

18. **接收镜像是"双写"的**：每次追加要么走 `push_receive_note()`（`receive_text` + 日志），要么 `push_str` 之后紧跟 `log_append()`（只进日志）。在 `receive_data` 里直接 `self.receive_text.push_str(...)` 会让日志文件悄悄少一段数据。另外 `&self.event_rx` 的借用活着时不能调用 `&mut self` 方法，所以提示类消息要先攒进 `Vec<String>` 再落笔。

19. **"记录中"的唯一事实来源是 `log.is_some()`**：不要再加 `bool` 标志位（两个字段表达同一件事迟早不一致），查询统一用 `is_logging()` / `log_path()` / `log_written_bytes()`。日志文件**绝不允许被静默覆盖**——同名时 `create_log_file()` 会追加 `_1`、`_2` 后缀，改成 `File::create` 就等于让用户丢日志。

20. **日志文件名模板来自用户输入，必须当不可信数据处理**：`chrono` 的 `DateTime::format` 遇到非法占位符会 **panic**，所以先用 `StrftimeItems` 逐项检查再格式化（`render_log_file_name()`）；路径分隔符等非法字符会被替换为 `_`，避免 `../` 之类把日志写出选定目录。

21. **图表吃的是原始字节，不是解码后的文本**：`receive_data()` 里对原始字节调 `chart.push_bytes()`，位置在 HEX / 字节查看 / 文本三个分支**之前**。所以切到 HEX 看字节时曲线照常推进（图表是并列的第四种解释，不是"文本视图的附属品"）。不要把它挪进某个模式分支——那样一换显示模式曲线就不动了，而且不报错，很难发现。

22. **图表宁可断线也不许编造数据**：
    - 某行字段数少于已建立的通道数 → 该组这些通道**不记样本**（曲线断开），**不许补 0**。补 0 会画出从未到达的数据，而曲线上看不出来。
    - 无换行的连续逗号流**不**按固定通道数循环分组。那样通道语义取决于猜出来的通道数，猜错就把数据画错位；明确不支持比猜错更好，所以半行超长时**整行丢弃**（剩余字节一直丢到换行符为止，否则被腰斩的尾部会变成一个假样本）。
    - 整行没有数值时不产出样本、**也不推进 x**，否则混进来的纯文本日志会在曲线上打出等距空洞。

23. **绘制点数必须封顶，且抽样要保留最后一个点**：`panels/chart.rs::points_for_drawing` 把每条曲线抽到 `MAX_DRAWN_POINTS`（4000）以内。样本上限是用户可以调到 10 万的设置项，而 `egui_plot` 每帧都要把点转成它自己的结构——不封顶的话一个设置项就能把界面拖死。抽样时**必须留下最后一个点**（那是最新数据），只按步长取点会把曲线末端切掉，看起来像"设备停了"。曲线配色 `CHANNEL_COLORS` 是**数据编码**（哪条线是哪个通道），刻意不随主题变化，并与主题色区分开。

24. **`egui_plot` 的版本号与 `egui` 并不同步**：`egui_plot 0.37` 才依赖 `egui ^0.36`，而 `egui_plot 0.36` 依赖的是 `egui ^0.35`。升级任何一个之前先核实真实依赖（`cargo tree -i egui`，或直接看本地 `registry/src/**/egui_plot-*/Cargo.toml`），不要凭版本号数字相同就断定兼容。
25. **`Cargo.toml` 已清理两个死依赖并固定 release 优化级别**：`tokio`（features = `["full"]`，零调用）与 `egui-chinese-font`（零调用，中文字体走 `main.rs::setup_fonts` 直接读系统字体）已移除；`[profile.release] opt-level = 2` 已添加，用于与上游保持一致并缩短 release 构建时间。当前活跃依赖：`chrono` / `rfd` / `serde` / `serialport` / `egui_plot`，以及 F2 新增的 **`encoding_rs`**（GBK / GB18030 的增量解码，Mozilla 维护、纯 Rust、无 C 依赖）。这也是 `FEATURE_GAP_AND_PLAN.md` §8 那句"当前依赖集是最终形态"必须加例外说明的原因。
26. **应用图标在 `src/app_icon.rs` 里用代码画出来**，而不是 `assets/icon.png`：256×256 圆角方形 + 数字脉冲波形，4×4 超采样做边缘抗锯齿，11 条单元测试覆盖尺寸、透明度、渐变、前景笔画与越界检查。这样没有资源文件丢失风险，图标行为也能在 `cargo test` 里被断言（`egui::__run_test_ui` 只能证明"画得出来不 panic"，而图标的几何正确性靠纯计算测试）。颜色与波形需要人工肉眼确认时，见 `src/app_icon.rs` 末尾的 `#[ignore]` ASCII 预览测试。
27. **时间戳前缀是"显示注解"，不是数据**：它插在**片段层**（`timestamped_spans()`），因为 `receive_text` 是由片段拼接而来的——在文本层再插一遍迟早会与片段镜像走偏。它因此会进导出文件与实时日志，但**绝不进** `chart.push_bytes()` 的那份原始字节（图表解析的必须是设备真正发来的内容，`receive::tests::timestamps_do_not_leak_into_the_chart` 盯着这一点）。
    - 行首状态**不要另设字段**：`at_receive_line_start()` 从 `receive_text` 尾部推导（空串或以 `\n` 结尾）。三种显示模式、`push_receive_note()`、`clear_receive()` 都在写这个镜像，任何平行字段都会在某个分支上与它不一致，而症状只是"少一个时间戳"，很难注意到。
    - 一批只取**一次** `Local::now()`：同一块数据里的多行本来就同时到达，逐行取时间会造出"每隔几毫秒来一行"的假象。
    - **没有内容就不加前缀**，判据是"有非空片段"而不是"片段数组非空"——文本模式收到半个多字节序列时片段是空的，此时加前缀等于凭空造出一行。
    - 行边界就是 `\n`，所以 HEX 视图（每批一行）下"每行"自然退化为"每批"，这是设计如此，不是缺陷。
    - 片段的落点必须与模式分支顺序一致：HEX 优先于字节查看，因此落点是 `!hex_view && ascii_view`。写成 `ascii_view` 会让"两个勾选框同时打开"时片段落进另一份镜像。界面层现在**禁止**这个组合（见第 28 条），所以这里的 `!hex_view` 是纵深防御：手改存档、或将来有调用方绕过 UI 直接置位时，落点仍与分支一致（`receive::tests::hex_takes_precedence_over_ascii_like_before` 锁住了它）。
28. **「HEX 接收」与「字节查看」互斥，而且规则只有一处实现**：`ComTool::resolve_display_mode()`（`receive.rs`）。界面两个入口（接收设置行、视图菜单的 `display_mode_checkboxes()`）与启动修复（`config::restore_runtime_state`）都调它。
    - **为什么必须互斥**：`receive_data()` 的三条模式分支里 HEX 优先，HEX 分支只写 `display_spans`；而「颜色」未勾时渲染的正是 `ascii_spans`。两者同时为真 → 字节视图**一片空白**（不是"两种格式混排"，是"什么都没有"）。
    - **参数是"本轮哪个刚被勾上"，不要改成无条件让 HEX 胜出**：那样用户在 HEX 模式下点「字节查看」会被当场撤销，表现为"点了没反应"——比空视图更难排查。本函数负责把点击落实下来（置上刚点的那项、关掉另一项），调用方不必先自己写回字段。
    - 启动时传 `(false, false)` 用于修复旧存档里的非法组合，方向按 `receive_data()` 的分支优先级（HEX 胜出），保证修好后是一份**非空**视图。
    - **不要在某个界面上就地写 `if hex { ascii = false; }`**：规则一分散，迟早出现"菜单里能同时勾上、主界面里不能"这种两边不一致。

29. **发送只有一条路**：UI 的「发送」按钮、Enter 键、定时发送**全部**走 `ComTool::send_data()`。它返回"是否真的把字节交给了通道"，但 UI 调用点**不需要**看返回值（失败原因已经写进接收区）；返回值的存在意义是让测试能断言"解析失败时一个字节都没出去"。
    - `send.rs` 是**纯逻辑**（不依赖 egui、不依赖串口），所以 HEX 解析、行尾追加、编码、历史浏览全部可以直接单测。新加发送侧逻辑请先问"能不能放进 `send.rs`"。
    - **解析失败与写失败都不清空输入框**。`self.clear_send_after_send` 的语义是"**确实发出去了**才清空"——写失败（通道断开、设备长时间不收数据）时数据根本没出去，清掉等于让用户重打一遍。
    - `parse_hex()` **绝不静默吞掉看不懂的字符**：奇数个十六进制位、非十六进制字符都返回带原因的 `Err`。静默跳过会让用户以为发出去的是别的东西。
    - 定时发送的计时基准是 **`Instant` 差值**，不是"每帧累加一个间隔"（帧率波动会漂移，而重绘间隔本身就是可配的 16~1000ms）。勾上的那一刻**只起步计时、不立刻发一包**；断开连接一律 `stop_repeat_send()`，且 `repeat_send` **不落盘**（重启后一定是关的）。

30. **要用 `ByteChannel` 替身测发送链路时，用 `communicationtrait::fake`**：`#[cfg(test)] mod fake` 提供 `FakeChannel` + `FakeHandle`（记录写入、可拒绝写入、可注入 `ChannelEvent`）。有了它，"发送成功 / 解析失败 / 写失败 / 对端报错"这些路径都能在 `cargo test` 里跑完，**不需要串口硬件、也不启动 GUI**。给 `ByteChannel` 加新实现时，这个替身同样是验证 `receive_data()` 分支的现成手段。

31. **界面文案一律走 `self.language.strings()`，不要硬编码中文**（F3 起）：
    - 新增文案要在 `src/strings.rs` 的 `strings!` 宏里**同时写中英两列**（`字段名: "中文", "English";`）——宏的形态决定了"只补一种语言"写不出来。
    - 带占位符的文案**不能**写成 `format!(s.字段名, 实参)`：`format!` 的格式串必须是字面量，传变量会报 `format argument must be a string literal`。用 `strings::fill(s.字段名, &[&实参])`。
    - 取文案用 `let s = self.language.strings();`——返回值是 `&'static`，不借用 `self`，所以 `&mut self` 的方法体里可以随手取。
    - **源码级守卫**：`strings::tests::ui_sources_do_not_hardcode_chinese` 会扫描 `sidebar.rs` 与 `panels/*`，出现硬编码中文就失败。唯一豁免是语言下拉里的「中文」（档位自名）。
    - **接收区 / 日志里的程序提示行不翻译**（"串口已关闭"、"已导出到: …"）。它们会同步写进实时日志与导出文件，随界面语言变化会让同一台设备产出的两份日志不可比、也不能被 grep 脚本处理。所以 `app.rs` / `receive.rs` / `log_file.rs` 里**保留**中文提示行，那是有意为之。
    - 领域类型上**不要**挂界面词汇（F3 就删掉了 `TimestampPrefix::label()`）：留着迟早有人在别处顺手用上，那几个词就再也不会被翻译。

32. **收发共用一档编码**（F2 起）：`text_encoding` 同时管接收解码与发送编码，界面上必须说明这一点——用户不会希望"收到的中文按 GBK 解出来了、发出去的却是 UTF-8"。
    - 解**必须走增量解码**（`encoding_rs::Decoder` 挂在 `AnsiDecoder` 上），不能"收到一批就 lossy 解一次"：后者把被截断的多字节序列当场换成替换符，跨包的中文就永远是乱码。`receive::tests` 里有一组跨批次喂 UTF-8 的测试专门盯着它。
    - **GBK 与 GB18030 在 `encoding_rs` 里共用解码器、编码器不同**，所以两档"接收结果相同"，差别只在发送时的可表示字符范围——界面文案必须讲清，否则用户看到两档"好像一样"会犯嘀咕。
    - **UTF-16LE 在 `encoding_rs` 里只支持解码**（它的 `encode()` 会把 UTF-8 字节原样退回），所以小端字节由 `encode_utf16()` + `to_le_bytes()` 手写，且**不加 BOM**。另外 UTF-16LE 不做 ANSI 转义解析（`ESC` 是 `1B 00` 两个字节，按字节找 `0x1B` 会把数据吃掉）。
    - 切换编码时要丢弃上一批残留的半个字符：`AnsiDecoder::sync_encoding()` 与 `config::restore_runtime_state()` 两处都做。

33. **建连接只有一条路**（F4 起）：UI 按钮与自动重连都走 `ComTool::open_connection()`，它按 `connection_kind` 建串口 / TCP / UDP。**但 `configure` → `open` → 接管事件通道这三步只有一处实现**——不要为某种传输另写一遍，漏了事件通道就是"连上了收不到数据"，漏了 `configure` 就是"参数怎么改都没用"。
    - 前置条件统一由 `connection_ready()` 判定（串口要选了端口，网络要填了目标），提示措辞由 `channel_descriptor()` 给。**前置条件不满足时什么都不做**，尤其不能乐观置位 `connected`。
    - 串口的 `open()` 只拉起后台线程，失败要等 `Failed` 事件——那是既有的 `connected` 假阳性缺陷（见 `HANDOVER.md` §5）。网络侧刻意反过来：`open()` 返回 `Ok` 就**确实连上了**（TCP 同步 `connect_timeout`，UDP 当场 bind）。

34. **`channel` 与 `channel_kind` 同生共死**（F4 起）：通道对象的复用规则是「**同类型就地复用、换类型拆掉重建**」，判据只有一处：`open_connection()` 开头那句 `channel.is_none() || channel_kind != Some(connection_kind)`；`teardown_channel()` 把两者一起清空。
    - 这条规则同时服务两件事：串口断线重连是"同一个句柄再 `open` 一次"；`app.rs` 里 20 多条发送链路测试靠的是"注入的替身通道不被拆掉"（`connected_app_as()` 同时写 `channel` 与 `channel_kind`，就是这条规则的显式表达）。
    - 把复用改回"每次新建"，那些测试会**静默地**从"对着替身"变成"对着一个真串口跑"——不会编译失败，只会开始连真设备。改这里务必重跑 `cargo test`。

35. **串口专属控件必须随连接类型分叉**（F4 起）：端口 / 波特率 / 数据位 / 停止位 / 校验 / 流控在 TCP / UDP 模式下**整段不画**（不是置灰——留着会让人以为对 TCP 也有影响）；「可用串口」栏同样隐藏，只读信息区换成「目标 + 本地端口」。
    - 连接类型单选在**连接中锁定**：切了之后界面显示的参数就与真正打开的那条通道对不上，而按钮此时是「断开」——点下去断的是**另一种**传输。悬停要给出一句解释。
    - 按钮文案与动作**必须同向**，并按类型分叉：串口「打开串口 / 关闭串口」、TCP「连接 / 断开」、UDP「绑定 / 断开」。（F4 修掉了一个从上游继承的缺陷：原来已连接时显示「打开串口」却执行关闭。）

36. **UDP 不许说"已连接"**（F4 起）：UDP 没有连接，说"已连接"会让用户以为对端在线、以为掉线会被发现。状态词用「已绑定 / 未绑定」（`status_bound` / `status_unbound`）。
    - `UdpChannel::is_active()` 的含义只是「本地 socket 已绑定」，注释里写明了这点。
    - **UDP 的 `Ok(0)` 不是 EOF**（零长度数据报是合法数据报）；**TCP 的 `Ok(0)` 才是对端关闭**，要发 `ChannelEvent::Closed`。两者语义相反，改任一侧时别看错另一侧。
    - UDP 绑的是 `0.0.0.0:local_port`（不是回环）。`local_port` 允许为 0、由系统分配，因此 `ByteChannel::local_addr()`（**带默认实现**的 trait 方法，只有 UDP 覆盖）把这个实际端口暴露给界面——用户必须知道设备该往哪回包。

37. **提示行里的名词要跟着传输类型走**（F4 起）：`ComTool::transport_noun()` 给出「串口 / TCP / UDP」。提示文本本身**不翻译**（见第 31 条），但 TCP 掉线写一句"串口连接已关闭"是把日志发给别人看时的误导。
    - `receive_data()` 的 `Closed` / `Failed` 两条分支都用它，取名词要在借用 `self.event_rx` 之前先算好。

38. **网络侧的可忽略错误不要塞进公共函数**（F4 起）：`communicationtrait::worker::is_transient` 只放三种**所有传输共有**的（`TimedOut` / `WouldBlock` / `Interrupted`）。某个传输特有的（Windows 上 connected UDP socket 把 ICMP port unreachable 报成 `ConnectionReset`）由该实现自己补——塞进公共函数会让串口那边也跟着忽略一个真故障。
    - 共用脚手架（`WorkerCommand` / `StateGuard` / `IDLE` / `ACTIVE`）同理：三条实现共有的硬约定只有一处，不要复制三份。

39. **打开端口不许动控制线**（F5 / N1 起）：`open_connection()` 成功后只把 `rts` / `dtr` 同步成 `false`，**不发任何 `SetLineState`**。理由是硬的：上电瞬间把 DTR 拉高会把相当多的开发板直接拽进复位，而用户只是想打开端口看看。`serialport` 在 Windows 上打开时本就把两根线都置为 Disable（`windows/dcb.rs`），我们不去改变这个事实。
    - 勾选框的语义是「**我们请求的电平**」，不是"引脚此刻的电平"——驱动只提供写、不提供读，界面看不到硬件把线拉到了哪里。措辞、注释、悬停三处都必须说这一点，否则迟早有人把它当测量值用。

40. **控制线失败不是掉线**（F5 / N1 起）：`ChannelEvent::Notice` 是**非致命**事件，`receive_data()` 里只往接收区写一行提示，**不动 `connected`、不停定时发送**。与 `Failed` / `Closed` 严格分开——拉不动一根控制线时数据其实照样在收发，判成掉线会让界面跳到"未连接"。
    - 新加"失败但不致命"的通道侧事件时照这个来：加 `Notice` 的**新用途**，而不是把 `Failed` 的语义放宽。

41. **发送路径只有一个收口**（F4 起、F5 加固）：任何新的发送入口（按钮、常用数据、将来的文件发送）都必须走 `payload_for()`（编码 + 行尾，唯一一处）→ `transmit()`（写通道 + 报错 + `tx_bytes` 计数，唯一一处）。**不许在回调里自己拼一遍 `channel.write()`**：漏一次就是"某条路径失败了不提示"或"计数对不上"，而且都是静默的。
    - `tx_bytes` 只在 `write` 返回 `Ok` **之后**累加，与「发送后清空输入框」同一条原则：确实发出去了才算数。

42. **常用数据满了不淘汰**（F5 / N2 起）：`SendPresets` 与 `SendHistory` 的容量策略**故意相反**——历史是自动攒的、淘汰最旧的；常用数据是用户亲手挑的，满了就拒绝新增（`can_add()` 是**唯一**判定处，界面置灰与 `add()` 都调它）。
    - 标签不落盘：由 `preset_label()` 从内容现推（取首行 + 按**字符**截断）。

43. **绘制入口要能被无头测试驱动**（F5 起）：新增的绘制逻辑优先抽成 `fn xxx(&mut self, ui: &mut Ui)`（`pub(crate)` 视需要），而不是内联进 `eframe::App::ui`。原因是硬的：`ui()` 需要一个 `eframe::Frame`，无头测试构造不出来，内联在里面的控件等于**一次也不会被 `cargo test` 画到**。
    - 参照 `render_send_presets()`：抽出来之后 `panels::tests` 能把它"空 / 有内容 / 已满 / 英文"四种状态各画一遍。
    - 与之配套的一条：**纯绘制不得产生副作用**。无头渲染不投递指针事件，`changed()` 恒为 false——测试就断言"画完之后通道里一条命令都没有"。`show_sidebar` 的烟雾测试现在是这么写的。

44. **接收区默认文字色必须取自主题，不许写死黑色**（2026-09-24 修）：三条渲染路径的默认色统一改用 `ui.visuals().text_color()`——`show_plain_receive()` 用它构造 `LayoutJob::simple`，`render_color_receive()` / `render_ascii_receive()` 把它传给 `build_layout()` / `build_ascii_layout()`。
    - **修的是什么**：三处原先都硬编码 `Color32::BLACK`。浅色主题下看不出来，深色主题下接收区就是**黑底黑字、整块日志不可见**。
    - **`layout_job_from()` 的 `unwrap_or(default_color)` 是这条约定的关键**：只有设备真的发了 ANSI 颜色（`span.style.fg.is_some()`）才覆盖默认色。显式着色是**数据编码**（设备说"这段是红的"），不该被主题改写——所以修默认色**不要动** `ASCII_CTRL_COLOR` / `ASCII_NONPRINT_COLOR` / `TIMESTAMP_COLOR` 那几个常量。
    - **灰色系的显式色要单独看**：`TIMESTAMP_COLOR`（117,117,117）与 `ASCII_NONPRINT_COLOR`（128,128,128）在深色背景下偏暗但尚可辨认，**属于可接受范围，未改**——但若哪天用户抱怨时间戳看不清，先怀疑这两个而不是默认色。
    - **为什么能测**：颜色落在 `LayoutJob.sections[].format.color` 上，可以直接断言。**冒烟测试对颜色一无所知——写死黑色它照样是绿的**，所以这条专门配了三条断言颜色的测试（`receive::tests::default_text_color_follows_theme`、`ascii_view_default_color_follows_theme`、`explicit_ansi_color_overrides_theme_default`）。
    - 测试写法上踩过一次坑：`build_ascii_layout()` 读的是 `ascii_spans`，**只在字节查看模式下才有内容**；在文本模式下断言它会拿到空 `sections` 而 panic。要测它必须先切到 `ascii_view`，并用 `assert!(!ascii_spans.is_empty())` 确认内容非空，否则测试是空转的。

45. **"下拉框选中哪一档"用独立字段记录，不要从值反推**（2026-09-24 起）：波特率下拉新增了「自定义…」档，它旁边跟着一个数字框；两处状态是 `baud_rate: u32` + `baud_custom: bool`，**后者只表示档位、绝不参与 `SerialConfig` 的组装**。
    - **为什么不用哨兵值**（如 `BAUD_CUSTOM = 0` 混在 `baud_rate` 里）：那个值迟早会被某个 `if` 漏判而提交给驱动。独立 bool 让"哪个数是真的"始终唯一。
    - **为什么不从"`baud_rate` 是否在 `BAUD_RATES` 里"推导档位**：用户选了自定义、又手输一个列表里**恰好有**的值（比如 9600），下拉会自己跳回 9600 那一档，**输入框当场消失**——他看着自己刚敲的数字把控件弄没了。这条是明文写在 `baud_custom` 字段文档里的。
    - **点击语义只有一份实现**：`sidebar::apply_baud_pick(baud_rate, baud_custom, BaudPick)`。绘制那段**只攒一个 `Option<BaudPick>`**，闭包结束后再调它——不在闭包里就地改字段（同约定 43 的道理：纯函数才测得到）。四条语义各配一条测试，见 `sidebar::tests`。
    - **种子值 `BAUD_CUSTOM_SEED`（250000）不是标记**：只在"点自定义且当前值在列表里"时替换，目的是省掉用户删字符的动作；若当前值**本来就不在**列表里（典型是从旧存档读回的 74880）必须原样保留。有一条测试专门钉住"种子值本身不在 `BAUD_RATES` 里"，否则这个"省事"是假的。
    - **老存档兼容**：`baud_custom` 是新字段，旧存档没有该键，靠容器级 `#[serde(default)]` 落到 `false`（即"列表档"）。`config::tests::archive_missing_new_fields_still_loads` 里有断言，别把它删了。

46. **长内容窗口必须"限高 + 自带滚动区"，两件事缺一不可**（2026-09-24 加）：设置窗口的项数已经超过一屏，只在窗口上加滚动是不够的。
    - **限高**：`egui::Window` 默认按内容无限长高（`auto_sized` 语义），底部控件会被顶出屏幕，而且**没有滚动条够得着**——`Window` 的默认滚动状态是 `ScrollArea::neither()`。所以先给内容区一个 `max_height` 上限，超出部分才有"落在哪"这一说。设置窗口用的是 `SETTINGS_MAX_HEIGHT`（720）；窗口本身仍 `resizable(true)`，手动拉高那条路径不受该默认值限制。
    - **滚动区**：内容整体套 `egui::ScrollArea::vertical()`。**宽度必须 `auto_shrink([false, true])`**——纵向滚动条默认挤占宽度，若让滚动区随内容宽度收缩，`ui.separator()` 这类填满宽度的控件会跟着变窄，滚动条一出现/消失整排控件就左右抖。
    - **为什么不用 `Window::vscroll()`**：窗口确实有这个开关，但滚动条会长在标题栏下方、跟随窗口 frame 的圆角与内边距，视觉细节受上游实现变动影响；只要"内容可滚"就用内容级滚动区，更稳定。
    - **id 必须显式**（`SCROLL_ID = "settings_scroll"`）：默认 id 由调用栈位置推导，位置一变就等于换了个身份，之前记的滚动位置会神秘丢失；显式 id 也让将来「打开设置时复位到顶部」之类的需求有稳定名字可指。
    - **内容体要抽成 `pub(crate) fn xxx(&mut self, ui: &mut Ui)`**（这里是 `settings_body`）：`show_settings_window` 里套着 `Window::show` 的闭包，无头测试只能连窗口一起画——窗口画得出来不代表内容体本身没问题。抽出来后 `panels::tests::settings_body_renders_inside_a_short_viewport` 能在**很矮的可视区**里单独画它。约定 43 的同一道理。
    - **测"需要滚动"而不是"画得出来"**：只断言"没 panic"等于普通冒烟测试，把滚动区删掉照样绿。egui 在 `ScrollAreaOutput` 上给的判据是文档原文——*"If this [`content_size`] is larger than [`inner_rect`], then there was need for scrolling."* 再加一条"内容真实绘制高度 > 可视高度"（用闭包返回 `ui.min_rect().height()`）作为**防空转**断言。这两条在把内容体换成空壳时会立刻变红（已验证）。

47. **全局字体档位：存"档位"不存"像素"，且每帧从固定基线重算**（2026-09-24 加）：设置窗口新增「字体大小」下拉（`UiFontScale::{Small, Normal, Large, ExtraLarge}`，系数 0.85 / 1.0 / 1.15 / 1.3）。
    - **存枚举不存像素值**：像素是实现细节，存档里存 `13.0` 这种数字的话，将来换字号基线会让老存档"记住"一个错误的档位；存 `Large` 则语义稳定、显示层随便改。`#[default]` 必须落在 `Normal`（系数 1.0）——既有存档没有这个字段，反序列化要"和加功能之前一模一样"，不能让老用户一升级就发现字号变了（`config::tests::archive_missing_new_fields_still_loads` 末尾那两条断言就是钉这个的）。
    - **每帧从 `FONT_SIZE_BASELINE` 重算，绝不乘上一帧的值**：写成"在 `style.text_styles[x].size` 上乘系数"的话，`ui()` 每帧调一次就是指数放大，几秒钟字号就能到几万像素、控件全飞出屏幕。固定基线表是**幂等**的唯一写法，也是"标准档等于 egui 默认"这一承诺的唯一真相。
    - **`all_styles_mut` 而不是 `style_mut_of`**：深浅两份 `Style` 都要改，否则切主题时字号会跳回默认。用 `entry(..).or_default()` 而不是整个覆盖 `text_styles`，是为了让表里没登记的、egui 新增的 `TextStyle` 至少保留上游默认值。注意 `egui::TextStyle` **不是 `Copy`**（有 `Name(String)` 变体），当 BTreeMap 的键要 `.clone()`。
    - **"全局"的诚实问题**：接收区三条渲染路径 + 发送框原先都硬编码 `FontId::monospace(14.0)`。只改正文样式的话，用户调到"特大"后正文按钮都变大、**日志和发送框纹丝不动**——"全局"就名不副实。解法是 `receive::monospace_font_id(ui, fallback)`：从 `style.text_styles[TextStyle::Monospace]` 取实际字号，档位一改自动跟随，**不需要在每条调用链上多传参数**。发送框同样调它。
    - **`build_layout` / `build_ascii_layout` 的字体是必填参数**，不要改成 `Option` 加内部兜底：那样渲染路径哪天忘了传也会静默用一个默认字号，档位在这个视图上失效且无人察觉。
    - **测三条**（都做了"故意改坏→确认变红→还原"的空转验证）：
      ① `app::tests::font_scale_baseline_matches_egui_defaults` 把 `FONT_SIZE_BASELINE` 与 `egui::Style::default().text_styles` **双向**比对——正向防"上游改默认字号我们没跟"，反向防"egui 默认有的样式我们漏登记"（漏了的那档永不跟随档位）。
      ② `font_scale_is_idempotent_across_frames` **除了首末相等，还必须断言绝对值**（`基线 × 系数`）。只断言首末相等是**空转的**：实测"先连乘、再被别的赋值盖掉"这种写法末值照样稳定、测试照样绿；加绝对值后立刻抓到（`17.1925` vs 期望 `14.95`）。
      ③ `panels::tests::font_scale_dropdown_is_wired_to_the_field` 读 `FullOutput.shapes` 里**按落笔顺序**的文字，取「字体大小」标签**紧跟着的下一段**作为下拉选中文案。**不能**改成"整段文字里包含当前档位名"——`font_scale_preview`（「预览：{}」）本来就会写进当前档位名，那样把 `selected_text` 写死也测不出来（实测确实测不出来）。读输出要用 `Context::run_ui` 而不是 `__run_test_ui`（后者 `drop_without_applying_deltas()` 把输出丢了，什么也观察不到），且拿到 `FullOutput` 后要 `textures_delta.clear()`，否则 drop 时会 panic。
    - **无头环境测不了"点一下"**：不投递指针事件，`changed()` 恒 false。写"模拟点击再断言字段变了"会得到**恒真**的空转测试（事件没送到，字段自然没变，断言永远成立）。只能测"字段 → 显示"这条已接通的连线。
