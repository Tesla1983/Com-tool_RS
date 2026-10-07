# comtool-rs 开发参考文档

> 本项目是一个基于 `eframe`/`egui` 的串口调试工具。本文档描述当前项目的代码组织方式、功能开发的完整流程，以及各层职责划分，作为新增功能的参考。

---

## 0. ⛔ 运行禁令（最高优先级，动手前先读）

**未经项目所有者在本轮对话中逐次、明确批准，禁止以任何形式运行本项目。串口按端口分两类：
COM3 受「长测门闩」保护，COM4 是 AI 功能测试放行端口（但必须置 DTR 才有日志）。**

- **禁止运行**：`cargo run`（含 `--release`）、直接执行 `target/**/comtool-rs.exe`、
  `cargo install` 后运行、`start`/`Start-Process`/后台任务/计划任务等一切拉起本程序的方式；
  也包括任何会间接启动本程序本体的脚本、测试或工具调用。
- **COM3 —— 看长测门闩**（用户 2026-09-24 修订）：门闩存在（有长测在抓日志）⇒ 一律不碰，
  不得打开、连接、读写、配置、探测（`mode COM3`、`echo > COM3`、`pyserial`、
  `[System.IO.Ports.SerialPort]`、第三方串口助手等），不得停止/杀掉/禁用任何可能占用它的
  进程、服务或驱动；门闩不存在 ⇒ COM3 可用（可烧录、测试、复位单片机）。
  无论门闩在不在，都不得把 COM3 写进默认值、配置、测试或脚本。对 COM3 只能"看"
  （不打开端口的只读枚举，且需先说明理由），不能"碰"。
- **COM4 —— AI 功能测试放行端口**（用户 2026-09-26 修订）：AI 要验证生成软件的功能时可以用，
  **不受长测门闩影响**；**前提是必须置 DTR**——GUI 里勾 DTR 控制线，脚本里 `ser.dtr = True`
  或 `DtrEnable = true`。**没置 DTR 收不到日志不是程序的问题**，先查这一项。
  一条命令同时点名 COM3 与 COM4 时 COM3 优先。
- **默认允许**：`cargo fmt` / `cargo check --all-targets` / `cargo test` / `cargo clippy` /
  `cargo build` 等不启动 GUI、不触碰串口的命令。**385** 条测试全部不需要串口硬件，
  所有验收一律以这些命令为准。
- **门禁口径（四项全绿才算过）**：`cargo fmt --check` 干净 · `cargo check --all-targets`
  **0 warning** · `cargo test` **385 passed + 1 ignored** · `cargo clippy --all-targets`
  **0 warning**。最后一项 2026-09-26 起从"0 新增"收紧为 **0 warning**——原先的口径等于
  默许 8 条历史欠账永久存在（逐条见 `HISTORY_2026.md` §3.24）。
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
| 自测 | `python3 .workbuddy/hooks/test_guard.py`（**52 条**断言；门闩开/关两档分别断言，门闩那档用临时配置目录造门闩、**不碰用户真实门闩**；不启动程序、不碰串口）。**已在版本控制中** |
| 批准 | `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（**用户本人在 agent 之外执行**；`0` 撤销） |

- 批准文件 `<配置目录>/combot-run-approval` 第一行是过期 Unix 秒，不存在/过期/损坏 → 全部拒绝；
  **批准只放行"运行本项目"，与串口判定互不影响**（COM3 看门闩，COM4 是放行端口）。
- **COM4 的放行只跳过串口拦截，不下 `allow` 决策**：守卫对该情形输出一条带 DTR 提醒的
  `systemMessage` 后放行到正常权限流程，用户自己的确认提示照旧——否则任何"顺带提到 COM4
  的命令"都会被自动批准，那是过头了。
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
| `src/sidebar.rs` | 右侧边栏；返回 `SidebarAction` 而不是在绘制过程中改状态；连接类型单选 + 「串口参数 / 网络参数」双分支（F4）；F5 起含波特率自由输入与控制线 RTS/DTR 一行（勾选框在通道未就绪时置灰，并在下面常显一行原因，见 §6 约定 60） |
| `src/panels/mod.rs` | 面板模块声明 + 无头 UI 冒烟测试（含英文界面、已连接侧边栏、常用数据行） |
| `src/panels/menu_bar.rs` | 顶部菜单栏（文件 / 设置 / 视图 / 文本——**没有「发送」**）；返回 `MenuAction`；各档单选（主题 / 时间戳 / 编码 / 发送格式与行尾）共用一处定义；`apply_always_on_top` 挂在 `show_menu_bar` 上（理由见 §6 约定 59） |
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
| 日志长度上限（FIFO 淘汰） | `max_log_chars`；`receive_at_bottom` / `receive_trim_budget`（运行时，决定这一帧能裁多少，见 §6 第 58 条） | `trim_receive_logs()` / `is_at_bottom()` / `trim_budget_per_frame()`（`receive.rs`） | 设置窗口「日志长度上限」 |
| 自动滚动（含"滚轮往上就停住"） | `auto_scroll`；`receive_at_bottom`（运行时） | 三个接收视图的 `stick_to_bottom` 参数；**解粘是 egui 自己做的**，本项目负责"读历史时不裁旧内容"（见 §6 第 58 条） | 设置窗口 / 视图菜单 |
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

10. **开发全程不得运行本项目；串口按 §0 分端口对待**：COM3 看长测门闩（门闩在就别碰），
    COM4 是 AI 功能测试放行端口、但**必须置 DTR 才有日志**。开发阶段的验证一律用
    `cargo fmt && cargo check --all-targets && cargo test`，绝不用 `cargo run` 启动 GUI
    来"看一眼效果"——那会占用串口并可能打断门闩期间 COM3 上正在运行的其他程序。
    真要实机看效果，等用户批准运行，并在 COM4 上**先勾 DTR** 再判断收发是否正常。
    ⭐ **能用脚本验的就别等放行**：COM4 上自带一个持续数据源，用临时 `#[test]` 驱动
    `SerialChannel` 就能验到真实链路（**跑完删探针**并用 `git diff --stat src/` 确认零残留）——
    2026-10-07 这样把 `HANDOVER.md` 第 44 / 45 项从"待人工"变成了"已实证"。
    实测数据、步骤与陷阱见 **`TEST_PORT_COM4.md`**（⚠️ 它是 CDC ACM 类，**任何波特率都返回
    OK**，用它在 N5 非标波特率上得出的结论是假的）。

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

48. **底部状态栏：面板画在 `CentralPanel` 之前；圆点是自己画的实心圆，不是 emoji 字符**（2026-09-24 加）：主界面底部常驻一条状态栏（`status_bar.rs`），显示一颗圆点 + 一句话。
    - **绘制时序是硬约束**：`Panel::bottom("status_bar")` 必须在 `egui::CentralPanel` **之前**画。egui 的面板"先画先占位"，后画的只能拿到剩余空间——放在中央区之后，状态栏只横跨中央区剩余宽度，左右露出缝隙，看起来像一条没对齐的横条。与顶部菜单栏同理（`ui()` 里 ① 菜单栏 / ② 侧边栏 / ③ 状态栏 / ④ 中央区，编号即顺序）。
    - **判据是纯函数**（`status_bar::connection_indicator(connected, kind, s)`）返回 `ConnectionIndicator { is_on, label }`，绘制只负责画。这样"UDP 该显示已绑定"这类规则可以直接断言，不必先画一遍再猜像素（同约定 43 的道理）。`is_on` 存的是**语义**（亮/灭）而非颜色值，颜色由 `ConnectionIndicator::color()` 单独给出——将来换配色、或加第三态时，判据这层不用动。
    - **必须沿用侧边栏的四态**：`ConnectionKind::Udp` 下 `connected == true` 只表示**套接字绑定成功**，不是"对端在线"。显示成"已连接"会让用户以为掉线会被发现——侧边栏早就为此分了 `status_bound` / `status_unbound` 两档（见 `sidebar.rs`）。状态栏若只做两态，同一个窗口里两处指示会互相打脸。有一条测试（`status_bar_agrees_with_the_sidebar_on_udp`）把两者的分叉绑在一起。
    - **⛔ 圆点绝不要改回 emoji 字符**（这是本条最容易踩的坑，初版就踩过）：初版用 🟢(U+1F7E2)/🔴(U+1F534)，结果界面上是**两个大小不一、且都是灰黑色**的圆。根因不在字体选错，而在 egui 的文本管线：`epaint` 光栅化字形走的是 `skrifa` 的 `outline_glyphs()`（见 `epaint-0.36.2/src/text/font.rs::allocate_glyph_uncached`），**只取矢量轮廓**再填充成当前文字色；彩色 emoji 的颜色来自 `COLR`/`CPAL`（分层）或 `CBDT`/`CBLC`（位图）表，这些表 egui **从不读取**。实测两个候选字体的表结构：`seguiemj.ttf` 有 `COLR`+`CPAL`，但同一码点也有普通 `glyf` 轮廓（就是被当成单色圆画出来的那个）；`seguisym.ttf` 连 `COLR` 都没有。**所以换任何字体都得不到彩色——这是架构限制，不是配置问题。**
    - **正确做法**：用 `ui.painter().circle_filled(center, radius, color)` 画一个 `Color32` 实心圆。尺寸取 `ui.text_style_height(&TextStyle::Body) * DOT_SIZE_RATIO`（0.72，必须 < 1，否则满行高的圆会顶到相邻行），圆心取分配矩形的 `rect.center()` 而不是左上角——`allocate_exact_size` 按行高分配高度、比直径大，不居中会顶在上边缘。这样同时解决了三件事：颜色正确、两态大小一致（不再受 emoji 字形自带 advance 摆布）、垂直对齐（按行高居中而非跟字体基线漂移）。**副作用是状态栏对字体回退链的依赖归零**，`main.rs::setup_fonts` 里那档 emoji 字体已随之删除。
    - **颜色方向要断言，不能只断言"两色不同"**：只比"绿≠红"会被"亮=红、灭=绿"这种反色实现骗过。`the_dot_changes_colour_with_the_state` 额外断言 `green.g() > green.r() && green.g() > green.b()`（亮方确实偏绿）与对称的红方条件。
    - **绘制测试读 `Shape::Circle` 而不是 `Shape::Text`**：圆点是几何图形不是字符。`status_dot_is_painted_as_a_filled_circle_in_the_right_colour` 从 `FullOutput.shapes` 里取 `(radius, fill)`，断言**恰好一枚**颜色等于 `indicator.color()` 的圆——这同时拦住"没画出来"和"颜色写死"两种回归（两条都做过空转验证：把绘制注释掉 → 2 条红；把颜色改成 `Color32::GRAY` → 1 条红）。
    - **文案是短词**（"已连接"/"未连接"），与侧边栏的 `"状态: 已连接"` 分开写：侧边栏是逐行读数（前缀有助于扫读），状态栏是常驻指示（前缀是噪音）。

49. **请用户"目视 / 目测审查"之前，必须先把 exe 编译出来**（2026-09-24 用户明确要求，属交付前置条件）：凡是交付物包含 **界面外观**（新增面板 / 控件 / 配色 / 字体 / 图标），而最终判定"好不好、对不对"只能靠人眼时，**必须在提出目视请求之前跑 `cargo build --release`**，并核对 exe 的时间戳晚于最后一次源码改动。
    - **为什么单列一条**：`cargo check` / `cargo test` / `cargo clippy` **都不产出可执行文件**。只跑这三样（再加 `cargo fmt`）的时候，`target/release/comtool-rs.exe` 要么是上一轮的陈旧产物、要么根本不存在，用户双击到的是**不含本次改动**的程序——他会看着旧界面找新功能，然后得出"没做出来"的结论。这不是用户看错，是交付没做完整。
    - **判定"编译过没有"的唯一依据是时间戳比对**，不是"我跑过 cargo 命令了"：
      ```
      git log -1 --format=%cd --date=iso          # HEAD 提交时间
      ls -l --time-style=full-iso src/*.rs         # 最后改动源码的时间
      ls -l --time-style=full-iso target/release/ | grep -i '\.exe$'   # exe 时间戳
      ```
      要求：**exe 时间戳 ≥ 最近一次源码改动的时间**（实际以上一提交为界，见下）。三者的先后要对得上。
    - **正常交付次序**（与约定 10 / §0 的运行禁令不冲突——`build` 在默认允许清单里）：
      1. 改代码 → `cargo fmt` → `check --all-targets` → `test` → `clippy`；
      2. **`cargo build --release`**（这一步容易漏，因为它"看起来"和上面那些是同类命令）；
      3. `git add` → 用消息文件 `git commit -F`；
      4. **再确认一次 exe 时间戳在 commit 之后或同批**（改完代码又改了文档不影响 exe，但若在 commit 后又动过 `src/`，要重新 build）；
      5. 此时才向用户说"可以运行 `target/release/comtool-rs.exe` 目视检查了"，并给出**具体的观察点**（看哪个控件、期望看到什么）。
    - **禁止的两种说法**："应该能用了，你自己 build 一下"（把 build 推给用户），以及"跑过测试了，可以看了"（测试不产出 exe）。两者都是把没做完的交付说成做完了。
    - **exe 不能进版本控制**（`target/` 已在 `.gitignore` 里），所以它**每次都要现场重建**，不会随 clone / checkout 带过来。换机器或清过 `target/` 之后，第一次交付界面前同样要先 build。
    - **用户明确要求过这条**：本约定不是我的自拟规范，是用户在 2026-09-24 连续两次指出后的固化（第一次："先编译再让我目视检查啊，你没编译吧？"）。此后凡涉及界面交付，`build --release` 与 `test` 同级必跑。

50. **加一种语言 = 加一列，不改代码**（2026-09-24 多语种扩展起）：`src/strings.rs` 的表头 `langs: ZH, EN, ZH_HANT, JA, ES, IT, PT, ID, FR, DE, KO, VI, TR, RU, PL;` 是**唯一**的语言清单，每条文案后按同顺序跟 15 个字面量。加语言只做三件事：① 表头插一个语言标识；② **每一条**补上该列（漏一条编译期就报 `compile_error!`——宏会数每行的列数）；③ 在 `app.rs` 的 `Language` 枚举末尾追加变体 + `Language::ALL` + `endonym()`。**不要**在 `strings.rs` 的测试里写死语言数量，都从 `Language::ALL` / `LANGS` / `ALL` 里取。
    - **每加一种新文字系统，必须同时补一个"字形归属"检测器**。`columns_have_no_foreign_script` 的检测器集合原本只有假名/谚文/泰文/中日韩，**没有西里尔**——所以整列 `ru` 抄成波兰语（拉丁）时全部检测器不命中，静默通过。`has_cyrillic` 补上这个洞。
    - **"这一列是不是这种文字"要从数据自证，不能靠写死的字表**。`third_batch_columns_use_their_own_alphabets` 的判据：`ru` 列至少 100 条含西里尔（实测 195/199）、`pl` 列至少 40 条含**波兰独有**字母 `ą ć ę ł ń ś ż`（实测 101/199）。**不能用完整波兰字母表**——`ó`/`ź` 也出现在葡萄牙语（`não`）与西班牙语（`Versión`）里，拿它们当判据，整列换成西语也会命中。判据必须是**差异集**。
    - **守卫必须做"空转验证"，而且要能看出"测试压根没跑"**（2026-09-24 本轮踩了两个假绿坑，务必记住）：① 本项目**没有 lib target**，`cargo test --lib <名字>` 报 `no library targets found`——**测试一条也没跑**；② `-- --exact <裸名字>` 匹配不到模块限定路径（要写 `strings::tests::xxx`），输出 `running 0 tests; N filtered out` 然后 `test result: ok`——**同样是假绿**。两者都因为"红判据只找 `FAILED` / `panicked at`"而被吞掉。探针必须把**执行数 0** 也判为红。
    - **宏为什么写成"边走边剥"**：`macro_rules!` 既**不能拼接 token**（写不出 `(m m)` → `(m m m)`），也**不能在匹配时做算术**（`[$n - 1]` 不成立）。所以"取第 k 列"不能靠计数器，只能每处理一种语言就从每行字面量里**取走第一个**、把尾巴交给下一种（见 `@emit_langs` 的两个臂）。改宏时先想清楚这一点，否则会绕回那两个死胡同。
    - **枚举变体只能往后追加**：`Language` 派生 `Serialize` / `Deserialize`，按**变体名**存取。插在中间不影响正确性（名字不变），但让 diff 难读；删改已有变体名会让老存档读不出来。`Default` 仍是 `Zh`。
    - **字段名索引的守卫测试优先于"比数值"**：`template_placeholder_counts_are_pinned` 按**字段名**在 `ALL` 里查该条的各语言列，逐列核 `{}` 个数；不要退化成"只比中文和英文"。同理 `columns_have_no_foreign_script` 按语言分别判定允许的文字系统（日语列允许汉字、拉丁列出现汉字才是错），不要写成"一列一条规则"。
    - **守卫测试必须做"空转验证"**：新写的语言列守卫，先故意把被测的东西改坏（删一个 `{}` / 把某列粘错 / 把繁整列抄成简），确认**测试变红**，再还原。实测中"繁体列重合率 ≥50%"那条就是空转的假前提（真实重合只有 22/199），已改成"繁体列里有多少个简体列没有的汉字"。
    - **字体**：新增语言若用了微软雅黑覆盖不到的字符（拉丁扩展 / 谚文 / 泰文…），要在 `main.rs::setup_fonts` 的**两个**族（Proportional **和** Monospace）都追加对应字体——只加一个族会漏掉接收区。越南语靠 `tahoma.ttf`、韩语靠 `malgun.ttf`，两者实测覆盖完整；`setup_fonts` 的注释里写了实测结论，改前先核对。字体链的完整性由 `every_vietnamese_character_has_a_glyph_in_the_font_chain` 守着（遍历实际字符集，不是抽样）。
    - **彩色小图元不要指望 emoji 字符**：见约定 48——egui 只取矢量轮廓、不读 `COLR` 表，任何字体都出不来彩色，自己画。

51. **写侧卡住不要用"加个写超时"修——要么非阻塞写，要么把判定提前到调用线程**（2026-09-24 修 TCP 那个既有缺陷时实测得出，是约定 9 的加强版）：
    - **实测反例**：Windows 上给阻塞 socket 加 `SO_SNDTIMEO`（或 `set_write_timeout`）后，**超时的 `send` 可能已经把数据交给内核并发出了**，而调用方只拿到 `WSAETIMEDOUT`、**拿不到已写字节数** ⇒ 上层按"这片没写出去"重发 ⇒ 同一片数据在线上出现多遍。实测一片 64 KB、超时 10 ms、对端停读 2 秒：**同一片出现 126 遍**（片 42 的词数 1032192 = 126×8192）。这与约定 9 是同一个成因（`write_all` / 超时不报告已写字节数），只是后果更脏——约定 9 是**截断**，这里是**重复**。
    - **正解：非阻塞写**。`Ok(n)` 的 `n` 就是**准确**的已写字节数，`WouldBlock`（`ErrorKind::WouldBlock`）单独一档表示"一个字节都没写出去"，既不阻塞也不重发。落点在 `net_impl/tcp.rs::open` 的 `stream.set_nonblocking(true)`（另加 `set_nodelay(true)`）。
    - ⚠️ **代价要一起承担**：阻塞性是**每 fd** 的，改成非阻塞后**读侧的 `set_read_timeout` 同时失效**，循环必须自己节拍——见 `IDLE_SLEEP = 1 ms` + `idle` 标记（只在"这一轮一个字节都没动"时才 sleep，有活干时不拖慢吞吐）。这个节拍同时也是 `close()` 的最大响应延迟，注释里写明了这双重身份。
    - **反面也要判**：不是所有传输都需要非阻塞。**UDP 就是刻意保持阻塞的**——实测向一个从不 `recv` 的对端发 4000 × 60 KB 是 **0 次失败 / 最坏 0.1 ms**（TCP 对照组挂了 **2 分 05 秒**）⇒ UDP 没有"对端停读 ⇒ worker 卡住"这条路径，改非阻塞只会换来一个不必要的节拍。**判据是"该传输是否存在对端能掐死写侧的路径"，不是"统一风格"**（UDP 仍未测的那条残余风险见 `HANDOVER.md` §5）。
    - **背压的判定必须在调用线程**：队列满若在后台线程判，`write()` 就一律返回 `Ok`，调用方（界面）会记账、会清空输入框——两边说法不一致。落点是 `worker::submit`（**先预约额度 → 再投递 → 投递失败退还**）+ `worker::retire_finished_chunk`（**出队与归还额度必须一起做**），三个通道共用，worker 只 `push_back`。`open()` 里必须 `ledger.reset()`（通道对象会就地复用）。

52. **一个"显示模式"只能有一个判定，且"写数据"与"画界面"必须读同一个**（2026-09-24 用户目视发现接收区空白后得出）：

    - **反面形态**：同一个概念被三处各判一遍——「片段怎么解释」与「写进哪份镜像」看两个字段，
      「渲染画哪一份」却让第三个字段排在最前。前两处从未看过那个字段，于是某人加上第三个开关后：
      数据写进 A 镜像、界面画 B 镜像（B **从未被写过**）⇒ 屏幕**一片空白**，用户描述为"内容丢了"。
      落点在 `receive.rs` 与中央区的渲染分支。
    - **这类缺陷的隐蔽之处**：它长得像"少写了一份镜像"，其实是"写的那份和画的那份不是同一份"——
      所有"镜像写对了没有"的既有测试**全绿**，因为它们测的正是没坏的那一半；而"画的是哪一份"
      当时没有任何自动化判据。**修完之后要能说出"是哪一条测试抓住了它"，否则不算修完。**
    - **正解**：把判定抽成一个函数（`ComTool::display_mode() -> DisplayMode`），**片段构造 /
      镜像落点 / 渲染分发三处都调它**；渲染分发本身也要从 `eframe::App::ui()` 里抽出来
      （`render_receive_area()`）——否则测试够不着它（约定 3），"画的是哪一份"就只剩人工目视一条路。
    - **判据怎么才算数**：① 穷举**全部开关组合**（含非法组合），断言"当前模式下会被渲染的那份
      镜像非空"——这条覆盖的是**落点**；② 更关键的一条：把几份镜像摆上**互不相同**的标记，
      **真渲染一次、从画出来的字反推画的是哪一份**——只有这条覆盖**渲染分支**。两条都要有。
    - **互斥的方向**：新勾上的那项**胜出**（"最后被勾上的赢"），不要写成"某一项永远赢"——
      后者表现为"点了没反应"，比空视图更难排查（同约定 4 的三种禁令）。启动修复则相反：
      按"更强的视图胜出"把旧存档修干净，并且**不能把合法组合顺手关掉**（本项目里 `HEX + 颜色`
      是合法的，改动它等于替用户做决定）。
    - ⚠️ **顺带记一条推断纪律**：判断"两个视图同时勾上会怎样"时，**必须同时读渲染分支与写镜像的
      那条分支**。只看渲染分支会得出"只是被另一视图盖住、不丢内容"的**相反**结论——这正是本项目
      当天阶段报告写错的那一处（见 `HISTORY_2026.md` §3.16：报告说不丢内容，用户实测是空白）。

53. **界面的"表面色"一律取自主题，绝不写死；面板背景自己画，不要指望窗口底色**（2026-09-24 用户目视发现「浅色主题下状态栏没变色」后得出）：
    - **症状**：切到浅色主题后，整条底部状态栏仍是一条深色横带，而且那行字几乎看不见。
    - **根因不是配色表写错，而是那块面板根本没画背景**：`egui::Frame::NONE` 的 `fill` 是
      `Color32::TRANSPARENT`（常量定义在 `egui-0.36.2/src/containers/frame.rs`），于是透出来的
      是 `eframe::App::clear_color` 的默认值——**写死的**深灰 `rgba(12,12,12,180)`，那个参数
      的名字就叫 `_visuals`，**拿得到主题却不用**（`eframe-0.36.2/src/epi.rs`）。深色主题下
      它 (12,12,12) 与 `panel_fill` gray(27) 几乎一样，所以长期没暴露；浅色主题下 `panel_fill`
      是 gray(248)、而文字色是 gray(80) ⇒ **深字压在近黑底上**，"既没变色、又像没内容"。
    - **正解**：① 面板背景用 **egui 自己的面板 frame** —— `Frame::side_top_panel(ui.style())`
      （侧边 / 顶 / 底）或 `Frame::central_panel(ui.style())`，只覆盖我们确实要改的
      `inner_margin` 之类；它正是 `Panel::resolve_frame` 在"调用方没给 frame"时会选的那一份，
      于是底色的唯一来源留在 egui 那边，将来它改了我们也跟着改。② 顺手重写
      `eframe::App::clear_color()` 返回 `visuals.panel_fill.to_normalized_gamma_f32()`，作为
      "没人画背景的区域"的兜底。**不要**写 `.fill(Color32::from_gray(27))` 这类常量：那只是把
      同一个 bug 换个地方藏（非空转注入的 G2/G4 就是这两种写法，浅/深各露一次馅）。
    - ⚠️ **判据怎么写才不空转**：断言**等于该主题的表面色**（`Visuals::{light,dark}()
      .panel_fill`），**不要**只断言"两套主题颜色不同"——后者会被上面那种写死常量骗过。取面板
      背景的办法：无头渲染后在形状列表里找**横跨整幅宽、下边缘贴住屏幕底边**的那个
      `Shape::Rect`，读它的 `fill`。注意 `Frame::paint` **无论如何都会 emit 这个矩形**，哪怕
      fill 是透明的 ⇒ "没画背景"与"画了个透明背景"在形状列表里长得一样，**只能靠读 `fill`
      分辨**（这正是它躲过全部自动化测试的原因）。
    - **为什么旧测试全绿**：它们问的是圆点数 / 文字内容 / 文字位置 / 下留白——**没有一条问
      "这块面板自己的背景是什么颜色"**。"屏幕上有字"与"底色对"是两件事，UI 判据必须落在
      **被测机制本身**（约定 3 的同族教训）。
    - **适用面**：任何自绘表面（面板、状态带、提示条、图表背景）都按这条办。曲线配色
      （`CHANNEL_COLORS`）是例外且**必须**是例外——它编码的是"第几条通道"，与主题无关。

54. **"让它更像一个按钮"要落在量上：尺寸全部显式、自绘，且底色报"动作"不报"状态"**（2026-09-24 用户要求重做「打开串口」按钮，同日按目视反馈再修订一轮）：
    - **起点**：那个按钮原先只有一行——`egui::Button::new(label).selected(is_connected)`，
      宽度 / 高度 / 填充 / 描边 / 圆角**一个都没给**。egui 的默认值实测下来是：`button_padding`
      只有 `vec2(4.0, 1.0)` ⇒ **高度≈一行文字高、宽度随文案长度**；深色 `widgets.inactive` 是
      `weak_bg_fill = gray(60)` 压在 `panel_fill` `gray(27)` 上、`corner_radius = 2`、**无描边**；
      `selected(true)` 落到 `selection.bg_fill`（深色 `rgb(0,92,128)`）。所以"不像按钮"的根因
      **不是配色**，而是**没有任何一处告诉用户"这里可以按"**。
    - **正解 = 自绘 + 把量写死**：`ui.allocate_exact_size(vec2(width, 32), Sense::click())` +
      `Painter`，其中 `width` = 内容宽夹在 `[130, 可用宽 − 24]`。**宽度贴合内容**而不是全宽——
      这是用户第二轮目视的原话「按钮不需要那么长贴到右边框」；一块占满侧边栏的实心色块看起来
      像横幅，不像按钮。上下限各管一头：**下限 130** 给极短文案兜底（UDP 未连接的「绑定」纯按
      内容算不到 90，像标签）；**上限 = 可用宽 − 24** 保证任何语言下右边缘都不贴面板边界，
      撞上限时再按比例缩字号。按钮内部要排三个元素（状态点 / 文案 / 箭头）也是 `Button` 做不到
      的（它只画居中文字），所以自绘不是风格选择而是必要条件。
    - ⚠️ **先量后画，且顺序不能换**：宽度得先量文案才能定，而"量文案用的 `painter`"与
      `allocate_exact_size` **都要借 `ui`**，同时借会冲突 ⇒ 把量文本放进一个块里，出块再
      allocate。另外上限也要 `.max(最小宽度)` 夹一次：侧边栏拖到最窄（200）时
      `available_width()` 本身就小，不夹的话 clamp 的上下界会颠倒。
    - **底色报"按下去会做什么"，不报"现在是什么状态"**：未连接绿 / 进行中琥珀 / 已连接红。
      这与底部状态栏的圆点**同色不同义**（那边报状态）。其中琥珀**与状态栏
      `ConnectionIndicator::pending` 取同一个值**并有测试钉住——同一个含义在项目里只能有一处
      颜色定义。**悬停变亮 / 按下变暗**，方向要与 egui 默认控件一致（深色
      `inactive 60 → hovered 70 → active 55`），否则手感会和旁边的下拉框打架。
    - ⚠️ **文案永远跟 `is_connected` 走，包括「正在打开」那一档**：那时 `is_connected` 已为真、
      按下去确实是关闭 ⇒ 按钮必须显示「关闭串口」。写成"正在打开"就是 F4 那类"文案与动作
      相反"。**"正在进行"由底色 + 状态点 + 下方小字交代，不由按钮文案交代。**（当天出设计稿
      时我把这一态画成了"正在打开"四个字，是错的，实现时纠正了。）
    - ⚠️ **别拿"与 egui 正文档一致"当理由，先实测**：这一档原取 14，注释写着"与
      `TextStyle::Body`（14.0）一致"——**实测是 13.0**（0.36.2，运行期读
      `TextStyle::Body.resolve(ui.style()).size`）。于是 14 只是"比正文大 1 点"，放进实心色块里
      肉眼无差别，用户第二轮反馈就是「打开窗口这四个字太小了」。现在取 **16**，并且**故意大于
      正文**：它是主操作按钮的文案，不是正文。放不下时缩字号的下限取 **14**（不低于正文）。
    - ⚠️ **判据的四条纪律**（当天四条全踩了一遍，第一版测试因此报错两次）：
      ① **尺寸指纹要两条**：按钮靠"高 32"在形状列表里认出来，但侧边栏右上角那个 `>>` 收起
         按钮**也是 32 高**（`min_size(vec2(32.0, 32.0))`）——第一版就抓错了它（报"按钮宽 32"）。
         判据改成"高 32 **且**宽落在 **130..=200**"。**上界是必须的**：只留下界的话，
         "哪天又变回全宽"不会被发现。
      ② **要在同一帧里比位置**：按钮矩形与提示文字的位置互相比对时，**不能**分两次 `run_ui`
         取——主题、画布、绕行任何一处不同都会差几个点，比的就不是同一帧了。合并成一个
         `painted_frame()` 一次取回。
      ③ **先确认那个东西真的会被画**：`ComTool::default()` 默认已经选好 COM1，所以"未选端口"
         的提示根本不出现——断言前必须 `app.selected_port.clear()`，否则测的是"一个不存在的
         东西"。
      ④ ⭐ **阈值不许引用实现常量**：第二轮两条新测试的第一版分别写成
         `font_size >= CONNECT_TEXT_MIN_SIZE - 0.01`（注入把该常量压到 6 → **照样绿**，实测字号
         11.31 > 6）和 `font_size > body_size`（注入把字号改回 14 → **照样绿**，因为 14 > 正文
         的 13）。**引用实现常量就是自证**——判据的松紧必须对着"用户会不会不满意"定，不能对着
         "是否大于某个默认值"；阈值一律写成字面量。
    - ⚠️ **方向类断言要断通道大小，不能只断"两色不同"**：红绿对调之后"不同"依然成立。
      未连接必须 G 分量最大、已连接必须 R 分量最大；悬停/按下则比较三分量之和。
    - **顺带的布局后果**：那两条提示（「正在打开…」「请先选择端口」）必须放在按钮**下方**单独
      一行——按钮吃满整行时同行只剩几个字宽，收窄之后右边又留着边距，一样放不下这整句话。
      有测试钉住位置关系。

55. **全界面按钮统一外观：走 `crate::button`，宽度只许贴内容、不许超可用宽度**（2026-09-24 用户要求
    「把 GUI 界面上的所有按钮都按照打开串口的样式重新写一遍，注意和其它组件的边距，不允许覆盖到其它
    无关的组件」）：
    - **范围要先问清楚，不能默认"所有"**：这次定稿的结果是「只改界面命令按钮」——侧边栏、发送区、
      图表面板、设置窗口、发送区表头的「发送历史」占位，共 **16 处**。**弹出菜单里的项不改**：菜单栏
      「文件 / 设置 / 视图 / 文本」四个下拉、发送历史下拉的每条历史、接收区右键的「复制全部日志」——
      它们的宽度由菜单决定、整行高亮是菜单自己的选中语言，套上实心块会撑宽菜单并与菜单风格打架。
      用户说"所有按钮"时，"所有"的边界（含不含菜单项）**必须问**：方向写错返工代价大。
    - **两档配色 + 两档尺寸**：`ButtonTone::{Primary, Neutral}`（主操作绿＝发起一件事；其余中性灰）；
      `ButtonSize::{Normal 28/15, Compact 24/14}`。**刻意不做红色档**——界面上有五处
      「清空 / 恢复默认」，全画成红色会在屏幕上同时出现五个红块，反而让真正该被看见的那一个
      （连接按钮的"断开"）不再突出。绿的三个值**直接复用连接按钮那一档**：同一含义只能有一处定义。
    - ⚠️ **"不允许覆盖到其它无关的组件"要靠两条机制，缺一条都可能越界**：
      ① **宽度上限只能是 `ui.available_width()`**，**不许**写成 `max(available, 下限)`——那在窄容器里
         会算出超过可用宽度的值，正是用户警告的那种越界（实测注入后按钮宽 114.375 > 可用 90）。宁可
         按钮被挤窄：窄了还能看，越界会压住旁边的东西。收窄时按比例缩字号（标准档到 13、紧凑档到 12
         为止），**不截断**。
      ② **文字用 `painter.with_clip_rect(rect)` 画**：万一容器窄到"缩到最小字号仍放不下"，也只让字被
         裁在按钮内，不会跑出去压住旁边的控件。正常宽度下裁剪不起作用。
      `allocate_exact_size` 会把矩形计入布局（不像裸 `Painter` 那样凭空画上去），所以只要不超宽就压不到
      同行控件——这是"不覆盖"的前提。**宽度要在 `allocate_exact_size` 之前算完**：占位之后再改宽是无意
      义的，布局已经按旧宽度推进了光标。
    - ⚠️ **禁用态不是"把常态乘一个透明度"**：那样在浅色主题下会得到一块**更浅**的灰，反而比常态更
      扎眼。用两个固定值（深色 `#2E2E2E` + 字 `#6E6E6E`；浅色 `#EDEDED` + 字 `#A0A0A0`），并且判据是
      "**到面板底色的距离更小**"而不是"和常态不一样"——后者会被"改成任意另一个亮色"骗过。同时把
      `Sense` 降成 `Sense::hover()`，灰着就点不动；驱动方式用 `ui.add_enabled_ui(..)`，让绘制函数读
      `ui.is_enabled()`。
    - ⚠️ **底色报"按下去会做什么"**，与连接按钮同一条规则：侧边栏的「开始记录」在开始之后**变回中性
      灰**（那时文案是「停止记录」，按下去不再是"发起"）。一直绿着不变，用户会以为按下去还会开始。
    - **接入测试必须单独写一条**：纯函数测试全绿不代表界面调用了它（这正是约定 43 那条教训的又一次
      应用）。`button::tests::*` 之外还要有 `sidebar::tests::sidebar_buttons_use_the_shared_button_style`
      ——读实际画出的形状，要求至少三个「高 28 + 中性灰实心底」的矩形。**把界面里的调用改回 `ui.button`
      时它会变红**（实测：从 3 个掉到 1 个）。
    - **尺寸/字号判据一律用字面量**（28 / 24 / 15 / 14），理由同约定 54 ④：引用 `ButtonSize::height()`
      就是自证。这条规矩当天第二次派上用场——本轮的 7 处注入里，J1（高度改 30）与 J4（字号改 12）正是
      靠字面量才红的。

56. **写死尺寸的控件必须夹在可用空间里；"够不够显示"先量再答**（2026-09-24 用户问「实时图表的窗口
    高度这么小，能够正常显示数据吗」）：
    - **回答"能不能显示"这类问题，必须从形状列表里读真实矩形**，不能凭 `Plot::height(220.0)` 这个
      字面量推断。实测手段见 `panels/chart.rs::tests::paint()`：用 `Context::run_ui` 画一遍、遍历
      `FullOutput::shapes`——**绘图区**是"填充色恰好等于 `extreme_bg_color` **且**描边宽度 > 0"的那个
      `Shape::Rect`（图例背景同色，但它被 `background_alpha` 乘过透明度，所以这条判据能把它排除）；
      **曲线**是点数 ≥ 10 的 `Shape::Path`（网格线与坐标轴的点数少得多）。**末尾要
      `textures_delta.clear()`**。
    - 实测值（4 通道 × 60 点、宽 900）：图表块 **251 点** = 标题行 31 + 绘图区 220；**绘图区 886 × 206**；
      曲线包围盒 y **40.4–227.6**（落在绘图区 31–237 之内，上下各 10 点余量）⇒ **曲线用掉绘图区 90% 的
      竖直高度**。所以 220 点**够用**，"这么小能不能显示"的担心实测不成立。
    - ⚠️ **但 `egui_plot` 的 `Plot::height()` 是"固定"而不是"至多"**：它同时写 `min_size.y` 与
      `height`，而 `calculate_widget_complete_rect`（`egui_plot-0.37.0/src/plot.rs:814-853`）做
      `unwrap().at_least(min_size.y)` ⇒ 矩形**恒为那个值**，可用空间再小也不缩。实测中央区只剩 220 点时
      越界 **31** 点、只剩 120 点时越界 **131** 点；中央区**没有滚动条**，越界部分被面板裁掉 ⇒ 夹在它
      **下面**的发送区整块落到可视区之外（中央区高 640 点时图表底已经越界 53 点）。
    - **第一次修法**（当日已被第二次取代，见约定 57）：`plot_height(available) = available.clamp(90, 220)`，
      取值点放在**画完标题行之后**（`ui.available_height()`），比预先减一个常数准；非有限输入退回上限值，
      **不要产出 NaN**。**空间充足时必须与从前逐点一致**（仍 220）。它确实消掉了越界，但矮窗口下
      **绘图区被压扁 ⇒ 纵横比失真**，趋势看起来比实际平缓——所以只活了几小时。
    - **同族提醒**：凡是"写死尺寸 + 参加 `CentralPanel` 纵向堆叠"的控件（图表绘图区、接收区滚动区的
      `max_height(400)`），都要各自夹在可用空间内，因为**中央区没有滚动条**——越界就等于用户够不着。
    - **两条判据缺一不可**：① **永不越界**（画布 180 点 → 占用 ≤ 180，且不留空白）；② **容器够高时
      没变**（绘图区仍 ≥ 200 高 / ≥ 800 宽）。只有 ① 会放走"顺手把默认高度改小"的实现。
    - ⚠️ **一处负面结果要记下来**：注入"关掉 `auto_bounds`"**没有**让任何测试变红——首帧本来就会自动
      定界。所以"曲线落在绘图区内"这条断言**不等于**"自动缩放开着"，它拦的是"画到别的矩形里去 /
      变换退化"。要真正盯住"数据没被压成一条看不出来的线"，判据得是**曲线占绘图区竖直高度的比例 ≥ 70%**
      （注入 `include_y(1e6)` 时它会红，并报出 0%）。

57. **"高度不够"的解法是让容器夹住并滚动，不是把内容压小；而且要先定滚轮归谁**（2026-09-24 用户要求
    「复用日志滚动区」，图表随即移到中央区顶部）：
    - **两种"夹"的语义相反，先认清再用**：`ScrollArea::max_height` 是**至多**（可用空间更小时它自己缩，
      因此**永不越界**）；`Plot::height()` 是**固定**（同时写 `min_size.y`/`height` 再取 `at_least`）。
      把图表的 `Plot::height()` 交给外层滚动区的 `max_height` 去夹，就同时得到"块不越界"与"内容不被压扁"。
    - **复用方式是抽构造函数，不是把参数抄一遍**：`receive.rs::capped_scroll_area(axis, id_salt, max_height)`
      是**唯一**落点（`auto_shrink([false, false])` + 上限 + id 只定义一处），接收区两处与图表共用它；
      照抄参数迟早漂移。⚠️ 图表的 `id_salt` 必须与 `Plot` 的 id **不同**——滚动位置与视图范围是两份独立状态。
    - ⭐ **滚轮归属要先查清**（egui 0.36.2 `containers/scroll_area.rs:1219-1254`）：滚动区**只在"内容确实
      超出可视区、且该方向还能滚"时**才消费 `smooth_scroll_delta`（源码注释原文："Clear scroll delta so no
      parent scroll will use it"）。内容装得下 ⇒ 两个条件都不成立 ⇒ 滚轮原样留给内部控件。**这是"图表套
      滚动区"没有把交互弄坏的全部依据**；若把上限设成"永远装得下"，就等于套了个不会滚的空框（判据见下条）。
    - **两条判据缺一不可**（`panels/chart.rs`）：① **块不越界**（画布 150 → 占用 ≤ 150）；② **绘图区仍是
      220**（≥ 200）。只断 ① 会放走"顺手把高度算小"，只断 ② 会放走越界（改动前的写死 220 正是"绘图区
      完整 + 越界 131 点"）。另配一条**组件级**测试
      `receive::capped_scroll_area_caps_the_height_and_can_scroll`：① 容器 600 比上限 400 高时只占 400；
      ② 容器 120 比内容 300 矮时 `content_size > inner_rect`（egui 文档给出的"需要滚动"判据）。
      **② 是关键**：只断 ① 的话，"上限设成无穷"也照样绿。
    - **顺序也要有守卫**：`app::chart_sits_above_the_receive_area` 比两块**落笔文字的 y 坐标**，不是
      "文字存在"——后者把图表挪回底部照样通过。为此把中央区那一列抽成
      `ComTool::render_central_column(&mut self, ui)`：`eframe::App::ui` 需要无头环境造不出的
      `eframe::Frame`，写在里面等于一次也不会被 `cargo test` 画到。
    - ⚠️ **同一个 trait 不能在两个 `impl` 块里实现**（E0119）：为了给固有方法腾位置而把
      `impl eframe::App for ComTool` 提前闭合、稍后再开一块，**编译不过**。固有方法放独立的
      `impl ComTool`，trait 方法（含 `save`）全部留在**同一个** trait 块内。
    - **非空转验证 3 处注入全红**：图表挪回发送区之后 → 顺序测试红（报出两块各自的 y）；`max_height` 改
      `f32::INFINITY` → 上限断言红（"容器有 600 点高，滚动块却占 600.0——上限 400 没生效"）；绘图区高度
      改 60 → 绘图区断言红（"被压到只剩 46.0 点高"）。
    - **同族提醒**：凡是"写死尺寸 + 参加 `CentralPanel` 纵向堆叠"的控件都要自己夹在可用空间内——**中央区
      没有滚动条**，越界就等于用户够不着（接收区那处 `max_height(400)` 仍在，见约定 56 的同族提醒）。

58. **"往上翻就该停住"= 别在用户读历史时淘汰旧内容；而追平欠账必须分帧**（2026-09-26 用户报告：
    「串口日志在向上滚动的时候，用滚轮往上应该自动停止日志滚动，好像没有实现这个功能？」）：
    - **先分清"谁负责解粘"**：滚轮往上让视口不再粘底**是 egui 自己做的**——`stick_to_bottom` 的文档
      原文是 *"The scroll handle remains stuck until user manually changes position"*，实现是
      `containers/scroll_area.rs` 里滚轮分支把 `state.scroll_stuck_to_end` 置 `false`（还有拖内容、拖
      滚动条两处）。**本项目一行都不用写**；写测试量它也不该拿 `offset` 当判据（见下条）。
    - ⭐ **真正缺的那一环是淘汰**：`trim_text_front` 从**最旧**一端裁，滚动位置（像素）不变、内容却
      整体上移 ⇒ 用户读到的那一行被换成更晚的行，观感与"自动滚动根本没停"一模一样。贴底时看不出来
      （贴着的是最新内容），**只有往上翻着读时才暴露**。判据是**视口顶部那一行的绝对行号**，不是
      `offset`：`offset` 在缺陷下**每帧都不变**（实测 1316.0 恒定），而行号每帧 +5 往前跑。
    - **修法（两半，缺一不可）**：① 用户不在底部（`ComTool::receive_at_bottom == false`）时把上限放宽
      `RECEIVE_TRIM_DEFER_ALLOWANCE`（30 000 字符 ≈ 1 000 行），仍在读就不裁；② 回到贴底后要追平
      这笔欠账，但**每帧最多裁 `receive_trim_budget` 个字符**（上一帧实测的"半个可视区能装多少字符"，
      `trim_budget_per_frame()`）。
    - ⭐ **为什么必须分帧**（这一条是量出来的，不是想出来的）：egui 用**上一帧**的滚动位置给内容布局，
      把 `offset` 夹回合法范围发生在**内容画完之后**（`containers/scroll_area.rs` 里
      `state.offset[d].min(max_offset[d])` 在滚动条绘制段）。一帧里把内容缩短超过一个可视区高度，
      这一帧的内容就整个落在视口上方、被 `is_rect_visible` 判掉 ⇒ 用户看到**接收区空一帧**。实测：
      欠账 2 850 字符一次裁完时，那一帧画出的文字 shape 里只剩半行残句；改成 371 字符/帧后逐帧都
      有完整内容。**"半个可视区"是按高度换算的**，所以字体档位（`ui_font_scale`）与窄窗口自动跟上
      ——写死"12 行"在窄窗口（可视区 120）下就失效了。
    - **判据与空转验证**（`receive.rs` 四条新测试）：`is_at_bottom_covers_the_three_interesting_positions`
      （三态真值表：装得下 / 贴底 / 翻上去差 1 点以外）、`trim_budget_stays_well_under_one_viewport`
      （额度 < 可视区字符数的一半，且视口减半时额度同步减半）、
      `front_trim_is_deferred_while_the_user_reads_history`（读历史一个字符都不裁 / 额度用满后照旧裁 /
      贴底恢复原上限 / 接收区隐藏时照旧裁到上限）、`wheel_up_freezes_the_reading_position`（无头驱动
      **真实** `render_receive_area`，喂 `Event::MouseWheel`，从**画出来的文字 shape** 反推视口顶行行号）。
      **三处注入全红**：把"读历史放宽"改回一律裁到 `max` → 单元测试报"用户正在读历史，却把旧内容裁掉了"
      **且**真实路径那条报出行号逐帧前进 `[109, 113, 117, 122, …]`；把额度改成 `chars`（=一次裁完）→
      单元测试报"额度 5850 字符已经接近可视区高度"，真实路径那条报"这一帧没有画出接收区日志文本"
      （即空一帧）；把 `is_at_bottom` 改成恒 `true` → 真值表与真实路径两条同时红。
    - ⚠️ **量"用户看见什么"要素材化**：这一批断言读的是 `FullOutput.shapes` 里的 `Shape::Text`——
      `t.pos.y + row.rect().max.y > clipped.clip_rect.top()` 找出**压在视口顶部的行**，`clip_rect` 必须
      是接收区可视区（实测 400 高，不是整窗 600）。认出"日志那一份"**不能**要求文本以 `line ` 开头：
      分帧追平期间截口会落在行中间（顶行是半行）。
    - **留档取舍（2026-09-27 已改，见 §3.28）**：原先写的是"到额度后恢复淘汰，阅读位置跳一次，
      内存上限优先"。用户第二轮报告（"停留超过一定的时间，日志又自动滚动，再往上滚也停不住"）
      证明这个取舍**用户不接受** ⇒ 现在改成：额度用尽后**照旧淘汰，但滚动位置同步往回退**
      （`take_front_trim_compensation`），并且**最多裁到视口顶行为止**（`cut.min(headroom)`），
      于是阅读位置一直不动，代价是内存一路涨到 `max_log_chars + RECEIVE_PARKED_EXTRA_CAP`
      （默认合计 90 000 字符）。**越过那个硬上限才会重新让位给内存**，这是唯一的例外。
    - ⭐ **从前端删内容的两条硬规则**（都是量出来的，§3.28）：① **截口必须落在行首**——
      `floor_cut_to_line_boundary()` 方向是"宁可少裁"，**不足一行就这一帧不裁**；实测"差一个字符
      就照原样裁"会让每帧切掉 29 字符、把最前面一行连同其换行一起吃掉 ⇒ 视口顶行逐帧被换成下一行
      （增量 `+1, 0, +1, 0 …`）。② **补偿基准取上一帧 `out.state.offset.y`**，不能用
      `State::load`（绘制前的上下文里它恒为 `None` ⇒ 基准变 0 ⇒ 视口被钉死在顶部）。
    - **同族提醒**：任何"从容器**前端**删内容"的操作都会造成同样的位移（不仅是日志：列表、表格虚拟滚动
      同理）。判据一律用"用户正在读的那一项是否还停在原地"，不要用滚动条数值。

59. **"界面上的一个勾选 → 窗口真的变了"要能被无头测试证明**（2026-09-27「总在最前端」，见 §3.29）。
    涉及**宿主窗口**的偏好（置顶 / 全屏 / 标题 / 尺寸）与普通字段不同：**生效靠一条命令**
    （`ctx.send_viewport_cmd(ViewportCommand::…)`），**不是改个字段就完事**。三条规矩：
    - **窗口级状态读不回来 ⇒ 自己记"已下发值"做去重**。`apply_theme` 能靠
      `ctx.options(|o| o.theme_preference)` 问当前值，但**窗口层级没有对应的可读状态**
      （`ViewportCommand` 是只写的，egui 只把它转发给 winit）⇒ 必须自己存
      `applied_always_on_top: Option<bool>`（`#[serde(skip)]`，初值 `None` ⇒ 第一帧必发，
      这同时让"上次勾过"能在启动时恢复）。不记就是每帧一次 `SetWindowPos`。
    - **接入点选"每帧必画 + 只收 `&mut Ui`"的函数**。`ui()` 收 `eframe::Frame`，无头测试构造
      不出来 ⇒ 写在里面一次也不会被 `cargo test` 走到。菜单栏每帧必画，是同时满足"每帧同步"
      与"可测"的那个点。同一条理由适用于**任何**需要每帧下发的东西。
    - **关掉时要显式发回默认值，不是"什么都不发"**。窗口层级是三态
      （Normal / AlwaysOnBottom / AlwaysOnTop），取消置顶必须发 `Normal`——不发，窗口就一直
      停在置顶上，而且界面上再没有入口能把它弄下来。
    - **判据读 `FullOutput::viewport_output[&ViewportId::ROOT].commands`**（egui 把视口命令公开
      在每帧输出里）⇒ "发没发、发的是哪一档"可以断言，**不必真开窗口**。
      另配一条**落盘**断言（`config::tests` 的 `customized()` 里取**非默认**值），否则把字段标成
      `#[serde(skip)]` 也照样全绿——那就是"勾了置顶、重启就没了"。
    - ⚠️ **菜单主体要抽成 `fn view_menu_body(&mut self, ui)`**：菜单里的控件只在菜单**被点开**时
      才画，无头测试要模拟点开就得先算按钮坐标再伪造指针事件。抽出来后可直接驱动，
      判据不再依赖那堆坐标（与约定里的 `render_send_presets` 同一条）。
    - ⚠️ **反例（自己踩的）**：我一度加了条"没勾选时启动第一帧也要发 `Normal`"的断言——它确实
      能被注入弄红，但红不红对用户**没有任何差别**（窗口创建时本来就是 Normal）。
      **判据不对着"用户会不会不满意"的就是空转**，删掉。

60. **"这个控件能不能点"的判据全项目只有一条；禁用时**必须**常显说明理由**（2026-09-27，见 §3.30）。
    - **判据统一到"通道就绪"**：`ComTool::channel_is_ready()`（`pub(crate)`）是**唯一**入口，
      发送闸门与侧边栏的控制线勾选框都读它。⚠️ **`connected` 不是判据**——它的含义是"已经发起过
      一次打开"（F4 为修"假阳性"专门分出来的记账位）：串口 `open()` 返回后到端口真正打开之间，
      `connected` 为真而通道未就绪（界面显示"正在打开"）。拿它当"能不能操作"的判据，会出现
      **控件可点、命令只能进后台队列排队、而控件已经显示成点了**——界面在说谎。这条不一致在
      控制线勾选框上一直存在到 2026-09-27（发送闸门那边早就统一了，漏的是侧边栏）。
    - **禁用理由要常显，不能只挂 `on_disabled_hover_text`**：不把鼠标放到那个灰框上就看不到
      原因，用户只看到一个点不动的东西。既有的正确写法在「面板显隐」那处
      （`if !can_hide { ui.weak(s.at_least_one_panel) }`），照它来。**同一件事不要写两处**
      （悬停给一次、常显再给一次）——迟早一处改了一处没改；悬停留给**另一件事**
      （比如"勾的是请求电平、不是测量值"）。
    - **无头测试怎么断言"这就是禁用态"**：不能只断"提示语画出来了"（有人可能把
      `add_enabled` 改成恒真而提示照旧）。正解是**在同一套默认样式里画一对参照勾选框**
      （一个正常、一个 `add_enabled(false, …)`），读它们文字的
      `epaint::TextShape::fallback_color` 当标尺，再与目标控件的颜色比对——**不要写死颜色常量**
      （那是 egui 的实现细节，它一升级就假红，也说明不了"这就叫禁用态"）。
      实测（egui 0.36.2 深色默认）：启用 `#B4B4B4FF`、禁用 `#5A5A5A80`（**连 alpha 一起**被乘 0.5）。
      **非空转前提要写进断言**：先断 `enabled_ref != disabled_ref`，否则下面每条都拿同一个值比、
      怎么写都过。
    - ⚠️ **egui 0.36 的控件样式读的是上一帧同 id 的 `WidgetState`**
      （`widget_style::checkbox_style(&classes, state)` 里 `ui.ctx().read_response(id)`）⇒ 在**同一个
      `Context`** 里连着画"禁用 → 启用"，后一帧会拿到前一帧的状态、颜色滞后一帧。测不同状态时
      **每个状态新建一个 `Context`**。
    - ⚠️ **`ctx.run_ui` 要显式给 `screen_rect`**：不给的话 `Panel::right` 的可用尺寸会退化，
      侧边栏内容可能整个落进 `is_rect_visible` 的判否侧——"文字画没画出来"这类判据会全成假红。
    - **先量后写**：颜色能不能区分、差在哪，先用一个临时 `#[cfg(test)] mod` 打出来看，
      确认之后再把断言写死成"与参照对相等"。跳过量测很容易写出一条"两边都取到同一个占位色、
      怎么都通过"的空转测试。临时模块交付前删掉。


61. **"显示层"的加工（筛选 / 高亮 / 折叠）不许动数据；两条渲染路径的实现必须互相对拍**（2026-09-27
    接收区筛选，见 §3.31）。接收区有四份镜像（`receive_text` / `display_spans` / `ascii_spans`，
    三条渲染分发），任何"我只想看点别的"的需求都很容易顺着渲染路径改着改着把源数据也改了。
    - **只改"画哪一份内容"**：`receive_text` 与两个片段镜像原样保留 ⇒ 导出、实时日志文件、
      图表解析、`max_log_chars` 裁剪**一律照旧**。配一条测试专门断言"筛选从不改动三份镜像"
      （`filtering_never_touches_the_stored_log`）——没有它，"为了筛干净顺手删掉源数据"就是静默丢数据。
    - ⭐ **两份实现要对拍**。文本 / HEX 视图渲染 `receive_text`，彩色 / 字节查看渲染片段，
      所以过滤有两份实现（`filter_lines` / `filter_span_lines`）。**同一份输入喂两份、断言保留
      文本与两个计数逐字相同**：口径一旦分叉，症状是"同一个筛选在两个视图里筛出不同的行"，
      而单个视图的测试全绿。片段那份还必须**保住样式**（颜色是那两个视图存在的理由）。
    - **关掉时零开销**：文本那份返回 `Cow::Borrowed`（原样借用、不复制不扫描）；片段那份在
      **调用点**判空（不复制整份片段表）。别为了"统一写法"让关掉筛选也每帧复制几十 KB。
    - **按逻辑行（`
`）判定，不按折行后的视觉行**：彩色/字节视图会按窗口宽度折行，按视觉行筛
      会把半行留下、半行筛掉。末尾没有换行的半行（设备正发到一半）同样算一行、参与匹配。
    - ⚠️ **派生显示只算一处**：命中数由接收区那次绘制写入字段，顶部那一行只**读**它——顶部先
      于接收区绘制，所以读到的必然是上一帧的数（差一帧无影响）。在顶部再算一遍就会有两份数字。
    - ⚠️ **改了过滤条件要复位滚动位置**（复用「清空日志」那条 `reset_receive_scroll`）：内容整体
      换了，旧 `offset` 可能越过新内容的底部，视口落在空白处——看着像"筛选把日志筛没了"。
    - **临时工具不落盘，且"生效中"必须有常显指示**：筛选关键字与那一行的展开状态都是
      `#[serde(skip)]`（"启动后日志看起来是空的"极难自查）；而只要筛选在生效，命中数**一直显示**，
      包括把输入框收起来的时候——否则"日志被筛过"在界面上没有任何解释。

62. **"卡住"必须在界面上有痕迹——把不可见的状态变成可见的**（2026-09-27，P3 三条缺陷，见 §3.33）。
    三条里两条是同一个病：**行为其实一直存在，只是用户看不见**。"队列没满却写不动"与"记录中
    改配置不生效"原先都得靠用户自己盯着猜。
    - **诊断读数只在"有事"时出现**：状态栏的「待发 N 字节」只在 `> 0` 时画——0 的时候显示它只是
      噪音，而它要回答的问题恰恰是"有东西没出去"。
    - ⭐ **判据判的是"有没有进展"，不是"有没有变化"**：待发量的**下降**才算进展。写成"变化即进展"
      的话，用户每多发一次就把停滞计时归零，队列越堆越多反而**永远不报警**——正是要修的场景被修没了。
      测试里必须有一对断言专门区分这两种写法（把 `<` 改成 `!=` 会红）。
    - **时间相关的行为要把时刻作为入参**（`note_pending_bytes(now, at)`），否则测试要么真等 3 秒、
      要么什么都断言不出来。
    - **"静默"本身就是缺陷**：记录中改配置不切换文件是**正确**的语义（改一次就换文件会留一堆碎片），
      缺的只是"说一声"。补提示即可，**不要顺手改语义**。提示同样要**只在成立时出现**——常显就成噪音。
    - **让某个区域让位时，上限要随可用空间回落，而不是把常量调小**：调小常量会让宽窗口一起变矮。
      `receive_scroll_max_height` 写成 `(available - reserved_below).clamp(下限, 上限)`，而"要预留多少"
      按**当前字体的真实行高**估（`text_style_height`），不写死像素——写死的话字体档位调到「特大」
      时同一个缺陷会复发。
    - ⚠️ **新增 `pub(crate)` 字段会让所有外部模块的 `ComTool { ..Default::default() }` 报 E0451**
      （结构体字面量的 `..base` 语法要求出现的每个字段对当前作用域可见）。实测 45 条编译错误。
      这个项目里 `ComTool` 的字段因此基本都是 `pub`。

63. **「发送文件」：分块推进，且"没排进队列的那一块"绝不能丢**（2026-09-27，N7，见 §3.34）。
    - **在 UI 线程分块推进，不要另起线程**。`ByteChannel::write` 的背压判定**必须在调用线程**做
      （`PendingLedger` 的额度预约）——把写挪到后台线程等于绕开那条约定；而且 `channel` 是
      `ComTool` 独占的 `Box<dyn ByteChannel>`、`write` 还要 `&mut self`，要让别的线程拿到它就得
      引进共享句柄，搅动刚稳定的并发契约。每次 tick 读一块写一块，**一直推到队列排不下为止**
      ——这样吞吐由**队列的消耗速度**决定，不是帧率。
    - ⭐ **`write` 拿走 `Vec<u8>` 的所有权，失败时会把它丢掉**，所以"留着这一块下帧再写"只能靠
      **克隆**（4 KiB，代价可忽略）。不克隆就得丢掉重读——丢了是静默少发一段数据（收方拿到一个
      中间缺一段的文件，谁也发现不了），重读要 `seek` 而来源不一定可 seek。
    - **两种"停下来"要分开处理**：**通道不 ok 了 ⇒ 中止任务**（留着的话重连之后它会自己接着灌，
      收方拿到一个前半段属于旧连接的截断文件，而用户不知道谁在发——与定时发送同一条规矩）；
      **队列满 ⇒ 只暂停**（那正是背压在起作用，不是故障）。
    - **取消不撤回已经排进队列的部分**（它在通道自己的队列里，够不着），提示里必须写明。
    - 进度数字的含义是"**已排进队列**"，**不是"对端已收到"**——文案与注释都不许含糊。

64. ⭐⭐ **任何充当"判据"的辅助函数，都必须先用反向样本证明它不会假阳性**（2026-09-27，
    `HISTORY_2026.md` §3.35 ②）。本轮最贵的教训，值得单独成条。
    - **事故**：测试里的 `font_has_glyph` 实现是"在字体字节流里直接搜这个码点的 UTF-16BE
      两字节"——**根本没查 cmap 表**。实测它对中文字体 `msyh` 报"有"格拉哥里 / 吠陀扩展 /
      卢恩字形（一个中文字体不可能有这些，纯粹是那两个字节恰好出现在文件别处）。
    - **两层后果**：① **所有拿它做判据的测试全部空转**——越南语那条"每个字符都有字形"永远成立
      （注入验证时看得清清楚楚：把检测器改成恒 `true`，那条测试**照样通过**）；
      ② 它还被**反过来用错**了一次：本轮探针据此得出"天城文 128/128 全覆盖"，而真实实测是
      **四个候选字体全缺**。若直接把那个结论写进文档，就把一条错的可行性判断钉进了项目。
    - **规矩**：写出任何 `has_*` / `is_*` / `can_*` / `fits_*` 之类的判据辅助后，**先给它喂几个
      "答案已知为否"的样本**，确认它真的会说"否"。**能区分**才算能用——最好的证据是**两个样本
      给出相反答案**（汉字在 msyh 有、在 tahoma 没有；泰文反过来），那才说明它真在读数据，
      而不是在瞎点头。这条测试要**留下**（`the_glyph_probe_is_not_a_yes_machine`），它是检测器
      的守门人。
    - **同族坑**：判据依赖的"输入清单"漏了项也是**假红**——越南语那条测试的字体清单原先漏掉
      `malgun` / `seguisym`，重写检测器后立刻红在 `☰`（它确实在 `seguisym` 里、界面正常显示）。
      清单必须与真实来源（`main.rs::setup_fonts`）**逐项一致**。
    -       **口径要对着"用户真会用到的那批输入"**：按整个 Unicode 区块算覆盖率，希伯来文只有
      87/112（77%）——看着像缺陷，其实那 25 个缺口全是**本就未分配**的码点。换成"字母区"才对。

65. ⭐ **界面加入的新开关要有"用户看得到的效果"做判据，不能只有字段测试**（2026-10-06，
    见 `HISTORY_2026.md` §3.36）。清单三层，缺一层就有一类缺陷绕得过去：
    - **存在**：这个控件画得出来（无头渲染里能读到那句文案）。缺它 ⇒ "加了功能但没加入口"。
    - **接线**：控件绑定的**确实是那个字段**（把 `app.字段` 换成临时变量，画面必须变）。
      缺它 ⇒ "菜单上有，点了没反应"。注意它和"存在"是**互补**的：本项目实测
      `view_menu_puts_force_wrap_in_the_display_mode_group` 在"绑错变量"时仍然绿，只有
      `view_menu_checkbox_follows_the_switch` 会红。
    - **效果**：字段变化时**用户看到的东西真的变了**（读 galley 的行数 / 文本 / 高度，不是读状态）。
      缺它 ⇒ "字段改了、界面重画了、行为没变"。

    ⚠️ "默认勾选 / 默认值"这条尤其不能用 `assert!(app.字段)` 交差：那样配一个"没人读它"的实现
    也全绿。把判据写成**用默认的那个 App 跑一遍真实绘制**（本项目
    `receive::tests::a_fresh_app_wraps_long_lines_in_the_text_view`：全新实例在文本视图里
    400 个字符必须排成 3 行）。

    ⚠️ **判据里出现"必须恒定"的那处要特别小心**：本项目一度写过
    "勾选与不勾选画出的 shape 数量相同"，而勾选框多画一个勾号本身就是多一个 shape ⇒
    那条断言恒假。**同一份证的另一面是杀手**：不仅要有差异，还要规定差异**落在哪一行**
    （本项目：差异的 y 必须落在菜单里这一项那一行的 ±24 点内）。

66. ⭐ **改了一条"无法被单测证明"的代码时：把边界写进模块文档，并去找它「能被验证的那一面」
    做判据**（2026-10-07，见 `HISTORY_2026.md` §3.37 ②）。
    - **场合**：UDP 从阻塞改非阻塞，收的是"**本机网卡出口饱和**时 `send` 不再拖死 worker"。
      这个触发条件在门禁里**造不出来**（回环上发 4000 × 60 KB 给不读的对端是 0 次失败）。
      如果就此只写"已修复"，那是一句没有证的声明。
    - **规矩一**：模块文档里**显式写明"这条改动没有、也无法被单元测试证明"**，并把下面的测试
      定性为**防回归**而不是"生效证据"。写清楚的错比不写的对有用——下一个读者不会误以为
      "有测试绿着 ⇒ 出口饱和那件事验过了"。
    - **规矩二**：去找它的**副作用里能被验证的那一面**。同一个改动同时把"读侧不再每轮空等
      `READ_TIMEOUT`"带来了——旧实现下 `recv` 排在 `send` 前面、对端不发数据时每轮白等
      10 ms，**发送吞吐被钉死在约 100 个/秒**；改成非阻塞后一轮能走完整圈。于是判据定在
      吞吐上（2000 个数据报须在 3 秒内被**对端**收满；退回阻塞读实测 195/2000 变红）。
    - ⭐ **判据要用"对端实际收到多少"，不要写"我们写了多少"**：`write()` 只把数据排进队列
      （`worker::submit` 当场返回 `Ok`），真正发出去没有只有收方能作证。断"我们写了 N 条"
      的话，把 worker 整个停掉照样全绿——那正是空转。
    - **同族**：这条也解释为什么"修残余风险"要先分辨性质——§5 那张表里没划掉的 6 条里，
      只有 2 条真属"修得动"，另外 4 条是**刻意的设计取舍**（图表不支持无换行逗号流 / x 轴是
      组序号 / 面板隐藏时也解析 / 时间戳前缀是注解）。把它们一起改掉会顺手推翻四条已写进
      文档的产品决策。**"未修"不等于"待修"**。

---

## 7. 附录：逐模块约定与 egui 硬事实（英文，2026-09-26 从 `AGENTS.md` 迁来）

**为什么放在这里、为什么是英文**：这些条目原本堆在 `AGENTS.md` 的末尾（83 条、75 KB），
让那份文件涨到 **82 KB**——大到每次会话自动注入时**被截断**，AI 拿到的约束是不完整的。
2026-09-26 的文档拆分（技术债报告第 1 项）把它们整段迁到此处，`AGENTS.md` 只留
红线 + 精简模块图 + 索引 + 指针。

**照搬英文原文，没有翻译**——翻译过程中丢细节的风险大于收益，而这些条目里很多是
"实测出来的数字"，错一个数字比读着费劲严重得多。**中文口径以 §6 的约定为准；
两者若冲突，说明其中一份已经过期，请纠正而不是绕过。**

**⚠️ 本附录与 §6 重叠，但两份都**不能删**——这里给出**分工**与**对应关系**。**

**分工**（2026-09-27 标定）：

| | 内容性质 | 组织方式 | 语言 |
|---|---|---|---|
| **§6** | 规则的**规范表述**（"该怎么做"） | 编号（可直接引用"约定 52"） | 中文 |
| **§7** | 逐**模块**的实现说明 + **实测数字**（"为什么这么做、量出来是多少"） | 按主题成条 | 英文（原文照搬，未翻译） |

**冲突时以 §6 为准**：§7 是从 `AGENTS.md` 整段迁来的历史快照，§6 是持续维护的那一份。
遇到两者不一致，说明 §7 那一条已经过期——**请改掉，不要绕过**。

**为什么没有做"逐条删除重复"**：§7 那 68 条里有 **47 条**与 §6 有交集，但交集几乎都是
"同一条规则的两种写法"，而 §7 那一边**额外带着只有那里才有的实测数字**（例如
`Plot::height(220)` 实测越界 131 pt、TCP 阻塞写导致同一片重发 126 遍、`TextStyle::Body`
实测 13.0）。删掉重复句就会连带删掉这些数字，那是**丢信息**。所以这里做的是**标定**：
下表把每一处重叠指出来，读者不必再猜"该看哪一份、另一份是不是过期了"。

**重叠对应表**（脚本按"共同出现的代码标识符 ≥3 个"扫出来的；§7 条目序号 = 本附录
里 `- ` 开头条目的顺序号）：

| §7 条目 | 主题（首句摘要） | 对应的 §6 约定 |
|---|---|---|
| §7 第 2 条 | Module map: | §6 第 50、55、58 条 |
| §7 第 4 条 | Writing to the port is deliberately incr | §6 第 9 条 |
| §7 第 6 条 | Receive output has mirrored state: `rece | §6 第 11、61 条 |
| §7 第 7 条 | Receive logs are bounded by `max_log_cha | §6 第 11 条 |
| §7 第 8 条 | The timestamp prefix (`TimestampPrefix`: | §6 第 27 条 |
| §7 第 9 条 | The three display toggles (`hex_view` /  | §6 第 28、44、57 条 |
| §7 第 10 条 | Every consumer of the display mode must  | §6 第 11、28、52 条 |
| §7 第 11 条 | Every append to a receive mirror must al | §6 第 18 条 |
| §7 第 12 条 | Real-time file logging writes on its own | §6 第 17、20 条 |
| §7 第 13 条 | Logging state has a single source of tru | §6 第 19 条 |
| §7 第 18 条 | Chart drawing is capped: `panels/chart.r | §6 第 23 条 |
| §7 第 19 条 | The chart's plot height is a fixed 220;  | §6 第 23、56、57 条 |
| §7 第 20 条 | `Plot` and a `ScrollArea` both read `smo | §6 第 57 条 |
| §7 第 21 条 | Do not split one trait across two `impl` | §6 第 57 条 |
| §7 第 22 条 | There is exactly one way to establish a  | §6 第 33 条 |
| §7 第 26 条 | `open_connection()` must not write the s | §6 第 37、40 条 |
| §7 第 27 条 | The pending-send queue is bounded, and t | §6 第 51 条 |
| §7 第 28 条 | Reconnect uses exponential backoff: `rec | §6 第 37、40 条 |
| §7 第 29 条 | `channel` and `channel_kind` live and di | §6 第 34 条 |
| §7 第 31 条 | The connect button is self-drawn — keep  | §6 第 48、54、55 条 |
| §7 第 32 条 | The connect button's colour reports what | §6 第 54 条 |
| §7 第 33 条 | The two hints that used to share a row w | §6 第 54 条 |
| §7 第 34 条 | Every other button goes through `crate:: | §6 第 54、55 条 |
| §7 第 35 条 | UDP must never be described as "connecte | §6 第 36 条 |
| §7 第 37 条 | TCP's write side must stay non-blocking  | §6 第 51 条 |
| §7 第 39 条 | Every panel paints its own background —  | §6 第 53、54 条 |
| §7 第 40 条 | Transport-specific ignorable errors stay | §6 第 38 条 |
| §7 第 41 条 | Control lines (RTS / DTR, F5): opening a | §6 第 39、60 条 |
| §7 第 42 条 | Control-line failures are not disconnect | §6 第 37、40 条 |
| §7 第 44 条 | Which baud entry is selected lives in it | §6 第 45 条 |
| §7 第 45 条 | The settings window's content is taller  | §6 第 46、57 条 |
| §7 第 46 条 | There is exactly one outbound path: `pay | §6 第 41 条 |
| §7 第 47 条 | The bottom status bar (convention 48) mu | §6 第 48、54 条 |
| §7 第 48 条 | The global UI font scale (`UiFontScale`) | §6 第 47 条 |
| §7 第 49 条 | Presets (`SendPresets`, F5) deliberately | §6 第 42 条 |
| §7 第 50 条 | New drawing logic should be a `fn xxx(&m | §6 第 43、57、59 条 |
| §7 第 51 条 | The receive area's default text colour m | §6 第 44 条 |
| §7 第 52 条 | Panel visibility has an invariant: at le | §6 第 12 条 |
| §7 第 54 条 | Theme preference is persisted as the cra | §6 第 14 条 |
| §7 第 55 条 | Adding a language is a data change, not  | §6 第 50 条 |
| §7 第 56 条 | Multilingual fonts go in both families.  | §6 第 50 条 |
| §7 第 58 条 | Persistence: `ComTool` derives `Serializ | §6 第 3、50 条 |
| §7 第 60 条 | Tests are inline: parser/renderer/trimmi | §6 第 48、59、61 条 |
| §7 第 61 条 | The 2026-09-24 fix for the three long-st | §6 第 51 条 |
| §7 第 62 条 | Verification commands: `cargo check`, th | §6 第 10、49 条 |
| §7 第 64 条 | Compile before asking for a visual revie | §6 第 49 条 |
| §7 第 65 条 | The chart draws with `egui_plot`. Its ve | §6 第 24 条 |

其余 **21 条**（编号 1, 3, 5, 14, 15, 16, 17, 23, 24, 25, 30, 36, 38, 43, 53, 57, 59, 63, 66, 67, 68）在 §6 里**没有对应**——
它们是 §7 独有的内容（模块地图、图标、`opencode.json`、许可证、验证命令等），**不可删**。

- This is a single Rust 2024 binary crate (`comtool-rs`); the executable entrypoint is `src/main.rs`.
- Module map:
  - `src/app.rs` — `ComTool` state, `Default`, connection lifecycle (`open_connection` / `close_connection` / `teardown_channel`, branching on `ConnectionKind`), the `ThemeChoice` / `Language` / `ConnectionKind` preference enums, view-level helpers (`visible_panel_count`, `apply_theme`), the `eframe::App` top-level orchestration, and (F5) the control-line entry point (`apply_line_state`), the send-byte counter allocation in `transmit`, port hot-plug (`rescan_ports` / `note_focus`), and the preset row renderer (`render_send_presets`). `Language` carries 12 variants (`Zh, En, ZhHant, Ja, Es, It, Pt, Id, Fr, De, Ko, Vi`), a `const ALL` in the *same order* as the string-table columns, and `endonym()` (each locale's own name, used by the settings dropdown so a user can always recognise their own language) — variants are `Serialize`/`Deserialize` by name, so only ever append.
  - `src/receive.rs` — the receive pipeline: mirrored state, `AnsiDecoder`, byte-level ASCII visualization, the optional timestamp prefix (`TimestampPrefix` / `format_timestamp` / `timestamped_spans`), display-mode exclusivity (`resolve_display_mode`), layout building, the three view renderers, and the `max_log_chars` FIFO trimming (`trim_text_front` / `trim_spans_front`). **The trim limit is not the constant `max_log_chars`**: each render reports where the viewport sits (`ReceiveScrollOutcome` → `ComTool::receive_at_bottom` / `receive_trim_budget`), and while the user has scrolled back to read history the oldest content is **kept** (`RECEIVE_TRIM_DEFER_ALLOWANCE`) — dropping it would shift the text under their eyes, which looks exactly like "auto-scroll never stopped". Returning to the bottom catches up in per-frame instalments (`trim_budget_per_frame`) because shortening the content by more than one viewport height in a single frame blanks the log for that frame (egui lays out with the previous frame's offset and only clamps it afterwards). ⭐ Once the allowance is exhausted, trimming **resumes** but must not steal the reading position: `RECEIVE_PARKED_EXTRA_CAP` bounds how far the buffer may grow while parked, `floor_cut_to_line_boundary` keeps every cut on a line start (a half-line cut eats the line the user is reading), and `take_front_trim_compensation` rewinds the scroll offset by the re-laid-out height of the trimmed head so the viewport keeps pointing at the same line. See `DEVELOPMENT_GUIDE.md` §6 no. 58 and `HISTORY_2026.md` §3.28. `ChannelEvent::Notice` is handled here too: it writes a note **without** touching `connected` or stopping repeat-send.
  - `src/send.rs` — send-side pure logic: `parse_hex` / `parse_escapes` (returning `EscItem::{Literal, Bytes}`), `build_payload`, `SendFormat` (text / HEX / escape), `SendHistory` / `HistoryNav`, and (F5) `SendPresets` + `preset_label`. No `egui` and no serial dependency, so it is unit-testable as-is.
  - `src/log_file.rs` — real-time file logging: the `LogSink` writer thread, file-name template rendering, same-name collision handling, the bounded write queue, and the `impl ComTool` entry points (`start_logging` / `stop_logging` / `log_append` / `poll_log_events`).
  - `src/chart.rs` — the real-time chart's data layer: parsing the raw byte stream into multi-channel numeric series (`ChartData`), plus `impl ComTool { clear_chart }`. Deliberately free of any `egui` dependency so it is testable on its own.
  - `src/app_icon.rs` — the application icon, generated at compile time / run time from code (rounded square + digital pulse waveform, 256×256, supersampled). It has no external asset file, so it is fully covered by unit tests and needs no GUI to verify.
  - `src/button.rs` — **the single place that draws a button** (2026-09-24, "make every button look like the connect button"). Two orthogonal enums: `ButtonTone::{Primary, Neutral}` (green = "this starts something", neutral grey = everything else) and `ButtonSize::{Normal = 28 high / 15 pt, Compact = 24 high / 14 pt}` for buttons sharing a row with a combo box. `button()` is the only drawing entry point; `standard` / `primary` / `compact` wrap it. There is deliberately **no red tone**: five `清空 / 恢复默认` buttons in red would drown out the one genuinely destructive action (the connect button's "disconnect"). Two constraints carry the user's "must not cover unrelated widgets" requirement, and both are load-bearing: (a) the width is content-fit but **clamped to `ui.available_width()`** — the ceiling is *not* `max(available, min_width)`, since that computes an over-wide button inside a narrow container; (b) the label is drawn with `painter.with_clip_rect(rect)`, so when a container is too narrow even for the minimum font size the glyphs are clipped inside the button instead of spilling onto its neighbours. The disabled state is two fixed colours per theme rather than a multiply (a multiply gives a *lighter* grey in light mode, more prominent than the enabled button) and the sense drops to `Sense::hover()`, so a greyed button can never be clicked. The connect button keeps its own drawing (`sidebar::draw_connect_button`): its status dot and arrow are that action's own vocabulary, not part of the shared style — a generic button wearing them would be misleading.
  - `src/sidebar.rs` — the right-hand sidebar; returns a `SidebarAction` instead of mutating state while drawing. Holds the connection-kind radio plus the serial-vs-network parameter fork, and (F5) the baud dropdown (list entries + a "Custom…" entry backed by `ComTool::baud_custom`), and the RTS/DTR control-line row. The connect / disconnect button is **self-drawn** (`draw_connect_button`): full width, 32 high, a status dot on the left and a geometric arrow on the right, with `connect_button_tone` / `connect_button_fill` / `connect_button_label` as its pure decision functions — see the connect-button bullets below. Its pure helper `apply_baud_pick` is the single implementation of baud-click semantics. **It does not show running readouts**: the receive/send byte counters live in the status bar (below), and the available-port list is folded into the `串口信息` (`serial_info`) collapsing header rather than owning a header of its own. What stays here is *descriptive* state — port, baud, and the `display_mode` / `timestamp_prefix` lines. Its other command buttons (`>>` collapse, refresh, clear log, export log, start/stop logging, pick directory) all go through `crate::button` since 2026-09-24; **refresh uses the `Compact` size** because it shares its row with the port combo box, whereas the rest sit alone on a line. The logging toggle's tone follows the *action*, not the connection: `Primary` (green) while it reads "start logging", neutral once it reads "stop logging" — a green stop button would suggest it starts something.
  - `src/status_bar.rs` — the bottom status bar: a coloured dot, the connection word, and the receive/send byte counters, all as **one group centred horizontally and vertically** inside the bar. The bar is made taller than a line of text by `Frame::NONE.inner_margin(Margin::symmetric(BAR_PADDING_X, BAR_PADDING_Y))` and its panel separator line is switched off, so the content visually detaches from the panel border above and the window edge below. `connection_indicator(connected, opening, kind, s)` is the pure decision function (returns `is_on`, `is_pending` + label; `color()` maps that to a `Color32` — green / amber / red); `ComTool::show_status_bar` only paints it, and `draw_status_dot` draws a real filled circle rather than an emoji glyph (egui cannot render colour-emoji layers — see the status-bar rule below). It must be drawn **before** `CentralPanel` (egui panels reserve space in draw order) and it must use the same vocabulary as the sidebar — the **four core states** (serial/TCP connected/disconnected, UDP bound/unbound) **plus the `opening` state** — or the two indicators in one window will contradict each other. Centring is **computed, not declared**: egui's declarative centring (`with_main_align(Center)`, `vertical_centered`, `allocate_ui_with_layout` + `top_down(Center)`, `horizontal_centered`) all leave the painted text at x=0 in headless rendering, because they only re-align the *region rect* while emitted shapes never move back. So `show_status_bar` measures the row with `status_bar_row_width` (paints the identical row into an `invisible` probe `Ui` and reads its `min_rect().width()`) and then `add_space((avail - row_width) / 2)`. The row content lives in exactly one place (`status_bar_row`) because it is drawn twice — measure then paint — and the two passes must produce identical content or the centring drifts. The probe must be `invisible()`: a visible probe emits a second copy of every shape (two dots, text still at x=0), which is exactly the failure that was observed.
  - `src/strings.rs` — the UI string table. `strings!` declares `langs: ZH, EN, ZH_HANT, JA, ES, IT, PT, ID, FR, DE, KO, VI, TR, RU, PL;` and then one entry per string with **15 literals in that order** — a missing or extra column is a compile-time `compile_error!`, so an untranslated language is unwriteable. The macro transposes columns by "peel as we go" (a `macro_rules!` cannot concatenate token trees or do arithmetic in a pattern). Adding a language means: add the ident to `langs:`, append one literal to **every** entry, and append a `Language` variant + `ALL` + `endonym()` in `app.rs` — nothing else. Also holds `Language::strings()`, `fill()` for `{}` templates, and the guard tests (per-language script checks, N-way placeholder counts, traditional-Chinese sanity, font-chain coverage).
  - `src/panels/{menu_bar,settings,info,chart}.rs` — the top menu bar (`MenuAction`), the settings window, the about window, and the chart panel (`egui_plot` rendering + `points_for_drawing` downsampling). All inherent `impl ComTool` methods, so nothing is re-exported from `src/panels/mod.rs`. The menu bar carries **File / Settings / View / Text** — there is deliberately **no Send menu**: its contents (send format, line ending, repeat send + interval, send history) are an item-for-item duplicate of the send-area header row, which is now their **only** entry point (`ComTool::render_send_settings_row`, extracted from `ui()` so it is reachable from headless tests). Two guards pin this pairing: `panels::tests::menu_bar_has_no_send_menu` and `panels::tests::send_area_header_keeps_the_send_settings` — the first would pass trivially if it asserted "the whole frame has no 发送 text" (the send button and the settings heading also use that word), so it drives `show_menu_bar` alone. Likewise the menu bar's **Settings** menu is now the **only** way to open the settings window: the duplicate `⚙ Settings` button that used to sit in the central panel's top toolbar row was removed on 2026-09-24 (both merely set `show_settings = true`). `ComTool::render_top_toolbar` is what remains of that row — the sidebar visibility toggle plus (2026-09-27) the **filter toggle** and, whenever a filter is active, its match counter — and it is extracted from `ui()` for the same testability reason. The counter is written by the receive area during its own draw (that is the only place that actually filters), so the toolbar reads the **previous frame's** numbers; recomputing them there would create a second source of truth. Its guards are the pair `panels::tests::top_toolbar_has_no_settings_button` and `panels::tests::menu_bar_still_offers_settings`; the first matches the button text as a **literal** (the `settings_button` string-table entry was deleted with the button), so re-adding either the button or its table entry trips it.
  - `src/config.rs` — the persistence policy (`load` / `save` / `restore_runtime_state`) plus load-time invariant repair.
  - `src/communicationtrait.rs` — the `ByteChannel` trait (`open` / `close` / `is_active` / `write` / `configure` / `set_line_state` / `local_addr`; the latter three have default impls so a transport only overrides what it supports), `ChannelEvent` (`Bytes` / `Failed` / `Closed` / `Notice`), the worker-thread scaffolding shared by all three transports (`worker::{WorkerCommand, StateGuard, IDLE, ACTIVE, is_transient}`), and the `#[cfg(test)] mod fake` test double.
  - `src/serial_impl/{channel,config}.rs` — the serial implementation behind that trait; it is the only one that overrides `set_line_state` (RTS/DTR via `WorkerCommand::SetLineState`). `config.rs` also owns the arbitrary-baud range (`CUSTOM_BAUD_MIN` / `CUSTOM_BAUD_MAX` / `clamp_baud`).
  - `src/net_impl/{config,tcp,udp}.rs` — the network implementations (`TcpChannel`: synchronous `connect_timeout`, emits `ChannelEvent::Closed` on `Ok(0)`; `UdpChannel`: binds `0.0.0.0:local_port`, `Ok(0)` is a zero-length datagram, not EOF).
- Serial I/O must stay off the egui thread. `SerialChannel` in `src/serial_impl/channel.rs` owns a worker thread; it receives commands over one `std::sync::mpsc` channel (including `Halt`) and reports `ChannelEvent`s over another. Shutdown is driven by sending `Halt`, not by polling a shared flag — do not reintroduce a shared mutable state loop, and keep the `StateGuard` so `is_active()` can never stay `true` after the thread exits.
- Writing to the port is deliberately incremental: outbound bytes sit in a `VecDeque` and are advanced a bit per loop iteration. Do not collapse this back into a single `write_all`, because `timeout()` applies to writes too and `write_all` cannot report how much it wrote — a mid-write timeout would silently truncate long messages.
- `src/app.rs` polls serial events at the start of each UI frame and requests repaint every 30 ms; preserve this flow when changing receive or reconnect behavior.
- Receive output has mirrored state: `receive_text` is the plain-text/export view, while `display_spans` / `ascii_text` / `ascii_spans` feed the colored and byte-oriented renderers. Clear or append to the relevant mirrors together — `ComTool::clear_receive` is the single place that resets all of them.
- Receive logs are bounded by `max_log_chars` (0 = unlimited, default 30 000). Trimming happens in exactly one place, `ComTool::trim_receive_logs`, called at the end of `receive_data`. Each mirror is trimmed against the same character budget *independently* — the three modes do not write the same mirrors, so there is no index correspondence between them. Trimming must cut on **character** boundaries and must be able to split the boundary `ColoredSpan` from the middle (keeping its `AnsiStyle`); do not replace it with whole-span removal.
- The timestamp prefix (`TimestampPrefix`: `Off` / `PerLine` / `PerBatch`, default `Off`) is a **display annotation, not received data**. It is inserted in the *span* layer (`timestamped_spans`) so that `receive_text` — being concatenated from those spans — stays in sync automatically; do not add a second insertion pass in the text layer. It therefore also lands in the real-time log file and in exports, but **never** in the bytes fed to the chart. The line boundary is `\n`; `at_receive_line_start()` derives "the previous batch ended a line" from the tail of `receive_text` instead of keeping a parallel `bool` field (three modes, program notices and "clear receive" all write that mirror). One `Local::now()` per batch, so lines arriving in the same chunk share one timestamp. Nothing is prefixed when there is no content (`has_content`, not `spans.is_empty()`), so a partial UTF-8 chunk cannot produce a prefix with nothing after it. In the HEX view "per line" degenerates to "per batch" (one hex line per chunk) — that is intended, not a bug.
- The three display toggles (`hex_view` / `ascii_view` / `color_view`) have exactly **one** arbiter: `ComTool::resolve_display_mode` (`receive.rs`). Both UI entry points (the receive-settings row in `app.rs`, `panels/menu_bar.rs::display_mode_checkboxes`) and the startup repair in `config::restore_runtime_state` call it — do not inline `if hex { ascii = false }` at a call site, or the menu and the main window will eventually disagree. That is not hypothetical: the menu used to draw the colour checkbox **outside** the arbiter, which is exactly how the blank-receive-area defect was reachable from the menu while the settings row still looked correct. **Forbidden combinations** (each leaves the receive area **blank** because the writer and the painter disagree): `hex + ascii` (the HEX branch only writes `display_spans`, the byte view renders `ascii_spans`) and **`ascii + color`** (byte mode only writes `ascii_spans`, the colour view renders `display_spans` — 2026-09-24, found by the user's own eyeball inspection). **Legal**: `hex + color` (a hex dump shown through the colour view). The arguments mean "which one the user just ticked on", so the function applies that click and clears whatever conflicts with it; do **not** change it to "the view always wins" — that would silently undo a click on the other checkbox (looks like a dead control). With three `false`s it repairs stale archives: `hex` beats `ascii`, `ascii` beats `color` (the stronger view wins, and "byte view alone" is a legal configuration). Locked by `receive::tests::{enabling_hex_turns_the_byte_view_off, enabling_the_byte_view_turns_hex_off, enabling_the_byte_view_turns_color_off, enabling_color_turns_the_byte_view_off, stale_color_and_byte_view_are_repaired_in_favour_of_the_byte_view, legal_display_combinations_are_left_alone}` plus `config::tests::byte_view_wins_over_color_on_load`.
- **Every consumer of the display mode must read the same judgement**: `ComTool::display_mode()` returns `DisplayMode::{Color, Bytes, Hex, Text}` and is called by (a) fragment construction in `receive_data()`, (b) the mirror target (`to_ascii_mirror = mode == DisplayMode::Bytes`), and (c) the render dispatcher `ComTool::render_receive_area()`. Do **not** reintroduce a hand-aligned expression (`!self.hex_view && self.ascii_view`) or an `if self.color_view { .. } else if self.ascii_view { .. }` chain at any of the three sites: that asymmetry is what produced the blank receive area — data written into `ascii_spans`, screen painting the never-written `display_spans`. `DisplayMode`'s branch order only decides which view wins when a combination is *illegal* (hand-edited archive); correctness comes from both sides calling the same function. Locked by `receive::tests::no_display_combination_leaves_the_receive_area_empty` (all 8 toggle combinations, arbitration deliberately **not** applied, asserts the mirror the current mode paints is non-empty) and ⭐ `receive::tests::the_receive_area_paints_the_mirror_of_the_current_mode` (headless render; the three mirrors are tagged differently and the test infers which one was painted from the glyphs actually emitted). The dispatcher lives in its own function **so that it is testable at all** — inside `eframe::App::ui()` no test can reach it (conventions §6), and that gap is precisely why this defect passed every automated test on the first attempt.
- Every append to a receive mirror must also reach the real-time file log: use `push_receive_note` (mirror + log) or `push_str` followed by `log_append` (log only). Never call `self.receive_text.push_str` directly in `receive_data` without logging — that silently drops data from the log file. Note that `&self.event_rx` keeps `self` borrowed, so collect notices into a local `Vec<String>` first and apply them after the drain loop.
- Real-time file logging writes on its own thread (`log_file.rs::worker_main`); the UI thread must never `write` to the log file. Stopping sends `Halt`, and `Drop` must use `try_send` — `send` blocks when the queue is full, so a stuck disk could hang the UI thread forever. The file is opened by the UI thread and then handed to the thread, so "start logging" can report failure immediately; do not switch to the optimistic style used by the serial channel (its `open()` only spawns a worker, so a failure surfaces later as a `Failed` event). Existing files are never truncated: `create_log_file` adds a `_1`/`_2` suffix instead. Log file names are rendered from a user-supplied `chrono` template and must go through `render_log_file_name`, which falls back to the default template on invalid specifiers (`DateTime::format` panics on them) and replaces path separators with `_`.
- Logging state has a single source of truth: `log.is_some()` (query via `is_logging()` / `log_path()` / `log_written_bytes()`). The log file is *not* bounded by `max_log_chars`, and "clear receive" does not truncate it — both only affect in-memory mirrors.
- The real-time chart is **not** a fourth receive mirror. `receive_data` feeds it the **raw bytes**, before the per-mode branches run, so switching to HEX / byte view does not freeze the curves; conversely `ChartData` has its own parsing rules, its own limits, and its own clear entry point (`clear_chart`). Do not move the feed call inside one of the mode branches.
- Chart input is one group per line, fields comma separated (`\n` or `\r\n` ends a group). Field *k* always maps to channel *k*, and all fields of one group share the same x — the **group counter**, not a timestamp (serial data has no reliable time base). A line with no numeric field produces no sample **and does not advance x**, so text noise cannot punch holes in the curves.
- Never make the chart invent data: missing channels get no sample for that group (the curve just breaks — do not pad with zeros), and an unterminated comma stream is not re-grouped into fixed-width chunks (the channel count would be a guess, and a wrong guess draws the data misaligned). A half line exceeding `MAX_PENDING_BYTES` is discarded **as a whole line**: the rest of it keeps being dropped until a newline restores alignment, otherwise the truncated tail would become a fake sample.
- `ChartData` holds at most `MAX_CHANNELS` (16) channels and `chart_max_samples` points per channel, evicting the oldest — same FIFO idea as `max_log_chars`, but counted in points. Channel count only ever grows, derived from the widest line seen. The entry point clamps the limit itself so a hand-edited archive with `0` cannot make the chart look broken.
- Chart drawing is capped: `panels/chart.rs::points_for_drawing` downsamples to `MAX_DRAWN_POINTS` (4 000) per channel and **must keep the last point**, which is the newest data — losing the tail looks like "the device stopped sending". Curve colours come from a fixed palette (`CHANNEL_COLORS`, cycling per channel index): they encode *which channel*, so they are intentionally theme-independent.
- ⭐ **The chart's plot height is a fixed 220; what gets clamped is the *container*** (`panels/chart.rs`). `Plot::height()` in `egui_plot` sets *both* `min_size.y` and `height`, and `calculate_widget_complete_rect` applies `unwrap().at_least(min_size.y)` — so whatever value you hand it becomes a **fixed** rectangle that neither shrinks nor grows. Measured with 120 pt of room it still claimed 251 pt and overflowed by **131 pt**; the central panel has **no scroll area**, so that pushed the entire send area out of view. The fix is **not** a plot height that adapts to the available space — that was the same day's second attempt, and while it removed the overflow it squashed the plot, distorting the aspect ratio so trends read flatter than they are. Instead the plot keeps `PLOT_HEIGHT_MAX` (220) and is wrapped in `receive::capped_scroll_area`, the *same* constructor the receive log uses: `ScrollArea::max_height` really is an **upper bound** (it shrinks on its own when space runs out) and hands out a scrollbar once the content stops fitting. Keep both halves of the judgement together — "the block never overflows" **and** "the plot area is still 220" — each one alone lets a different defect through.
- ⭐ **`Plot` and a `ScrollArea` both read `smooth_scroll_delta`, so decide who owns the wheel.** egui 0.36.2 `containers/scroll_area.rs:1219-1254` only *consumes* the delta (its own comment: "Clear scroll delta so no parent scroll will use it") when the content genuinely exceeds the viewport **and** that axis can still scroll. When the content fits, neither condition holds, so the plot keeps its wheel zoom/pan; only once it is short enough to show a scrollbar does the container take over. That is why the chart is **wrapped rather than shrunk** — and it is the first thing to re-check if the chart ever ends up inside another scroll area.
- ⭐ **Do not split one trait across two `impl` blocks** (E0119 — the compiler rejects it, it is not merely a style question). Making room for an inherent method by closing `impl eframe::App for ComTool` early and reopening it after the new `impl ComTool` block does **not** compile. Put the inherent method in its own `impl ComTool` block and keep every trait method (including `save`) inside the single trait block.
- There is exactly one way to establish a connection: `ComTool::open_connection()`, used by both the UI button and `auto_reconnect`. It branches on `connection_kind` to build a serial / TCP / UDP channel, but `configure` → `open` → adopt the event channel is written **once** — do not clone that sequence per transport (a missing event channel means "connected but no data"; a missing `configure` means "the settings never take effect"). Preconditions are decided solely by `connection_ready()`, the wording comes from `channel_descriptor()`, and when they are not met the function must do **nothing** — never set `connected` optimistically.
- ⭐ **`connected` means "a connection attempt has been started", NOT "the channel is usable".** Serial is the only transport whose `open()` returns before the port is actually open (it just spawns the worker thread; the real `open` happens inside it). So ask `ComTool::channel_is_ready()` — **never** `connected` — whenever the decision is "may I let bytes through?": `send_data`, `send_preset`, and `tick_repeat_send` all guard on it. Guarding on `connected` alone resurrects the old false-positive defect one layer down (the UI shows "Connected", the user hits Send, and the bytes go nowhere). `connected` stays as the *attempt* accounting bit (it clears `last_opened_port` / `reconnect_failures` semantics, decides whether `close_connection` has anything to do, and gates `auto_reconnect`).
- ⭐ **The third state is `worker::OPENING`** (`communicationtrait::worker`). `SerialChannel::open()` stores `OPENING` *before* spawning the worker, and `worker_main` raises it to `ACTIVE` only after `open_port()` succeeds; `StateGuard` drops it back to `IDLE` on any exit. The ordering is the contract, not decoration: `UI stores OPENING` must precede `spawn` (otherwise the gap after `open()` returns looks like `IDLE` = "not running" = "the attempt failed"), and it must precede the worker's `ACTIVE` store (or a fast `open_port` gets stomped back to `OPENING`). `ByteChannel::is_ready()` (default: forwards `is_active()`; overridden **only** by `SerialChannel`) is what turns that state into the app's answer — TCP / UDP do connect/bind synchronously inside `open()`, so for them the two are identical and no implementation is needed. Do not add a serial `downcast` in the app layer: `is_ready` is the single fork point.
- The visible third state is "正在打开" (`status_opening` in the sidebar, `statusbar_opening` in the status bar; the dot turns **amber** via `ConnectionIndicator::is_pending`). It must stay distinct from both "Connected" and "Not connected": merging it into "Connected" is the defect, and merging it into "Not connected" makes a failed open look like nothing was ever attempted. The connect button's label treats "opening" as *already initiated* (it reads Close/Disconnect, matching the action), and the sidebar shows `opening_hint` instead of the "pick a port first" hint — those are different messages ("clicked, waiting" vs "clicking will do nothing") and must not share a string.
- `open_connection()` must not write the success line itself for serial: `open()` returning `Ok` only means "the thread started". Serial gets "正在打开 …" and the real "已打开" is appended later by `ComTool::settle_connection_state()`, which runs once per frame right after `receive_data()` and is idempotent via the `announced_open` bit (the channel stays ready for many frames — without that bit the message repeats every frame). Network channels set `announced_open = channel_is_ready()` right in `open_connection` because their own message is already final there. `announced_open` is cleared wherever `connected` goes false (`teardown_channel`, `Failed`, `Closed`) so a reconnect announces again. **`settle_connection_state` only ever raises — never clears `connected`**: a channel becoming not-ready is not a drop, and only the `Failed` / `Closed` events know *why*, so clearing here would double-report the same drop.
- The pending-send queue is **bounded**, and the verdict lives on the **calling thread**, not in the worker: `worker::MAX_PENDING_BYTES` (1 MiB) is enforced through `worker::PendingLedger` (an `Arc<AtomicUsize>` counting bytes handed over but not yet written), and every channel's `write()` goes through `worker::submit(ledger, sender, bytes, noun)` — *reserve quota → send → refund on send failure*. The three `worker_main`s only `push_back`; they must **not** judge capacity (two implementations of one rule drift, and the drifting one is the one nobody reads). Retirement is likewise shared: `worker::retire_finished_chunk` pops a fully-written chunk **and** returns its quota — the two must happen together, which is why they are one function. Rejection is a **synchronous `Err`** from `write()` (this is what makes the app's byte counter and "clear the input box" correct — it already gates both on `Ok`), worded 「待发队列已满（在队 N 字节，上限 1048576 字节），本条未发送」. Rationale: a one-way information loop (verdict in the worker, `write()` always `Ok`) is exactly the defect that made the status-bar counter disagree with the "not sent" notice. Growth is capped by total **bytes**, not entry count (one 4 MiB frame and four thousand 1 KiB commands stress memory equally). Every `open()` **must** `ledger.reset()`: the channel object is reused in place when the transport kind is unchanged, and chunks still queued at `close()` never return their quota — skip the reset and the next connect reports "queue full" before sending anything.
- Reconnect uses exponential backoff: `reconnect_backoff(failures)` (500 ms base, doubling, capped at 30 s) plus a `MAX_RECONNECT_ATTEMPTS` (6) give-up point, after which `auto_reconnect` stops entirely and `receive_data` has already written "自动重连已停止 …" exactly once (the message is emitted only on the crossing frame). `reconnect_failures` is `#[serde(skip)]` runtime state, incremented on `Failed`, **reset on `Bytes`** (received data is unforgeable proof the link works — this is the only reliable success signal for serial, whose open is optimistic), and reset by `close_connection`. `Closed` deliberately does **not** count: a graceful peer close is not a failed retry, and counting it would make a normal close-and-reopen slower every time.
- `channel` and `channel_kind` live and die together: the reuse rule is "**reuse in place when the transport kind is unchanged, tear down and rebuild when it changed**" (the guard reads `channel.is_none() || channel_kind != Some(connection_kind)`), and `teardown_channel()` clears both. That rule serves two things: serial reconnect is "open the same handle again", and the send-path tests depend on the injected `FakeChannel` *not* being torn down. Reverting it to "always build fresh" compiles fine but silently turns those tests into ones that talk to a real serial port.
- Serial-only controls must fork on the connection kind: port / baud rate / data bits / stop bits / parity / flow control are **not drawn at all** in TCP / UDP mode (grey-out is not enough — it implies they still apply), the available-ports list is hidden too, and the read-only info block becomes "target + local port". The connection-kind radio is **locked while connected**: switching it would leave the shown parameters describing a channel other than the open one, while the button reads "disconnect" and would tear down a different transport. Button labels and actions must always agree and follow the kind ("Open port / Close port", "Connect / Disconnect", "Bind / Disconnect").
- ⭐ **The connect button is self-drawn — keep every measurement explicit** (`sidebar::draw_connect_button`). It replaced `egui::Button::new(label).selected(is_connected)` with no width, height, fill, stroke or corner radius, i.e. everything from egui's defaults: `button_padding` is only `vec2(4.0, 1.0)` (height = one text line, width tracks the label), the dark-theme `widgets.inactive` background is `gray(60)` on a `gray(27)` panel with `corner_radius = 2` and **no stroke**, and `selected(true)` falls back to `selection.bg_fill` (`rgb(0,92,128)` in dark). The user's verdict was "make it look like a button"; the cause was not the palette — nothing in it marked the thing as clickable. It is now drawn with `allocate_exact_size` + `Painter`: **width hugs the label**, clamped to `[CONNECT_BUTTON_MIN_WIDTH 130, available_width − CONNECT_BUTTON_RIGHT_MARGIN 24]` × 32 high × corner 6 × padding 12 × 9 px status dot × a **triangular arrow drawn as geometry** (never the `▸` glyph — `epaint` only takes vector outlines, so an uncovered glyph becomes a tofu box; same lesson as the receive area). Width is computed *before* allocating — measure the label with a `painter` that lives only inside a block, since both it and `allocate_exact_size` borrow `ui`; the label ships in 15 languages and German/Russian run far longer than Chinese, so the ceiling caps the growth and the leftover overflow is absorbed by shrinking the font down to `CONNECT_TEXT_MIN_SIZE`. The label font is **16.0**, deliberately *larger than `TextStyle::Body`* — which measures **13.0** in egui 0.36.2, **not** the 14.0 the old comment claimed; the user's second verdict was 「打开窗口这四个字太小了」, and 14 was only one point above body text. Three reasons for self-drawing, in the function's own docs: an explicit size policy (min width, right margin), three elements inside one button (dot / label / arrow — `Button` only centres text), and a fill that follows the *action*.
- ⭐ **The connect button's colour reports what the click will DO, not what the state IS** (`sidebar::{ConnectButtonTone, connect_button_fill}`) — the same hues as the status-bar dot but a different meaning: idle = green `#1A7F4B` (light theme `#17794A`), opening = amber `#F5A623`, connected = red `#B3261E`. The amber **is the status bar's own `ConnectionIndicator::pending` colour**, pinned equal by a test — one meaning, one colour definition. Hover brightens and press darkens, matching egui's own direction (dark `inactive 60 → hovered 70 → active 55`) so it does not fight the dropdowns beside it. Guard the *direction* by comparing channel magnitudes (idle must have G highest, connected R highest): asserting merely "the two colours differ" is fooled by swapping them. The label, by contrast, always tracks `is_connected` — **including the opening state**, where the click really does close the port, so it must read Close/Disconnect; showing "Open port" there is the F4 "label contradicts the action" defect again, and "in progress" is conveyed by the amber fill, the filled dot and `opening_hint` instead (`sidebar::connect_button_label`).
- The two hints that used to share a row with the connect button (`opening_hint`, `need_port_to_connect`) are now **below** it — the button no longer fills the row, but the hint is a whole sentence and still will not fit beside it (nor beside its right margin). Guarded by `sidebar::tests::connect_button_hint_sits_below_the_button`, which compares the hint's `pos.y` against the button's `max.y` **from the same frame**; do not split the button rect and the text positions across two `run_ui` calls, or you are comparing two different layouts. Recognition in the shape list is a **size fingerprint** ("32 high **and** width within `130..=200`"): the height alone also matches the 32×32 `>>` sidebar-collapse button, which is exactly what the first version of that test grabbed; the width **ceiling** is what catches "it went back to filling the whole row" — a floor alone passes silently. And `ComTool::default()` already has COM1 selected, so a test asserting on the "pick a port first" hint must clear `selected_port` — otherwise it asserts on something that was never drawn.
- ⭐ **Every other button goes through `crate::button` — do not inline `ui.button` again** (2026-09-24: "make every button look like the connect button"). The scope is **interface command buttons**: the sidebar, the send area (send / clear / save-preset / the preset buttons), the chart panel, the settings window, and the send-history placeholder in the send-area header. Items inside pop-up menus — the menu bar's File/Settings/View/Text entries, the send-history dropdown's rows, and the receive area's right-click "copy all" — **deliberately keep the menu's own row-highlight style**: a solid block there widens the menu and fights its vocabulary. Two rules carry the user's explicit "must not cover unrelated widgets": the width hugs the label but is **clamped to `ui.available_width()`** (the ceiling must **not** be `max(available, min_width)` — that computes an over-wide button inside a narrow container, which is precisely the overflow the user was warning about), and the label is drawn through `painter.with_clip_rect(rect)`, so a container too narrow even for the minimum font size clips the glyphs inside the button instead of spilling them onto the next widget. Tones are `Primary` (green — 发送 / 保存常用数据 / 开始记录) and `Neutral` (everything else); there is deliberately **no red tone**, because five `清空 / 恢复默认` buttons in red would drown out the one genuinely destructive action. Tone reports the **action**: the logging toggle is green while it reads "start logging" and neutral once it reads "stop logging" — a green stop button suggests it starts something. Disabled buttons get two fixed colours per theme (not a multiply — that yields a *lighter* grey in the light theme, i.e. more prominent than an enabled button) and drop to `Sense::hover()`, so a greyed button can never be clicked; drive that from `ui.add_enabled_ui(..)` and let the drawing read `ui.is_enabled()`. Two sizes only — **28 high / 15 pt** alone on a line, **24 high / 14 pt** when sharing a row with a combo box (the port row's refresh button) — and the label font stays **above** `TextStyle::Body`. The connect button is the one exception that keeps its own drawing (its dot and arrow are that action's vocabulary, not the shared style). Guarded by `button::tests::*` (hover brighter / press darker; primary green and neutral genuinely grey; disabled closer to `panel_fill`; the two heights and two font sizes as **literals**; width follows the label; **width never exceeds the available space**; two buttons side by side neither overlap nor touch) plus `sidebar::tests::sidebar_buttons_use_the_shared_button_style`, which reads the painted shapes and requires at least three 28-high neutral-grey backgrounds — that last one guards against "the helper is right but the interface never calls it".
- UDP must never be described as "connected": it has no connection, and saying "connected" implies the peer is online and that a drop would be noticed. Use "bound / not bound" (`status_bound` / `status_unbound`). Related: UDP's `Ok(0)` is a zero-length datagram (valid traffic), whereas TCP's `Ok(0)` means the peer closed and must emit `ChannelEvent::Closed`. The bind address is `0.0.0.0:local_port` (not loopback), and because `local_port` may be 0 the actual port is exposed through `ByteChannel::local_addr()` (a trait method with a default of `None`, overridden only by UDP) and shown in the sidebar — the user has to know which port the device should reply to.
- Notices written into the receive log name the transport via `ComTool::transport_noun()` ("串口" / "TCP" / "UDP"). The notice text itself is deliberately **not** translated (see the string-table rule in `DEVELOPMENT_GUIDE.md` §6), but a TCP drop must not claim the serial port closed.
- ⭐ **TCP's write side must stay non-blocking** (`stream.set_nonblocking(true)` in `net_impl/tcp.rs::open`, plus `set_nodelay(true)`). Blocking-ness is **per fd**, so this also removes the read timeout — the worker loop therefore paces itself with the explicit `IDLE_SLEEP = 1 ms` and an `idle` flag (set when an iteration moved no bytes; `Ok(n)` and `Transmit` clear it so throughput is not throttled). Do **not** "fix" a wedged write by adding a send timeout: measured on Windows, a blocking `send` that times out may already have queued/sent the data while the caller only gets `WSAETIMEDOUT` and **no byte count**, so the worker re-sends the whole chunk — the same 64 KB chunk appeared **126** times on the wire. Non-blocking `Ok(n)` is byte-accurate and has no such ambiguity; serial has none either (the driver queues nothing it does not accept, so its 10 ms timeout is safe).
- **UDP stays blocking on purpose** (measured, not assumed): 4000 × 60 KB datagrams to a peer that never `recv`s produced **0** failures and a worst single `send` of **0.1 ms**, while the TCP control group hung for **2 m 05 s**. So UDP has no "peer stops reading ⇒ worker wedges" path, and non-blocking there would buy an unnecessary pace loop. Residual risk: a saturated **local NIC egress** can still block `send` (untested) — see `HANDOVER.md` §5.
- ⭐ **Every panel paints its own background — never let the window clear colour stand in for it.** A panel built with `Frame::NONE` has `fill: Color32::TRANSPARENT`: nothing is painted, and whatever `eframe::App::clear_color` returned shows through. eframe's *default* implementation returns a **hard-coded** dark gray `rgba(12,12,12,180)` and ignores the `Visuals` it is handed (its parameter is literally named `_visuals` — `eframe-0.36.2/src/epi.rs`), so any transparent area stays dark in the light theme. That is the 2026-09-24 「浅色主题下状态栏没变色」defect found by the user's own eyes: the status bar used `Frame::NONE`, so in the light theme it kept a near-black band while its labels turned `gray(80)` (`widgets.noninteractive.fg_stroke`) — dark text on a dark band, which reads as "the bar never changed colour *and* looks empty". In the dark theme the same band was `(12,12,12)` against a `panel_fill` of `gray(27)`, i.e. nearly invisible, which is why it survived so long. The status bar now uses `Frame::side_top_panel(ui.style())` — the same frame `Panel::resolve_frame` would have chosen on its own — overriding **only** the inner margin, and `ComTool::clear_color()` overrides the eframe default to return `visuals.panel_fill` as a backstop for regions no panel covers. Prefer `Frame::side_top_panel` / `Frame::central_panel` over a hand-written `.fill(ui.visuals().panel_fill)`: the value then tracks egui, and hard-coding surface colours is exactly how this bug happened. ⭐ **Why no test caught it**: they asked about dot count, label text, position and padding — none asked *what colour this panel's own background is*, and "painted nothing" is indistinguishable from "painted a transparent rect" unless you read the fill. Locked by `status_bar::tests::status_bar_background_follows_the_theme` (headless render per theme; find the full-width rect whose bottom edge is the screen's bottom edge — the panel background — and assert its fill **equals that theme's `panel_fill`**) and `app::tests::window_clear_color_follows_the_theme`. Assert **equality with the theme's fill**, never merely "the two themes differ": the weaker form is satisfied by any other hard-coded colour, which is precisely injections G2/G4 in `.workbuddy/tmp_probe_theme_bg.py`.
- Transport-specific ignorable errors stay in that transport, not in the shared helper: `worker::is_transient` lists only the three kinds every transport shares (`TimedOut` / `WouldBlock` / `Interrupted`). Windows reports ICMP port-unreachable on a connected UDP socket as `ConnectionReset`, so `udp.rs` adds that case itself — putting it in the shared function would make serial ignore a real fault.
- Control lines (RTS / DTR, F5): opening a port must **never** touch them. `open_connection()` only syncs the `rts` / `dtr` checkboxes to `false` and sends no `SetLineState` command — `serialport` on Windows already leaves both lines disabled on open (`windows/dcb.rs`), and pulling DTR high at open time resets a large class of dev boards while the user only wanted to look at the port. The checkbox means "**the level we requested**", not a measurement: `serialport` only offers writes, never reads, so the UI cannot know where the hardware actually drove the lines. Say that in the tooltip, the comments and the wording alike. Both checkboxes are **disabled until the channel is actually ready** — the gate is `ComTool::channel_is_ready()`, **not** `connected` (that flag only means "an open has been started", and it is true during the serial `OPENING` window, where a click would only queue a command in the worker while the box already shows as ticked). The reason they are greyed is **always visible** as a `ui.weak(...)` line under the row, matching what the panel-visibility checkboxes already did — `on_disabled_hover_text` alone is invisible unless the user happens to hover the dead box. `on_disabled_hover_text` was dropped for that reason; `on_hover_text` stays because it explains something else (requested vs measured level). Pinned by `sidebar::tests::control_line_checkboxes_follow_the_channel_not_the_connected_flag`, which compares the painted `Shape::Text::fallback_color` of "DTR" against a reference pair of enabled/disabled checkboxes drawn with the same default style (never against hard-coded colours), plus the presence/absence of the hint line; four injections confirmed it is non-vacuous (revert the gate to `connected`; make the boxes always enabled; hide the hint; always show the hint).
- Control-line failures are **not** disconnects: `ChannelEvent::Notice` is the non-fatal channel event. `receive_data` writes a note for it and deliberately leaves `connected` alone and keeps repeat-send running, because data keeps flowing while a line toggle may have failed. Use `Notice` for new "failed but not fatal" channel reports instead of relaxing `Failed`; `Failed` / `Closed` still mean the channel is gone and stop repeat-send.
- `ByteChannel::set_line_state(rts: Option<bool>, dtr: Option<bool>)` uses `Option` (not two bools) because the driver can only write: `None` means "leave that line alone". It has a default impl returning `Unsupported` (same precedent as `configure`); only `SerialChannel` overrides it, and TCP / UDP just ignore `WorkerCommand::SetLineState` in their command loops.
- Which baud entry is selected lives in its **own field** (`ComTool::baud_custom: bool`), never inferred from the value. Deriving it from "is `baud_rate` in `BAUD_RATES`" collapses under a user who picks Custom and then types a value that *happens* to be listed (9600): the dropdown jumps back to that entry and the input box **vanishes under their cursor**. Click semantics have exactly one implementation, the pure `sidebar::apply_baud_pick(baud_rate, baud_custom, BaudPick)`; the drawing block only accumulates an `Option<BaudPick>` and applies it after the closure — do not mutate those fields inline inside the ComboBox. `BAUD_CUSTOM_SEED` (250000) is a **seed, not a marker**: it is substituted only when Custom is picked *and* the current value is listed; an unlisted value (e.g. 74880 restored from an old archive) must be preserved. `baud_custom` never participates in building `SerialConfig` — `baud_rate` is the only real value handed to the driver.
- The settings window's content is **taller than one screen**, so it needs both halves of the fix: `Window::max_height(SETTINGS_MAX_HEIGHT)` (without it the window grows to fit the content and pushes the bottom controls off-screen with **no scroll bar reachable** — `Window` defaults to `ScrollArea::neither()`) and a `ScrollArea::vertical()` around the body. Use `auto_shrink([false, true])`: a vertical scroll bar eats width, and letting the area shrink to the content makes full-width widgets (`ui.separator()`) narrow whenever the bar appears, so rows visibly jump sideways. Prefer the content-level `ScrollArea` over `Window::vscroll()` — the latter puts the bar under the title bar and ties its look to the window frame. Give the scroll area an explicit `id_salt` (`SCROLL_ID`): the default id derives from the call site, so moving the code silently loses the saved scroll position. The body lives in `ComTool::settings_body` so a headless test can draw it inside a deliberately short viewport; assert the **documented scroll-need signal** (`ScrollAreaOutput.content_size > inner_rect`) plus "content really painted taller than the viewport" — asserting only "it drew without panicking" passes even with the scroll area removed.
- There is exactly one outbound path: `payload_for()` (encoding + line ending) → `transmit()` (write + error reporting + `tx_bytes` accounting). Every sender — the send button, the Enter key, repeat-send, and preset buttons — goes through it. Never write your own `channel.write` in a callback: you will silently lose either the failure notice or the byte count. `tx_bytes` is accumulated only **after** `write` returns `Ok` (same principle as "clear the input only if it was really sent"), and it resets on teardown.
- The bottom status bar (convention 48) must be drawn **before** `CentralPanel` — egui panels reserve space in draw order, so drawing it afterwards leaves it spanning only the central panel's remainder with visible gaps at both ends. Its decision logic lives in the pure `status_bar::connection_indicator`, and it must keep the sidebar's **four-state** vocabulary **plus the `opening` state**: for UDP, `connected == true` means the socket is merely *bound*, so showing "Connected" would tell the user a dead peer is online; for serial, `connected == true` together with `opening == true` means the port is **still opening**, so "Connected" would claim data can flow when it cannot. `is_pending` drives the third colour (amber) and is a **separate field** from `label` on purpose — brightness and wording are two dimensions, and packing them into one field turns "there happen to be three colours" into an implicit contract. **The dot is drawn, not a character** — use `ui.painter().circle_filled(..)` with `ConnectionIndicator::color()`. Never go back to the 🟢/🔴 emoji: egui rasterizes text by taking the font's *vector outlines* only (`epaint`'s `allocate_glyph_uncached` calls `skrifa`'s `outline_glyphs()`), so the `COLR`/`CPAL` colour layers are never read — the initial emoji version rendered as two differently-sized monochrome circles, and no font swap can fix that. Size the circle from `ui.text_style_height(&TextStyle::Body) * DOT_SIZE_RATIO` (0.72, compile-time asserted < 1) and centre it on the allocated rect's `center()` (the rect is line-height tall, taller than the diameter). The bar is **one centred group**, not left-status/right-readout: do not reintroduce a `right_to_left` split. Centre it by **computing the pad** (`add_space((avail - row_width) / 2)`) rather than by any declarative alignment API — the latter leave the row at x=0 in headless draws (see the module bullet in §2). The row is drawn twice (once into an `invisible` probe `Ui` to measure, once for real), so its content must stay in the single `status_bar_row` method or the measured width stops matching what is painted. Height comes from `BAR_PADDING_Y` via `Frame::NONE.inner_margin(..)`, and `show_separator_line(false)` removes the panel's own border line; both exist so the content does not look glued to the borders — keep them, and keep `BAR_PADDING_Y > 0` (compile-time asserted). This also removes the status bar's dependency on the font fallback chain entirely, so no emoji font entry is needed in `main.rs::setup_fonts`.
- The global UI font scale (`UiFontScale`) persists the **step** (`Small`/`Normal`/`Large`/`ExtraLarge`), never a pixel size — pixel values are an implementation detail, and storing them would let a baseline change "remember" a wrong step. `#[default]` must stay `Normal` (factor 1.0) so archives written before this feature load exactly as before. `apply_font_scale` recomputes **from the fixed `FONT_SIZE_BASELINE` table every frame** — never multiply the previous frame's value, which compounds exponentially and blows the UI off-screen within seconds. It uses `all_styles_mut` (both the dark and the light style must change, or switching themes snaps the size back) and `entry(..).or_default()` rather than replacing `text_styles`, so a `TextStyle` we have not registered keeps egui's own default. `egui::TextStyle` is **not** `Copy` — clone it as a BTreeMap key. "Global" has to actually reach everything: the three receive render paths and the send box used to hard-code `FontId::monospace(14.0)`, so they now go through `receive::monospace_font_id(ui, fallback)`, which reads `text_styles[TextStyle::Monospace]` from the live style; do not reintroduce a literal size, and do not make `build_layout`/`build_ascii_layout`'s font `Option`al (a silent default would let the scale die in one view unnoticed). A dropdown's selected text must be derived from the field (`font_scale_name(self.ui_font_scale, s)`), never a constant — and note that `font_scale_preview` ("预览：{}") legitimately echoes the current step's name, so "the painted text contains the step name" is a **vacuous** assertion; read the painted shapes in order and check the text right after the label instead.
- Presets (`SendPresets`, F5) deliberately do **not** evict when full — the opposite of `SendHistory`. History is collected automatically, so dropping the oldest is expected; presets were picked by hand, and silently removing the first one looks like data loss. `can_add()` is the single decision point (both the greyed-out button and `add()` call it). Labels are derived from the content by `preset_label()` (first line, truncated by **characters**) rather than stored.
- New drawing logic should be a `fn xxx(&mut self, ui: &mut Ui)` method (e.g. `render_send_presets`), not inline in `eframe::App::ui` — `ui()` needs an `eframe::Frame` that headless tests cannot build, so anything written there is never drawn by `cargo test`. Keep pure drawing side-effect free and assert it: `panels::tests::connected_sidebar_renders_without_panic` checks that drawing the sidebar issues **no** `set_line_state` call at all.
- The receive area's default text colour **must come from the theme** (`ui.visuals().text_color()`), never a hard-coded black. All three render paths (`show_plain_receive`, `render_color_receive`, `render_ascii_receive`) take it from there; hard-coding `Color32::BLACK` makes the whole log **invisible on a dark theme**. What keeps this from breaking ANSI support is `layout_job_from`'s `span.style.fg.unwrap_or(default_color)`: only bytes the device actually coloured override it. Those explicit colours (`ASCII_CTRL_COLOR`, `ASCII_NONPRINT_COLOR`, `TIMESTAMP_COLOR`) are data encoding, not theme decoration — do not "fix" them to follow the theme. Smoke tests know nothing about colour (a hard-coded black passes them happily), so colour is pinned by asserting `LayoutJob.sections[].format.color` directly.
- Panel visibility has an invariant: at least one of `sidebar_open` / `show_receive_panel` / `show_chart_panel` / `show_send_panel` must stay visible (`visible_panel_count() >= 1`). The view menu disables un-checking the last visible panel, and `restore_runtime_state` repairs hand-edited archives. A new panel must be added to `visible_panel_count`, the view menu, and that repair.
- Drawing order in `ComTool::ui` is load-bearing: top menu bar → right sidebar → `CentralPanel` (must be last), and inside the central panel the order is receive → chart → send. Panels claim space in the order they are added.
- Theme preference is persisted as the crate's own `ThemeChoice`, never as `egui::ThemePreference` (a foreign enum whose variants can change). `apply_theme` pushes it to the context only when it differs from the active value.
- Adding a language is a **data change, not a code change**. `src/strings.rs`'s `langs:` header is the single source of truth for the language list (`ZH, EN, ZH_HANT, JA, ES, IT, PT, ID, FR, DE, KO, VI, TR, RU, PL`); every entry carries one literal per language in that order, and the macro emits a `compile_error!` when a row has the wrong number of columns. The three steps are: add the ident to `langs:`, append one literal to **every** entry, and append a `Language` variant + `Language::ALL` + `endonym()` in `app.rs`. `Language::ALL`, the `LANGS` mirror and the table's column order are cross-checked by `tests::langs_table_matches_language_order` — a mismatch there shows the whole UI in the wrong language and still **compiles**, so that test is load-bearing. Variants are `Serialize`/`Deserialize` by name: only append. Guard tests must be written per-language and validated by deliberately breaking the thing they guard (a `{}` dropped from one column, a column pasted from another language, the whole zh-Hant column copied from zh) — assert on the **mechanism's own signal**, not "it drew without panicking". A test that iterates a hard-coded pair of languages is the classic way to write a vacuous multilingual guard. **A script detector must be added for every new script** — the `columns_have_no_foreign_script` set originally had no Cyrillic member, so an entire `ru` column pasted from Polish passed unnoticed; `has_cyrillic` closes that hole, and `third_batch_columns_use_their_own_alphabets` asserts from the **data itself** that `ru` really is Cyrillic and `pl` really carries the Polish-exclusive letters `ą ć ę ł ń ś ż` (a hard-coded "looks Polish" table is unreliable — `ó`/`ź` also occur in Portuguese/Spanish).
- Multilingual fonts go in **both** families. `main.rs::setup_fonts` appends each font to `FontFamily::Proportional` *and* `FontFamily::Monospace`; adding it to only one leaves the receive area (monospace) rendering tofu. Measured coverage (recorded in the function's comments): msyh/simsun/simhei cover CJK but **not** the Vietnamese precomposed-diacritic block (U+1EA0–U+1EF9) nor Hangul; `tahoma.ttf` covers Vietnamese and Thai; `malgun.ttf` covers Hangul. `tests::every_vietnamese_character_has_a_glyph_in_the_font_chain` walks the column's **actual** character set (not a sample) against the chain. Colour emoji still cannot be rendered — see the status-bar rule.
- `SerialConfig` defaults to 115200 baud, 8 data bits, no flow control/parity, one stop bit. It is passed to the channel via `ByteChannel::configure` before `open`.
- Persistence: `ComTool` derives `Serialize`/`Deserialize` with a container-level `#[serde(default)]`. Only user preferences are stored; serial handles, the port list, connection state, the receive log, `ChartData`, and the running `LogSink` are `#[serde(skip)]` and rebuilt by `config::restore_runtime_state` on load. Add new runtime-only fields to both the `skip` list and that function. Removing a persisted field is safe (unknown keys are ignored — keep `config::tests::archive_with_a_removed_field_still_loads` green, it is what stops anyone from adding `deny_unknown_fields`), and a new preference field must be added to `Default`, to `customized()` in the config tests, and to that round-trip assertion.
- Do not ship a preference that nothing consumes (a checkbox that changes nothing is worse than a missing feature). P6 deliberately drops the version check, and `check_update_on_start` was removed together with its settings section rather than left in the UI as a dead switch.
- Tests are inline: parser/renderer/trimming/timestamp tests at the bottom of `src/receive.rs`, chart parsing/buffer/limit tests at the bottom of `src/chart.rs`, logging/format/naming/disk tests at the bottom of `src/log_file.rs` (they use a private temp-dir helper, no extra crate), channel contract tests at the bottom of `src/serial_impl/channel.rs`, persistence and load-invariant tests at the bottom of `src/config.rs`, headless UI smoke tests (menu bar, settings/about windows, all three receive views, the sidebar, the *connected* sidebar, the preset row, the chart panel) at the bottom of `src/panels/mod.rs`, icon geometry/color/alpha tests at the bottom of `src/app_icon.rs`. Run them with `cargo test` (or e.g. `cargo test config::tests::` for one module) — none of them require serial hardware, and the UI smoke tests use `egui::__run_test_ui` so they need no window either. `receive::tests::feed` injects bytes through the event channel, which is the same path a real port takes (no hardware needed). The network channels are covered at the bottom of `src/net_impl/{config,tcp,udp}.rs` — two of them talk over loopback (`TcpListener` / `UdpSocket`), so they need no hardware and no GUI either. `app.rs`'s send-path tests inject a `FakeChannel` through `connected_app_as(kind)`. A test may call `scan_ports()` (plain enumeration, no port is opened) — `config.rs` and `serial_impl/channel.rs` already do. `src/sidebar.rs` keeps tests for its pure helpers at the bottom too (`apply_baud_pick`'s four click semantics), plus (since the 2026-09-24 layout refactor) three **drawing** tests that read `Context::run_ui`'s painted text: `sidebar_no_longer_shows_the_byte_counters` (the counters must have *left* the sidebar — the subtraction half of a "move" refactor is the easy one to forget), `available_ports_render_inside_the_serial_info_section` (the port list is merged in, the old `接收统计` header text is gone), and `sidebar_keeps_display_mode_and_timestamp_lines` (those two stayed behind rather than being swept into the status bar with the counters). The settings window's scroll behaviour is pinned by `panels::tests::settings_body_renders_inside_a_short_viewport`, which draws `settings_body` inside a 120px-tall viewport and asserts the content really overflows it. The global font scale (`UiFontScale`, convention 47) has three guard tests: `app::tests::font_scale_baseline_matches_egui_defaults` (bidirectional diff against `egui::Style::default().text_styles`), `app::tests::font_scale_is_idempotent_across_frames` (repeated application must not compound — asserts the **absolute** value, not just first == last), and `panels::tests::font_scale_dropdown_is_wired_to_the_field` (reads the painted shapes in order and checks the text right after the label is the current scale's name). `receive::tests::monospace_font_id_follows_the_style_not_a_constant` pins the single hinge that makes the scale reach the receive views and the send box; `receive::tests::monospace_font_id_falls_back_when_the_style_lacks_it` pins the fallback path. The status bar's decision table is pinned by `status_bar::tests::indicator_covers_all_five_states`, `the_dot_changes_colour_with_the_state`, `indicator_follows_the_language`, `status_bar_agrees_with_the_sidebar_on_udp` and `status_bar_agrees_with_the_sidebar_on_opening`; `status_bar_paints_the_indicator` reads the painted shapes so the bar cannot compute the indicator and silently drop it, and `status_dot_is_painted_as_a_filled_circle_in_the_right_colour` reads `Shape::Circle` (radius + fill) to require exactly one circle in `indicator.color()` — that catches both "not painted" and "colour hard-coded" (both were verified by deliberately breaking them). `dot_size_tracks_the_line_height_but_stays_smaller_than_it` back-computes the line height from the painted radius so a broken size formula cannot pass. `status_bar_paints_live_byte_counters` and `status_bar_byte_counters_change_with_the_data` pin the byte counters into the bar: the first builds the expected string with `fill()` (so a label that is painted but carries a constant 0, or rx/tx swapped, both fail), the second draws two different datasets and requires the painted strings to differ (a hard-coded value cannot pass). The 2026-09-24 layout pass — making the bar taller and centring its group — is pinned by two **geometry** tests that read the painted text boxes (not just their text): `status_bar_is_taller_than_one_line_of_text` asserts the bottom gap between the text and the screen edge is at least `BAR_PADDING_Y` (removing the `Frame` margin collapses it), and `status_bar_content_is_horizontally_centred` asserts the group's painted centre is within 12 pt of the container centre (the 12 pt covers the fact that the dot is geometry, not text, so a text-only measurement sits a few points right of the true centre; left/right/right-aligned implementations miss by hundreds of points, so the line still separates them). Both were proven non-vacuous by injecting the four corresponding bugs (drop the margin; pad = 0; pad = full remainder; row width hard-coded) and confirming each turns its test red. The 2026-09-24 removal of the menu bar's Send menu is pinned by a **pair** of tests in the same module, because either one alone leaves a hole: `menu_bar_has_no_send_menu` drives `show_menu_bar` and asserts the top-level button row no longer contains `send_title` **and** still contains File / Settings / View / Text (so it fails both when the menu comes back and when the bar is gutted for an unrelated reason — asserting against the whole frame instead would be vacuous, since the send button and the settings heading also render 「发送」); `send_area_header_keeps_the_send_settings` draws `render_send_settings_row` and requires the format/line-ending options and the repeat-send checkbox to still be there, so "delete the menu" cannot quietly become "delete the feature". Both are run in Chinese and English. Four injections confirmed they are non-vacuous: re-add the `send_title` menu button (first goes red), reduce the bar to File only (first goes red, proving the "kept menus" half is live), drop the line-ending radios (second goes red), and stop drawing the header row entirely (second goes red). The same day's removal of the central panel's duplicate `⚙ Settings` button is pinned by the matching pair `top_toolbar_has_no_settings_button` (drives `render_top_toolbar` alone — asserting against the whole frame would be vacuous because the menu bar's own 设置 menu title and the settings window's title both contain that word; it requires the button text to be absent **and** the sidebar toggle to still be painted) and `menu_bar_still_offers_settings` (drives `show_menu_bar` and requires the Settings menu plus File / View / Text, so "delete the duplicate" cannot become "delete the only entry point"). Both run in Chinese and English, and four injections confirmed them: re-add the button (first goes red), blank the whole toolbar row (first goes red, proving the "toggle still there" half is live), drop the Settings menu (second goes red), drop the Text menu (second goes red). The 2026-09-27 「总在最前端」 entry in the View menu is pinned the same way: `panels::tests::view_menu_offers_always_on_top` drives `view_menu_body` (extracted from the menu closure because a menu body is only painted while the menu is open — simulating the open click would make the assertion depend on button coordinates), and `always_on_top_sends_the_window_level_command` reads `FullOutput::viewport_output[&ViewportId::ROOT].commands` for `WindowLevel(AlwaysOnTop)`, for the absence of a repeat while the state is unchanged, and for an explicit `Normal` when the box is cleared; the persistence half lives in `config::tests::preferences_survive_round_trip`. Five injections confirmed they are non-vacuous: drop the apply call, send `Normal` while ticked, drop the de-dup guard, drop the checkbox, and `#[serde(skip)]` the field. Note `apply_always_on_top` hangs off `show_menu_bar` rather than `ui()` **purely** because `ui()` takes an `eframe::Frame` no headless test can build — the menu bar is painted every frame, so the behaviour is identical. Unlike `apply_theme`, it cannot ask egui for the current value: window level is a write-only command, so the last-applied value is kept in `applied_always_on_top` (`#[serde(skip)]`, `None` at startup, which is also what makes a persisted 'on top' take effect on the first frame). The 2026-09-27 receive-area **filter** (`filter_lines` / `filter_span_lines`, `HISTORY_2026.md` §3.31) is a *display-layer* transform: the three mirrors (`receive_text`, `display_spans`, `ascii_spans`) are left untouched so export, the live log file, chart parsing and the `max_log_chars` trim are all unaffected — pinned by `receive::tests::filtering_never_touches_the_stored_log`. Because the text view renders `receive_text` while the colour/byte views render spans, there are **two implementations**, and they are cross-checked against each other on the same input (same kept text, same two counters) so the two view families cannot drift apart. Both return `(kept, matched_lines, total_lines)`; the text one returns `Cow::Borrowed` when the needle is empty (zero-copy while filtering is off) and the span one is skipped entirely at the call site (never clone the span table for nothing). Matching is per **logical line** (`
`), never per wrapped visual line, and a trailing newline-less half line counts as a line. Four end-to-end tests drive the real `render_receive_area` and read the **painted** text, plus two UI tests pin the row (absent by default, and painted *above* the receive content by y-coordinate) and the toolbar counter's three states; four injections were confirmed red (render the unfiltered mirror while still filtering; drop the counter write; never draw the row; never show the counter).
- The 2026-09-24 fix for the three long-standing defects (§5) carries its own guards. **Backpressure**: `communicationtrait::worker_tests::{ledger_hands_out_quota_up_to_the_cap, ledger_counts_total_bytes_not_entries, ledger_release_returns_the_quota, ledger_reset_clears_the_quota, retiring_a_finished_chunk_pops_it_and_returns_the_quota, retiring_on_an_empty_queue_is_a_no_op, submit_refuses_when_the_ledger_is_full_and_queues_nothing, submit_hands_the_bytes_to_the_worker_and_charges_the_ledger, submit_returns_the_quota_when_the_worker_is_gone}` — the refusing test asserts `rx.try_recv().is_err()`, i.e. **not one byte reached the worker** (a "send then error" half-implementation would otherwise pass), and the release/retire tests pin that quota comes back (a queue that only shrinks reports "full" forever). Each channel adds `oversized_payload_is_refused_at_the_call_site` (the refusal is synchronous, at the caller) and `reopening_the_channel_clears_the_stale_quota` (the `open()`-resets coupling), and `net_impl::tcp::tests::close_returns_promptly_even_when_the_peer_never_reads` pins the non-blocking write — it must **first saturate the kernel send buffer** (sustain 2 s of 64 KB writes, require ≥ 2 MiB accepted *and* ≥ 1 refusal) because a full ledger alone does **not** wedge the worker; without that step the test passes even with blocking writes and is vacuous. **Backoff**: `app::tests::{reconnect_backoff_doubles_until_capped, channel_failure_increments_reconnect_failures, receiving_bytes_resets_reconnect_failures, reconnect_gives_up_after_max_attempts, explicit_close_resets_reconnect_failures}` — the first asserts **absolute** durations (500/1000/2000/4000/8000/16000/30 s), not just monotonicity. **`connected` false positive**: the serial state machine is pinned at the bottom of `src/serial_impl/channel.rs` by `is_ready_is_false_until_the_port_is_actually_open` (checks the not-ready answer both before and after a failed open), `open_transitions_through_the_opening_state` (reads the state bits directly and forbids `ACTIVE` in the window after `open()` returns — the result-only test can be fooled by an implementation that stores `IDLE` instead of `OPENING`), and `is_ready_agrees_with_is_active_outside_the_opening_window`; the app-layer contract is pinned by `app::tests::{serial_open_does_not_claim_connected_before_the_port_is_ready, sending_is_blocked_while_the_port_is_still_opening, settling_never_downgrades_an_established_connection, settle_wording_follows_the_connection_kind_not_the_channel_kind, settle_announces_open_only_once_per_connection}`, which drive `FakeHandle::defer_ready()` / `signal_ready()` — a mode added to the test double so it can model "serial `open()` returns before the port is open". Without that mode the double is always synchronously ready and the whole app-layer contract is untestable. Five injections across these (store `ACTIVE` instead of `OPENING`; guard sends on `connected`; force `opening()` to false; drop the `announced_open` bit; delete the status bar's `opening` arm) each turned at least one test red. Note the fake's `is_ready()` deliberately mirrors `is_active()` and is driven by its own `active` flag, so the "store `ACTIVE` instead of `OPENING`" injection does **not** hit the two app-layer tests — by design, they cover the app's handling of a not-ready channel, not `SerialChannel`'s internals, which are covered by the serial-channel tests. The ledger/non-blocking round added four more injections (drop `set_nonblocking`; retire without releasing; `submit` stops reserving; drop `open()`'s `ledger.reset()`), all of which go red — run them with `.workbuddy/tmp_probe_ledger.py`, and when you touch that harness remember it must treat only `error[E` / `could not compile` as a compile failure (the trailing `error: test failed, to rerun pass …` is a *test* failure and was once misread as "injection too aggressive", hiding three genuine reds).
- Verification commands: `cargo check`, then `cargo test`; use `cargo fmt --check` to check formatting. `cargo clippy --all-targets` **is gated at 0 warnings** — the 8 historical style warnings (`derivable_impls`, `manual_range_contains` ×3, `needless_option_as_deref`, `unnecessary_map_or`, `needless_borrow`, `wrong_self_convention`) were cleared on 2026-09-26; new code must not reintroduce any.
- `tools/backpressure_check.py` is the out-of-tree verifier for the backpressure item (F5 71, `HANDOVER.md` §5). It never launches the app; it plays the **peer** — loopback TCP with a throttled read (zero hardware), or one side of a virtual serial pair — samples the app's commit/working-set through `ctypes` (no `psutil`), and prints a pass/fail plus the things only a human can see. `--self-test`'s third check deliberately uses a **blocking** socket to re-derive why `TcpChannel` had to become non-blocking (a peer that stops reading blocks a blocking `send` forever; a slow reader keeps it returning) without touching the app or a serial port — run it after changing the script. Its sibling `tools/backpressure_dummy_sender.py` is **not** the app: it is a stand-in that reproduces the send-side *structure* (UI thread → mpsc; worker → the same 1 MiB byte cap) so the script's verdict logic can be exercised in both directions. Two traps that run exposed and are worth remembering: a harness that reuses one 32 KB `bytes` object across 2048 queue entries makes the memory criterion **falsely green**, and a harness that enqueues in the *same* loop as its writes does not model the real split at all (so its queue can never grow). Verify a new criterion in **both** directions before trusting it.
- **Compile before asking for a visual review.** `fmt` / `check` / `test` / `clippy` produce **no executable**, so after running only those, `target/release/comtool-rs.exe` is either stale or absent and the user would be double-clicking a build that does not contain your change. Any deliverable whose correctness can only be judged by eye (new panel, control, colour, font, icon) requires `cargo build --release` *before* you say "please take a look", plus a timestamp check that the exe is newer than the last `src/` edit. Prove it from timestamps — `git log -1 --format=%cd --date=iso`, `ls -l --time-style=full-iso src/*.rs`, and the same on `target/release/` — never from "I ran a cargo command". `target/` is gitignored, so the exe never arrives with a clone and must be rebuilt after every `target/` wipe or machine change. Never say "just build it yourself" or "the tests pass, have a look". See `DEVELOPMENT_GUIDE.md` §6 convention 49.
- The chart draws with `egui_plot`. Its version number is **not** in lockstep with egui: `egui_plot 0.37` needs `egui ^0.36` whereas `egui_plot 0.36` needs `egui ^0.35`. Before bumping either crate, check the actual requirement (`cargo tree -i egui`, or the local `registry/src/**/egui_plot-*/Cargo.toml`) instead of assuming the numbers match.
- `DEVELOPMENT_GUIDE.md` records the verified state/default/UI wiring conventions; consult it for changes that add persistent `ComTool` state, and initialize every new field in `Default`.
- Repository-local OpenCode agents are configured in `opencode.json` and `.opencode/agents/`; the executor workflow expects code changes followed by `cargo check`/`cargo test`.
- Licensing: `SerialGUI-rs/` (GPLv3) is a reference implementation only and is gitignored. The serial modules were independently re-implemented to reduce derivation risk; do not copy code back from that directory. **This project itself ships as `GPL-3.0-only`** — `LICENSE` (full text) + `NOTICE.md` (derivation/modification notice, measured verbatim-overlap report, redistribution checklist) + `Cargo.toml`'s `license` field must be kept in sync; `NOTICE.md` §3 records the measurement (`tools/license_overlap_check.py`, 106 identical lines, all framework-mandated boilerplate).
