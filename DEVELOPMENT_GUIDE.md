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
  `cargo build` 等不启动 GUI、不触碰串口的命令。79 条测试全部不需要串口硬件，
  所有验收一律以这些命令为准。
- **确实需要运行时**：停下来说明原因并请求批准，由项目所有者自行运行；不得"先跑再报"。
- 一次批准只对当次生效，不构成对后续轮次的授权。

> 完整条文见 `AGENTS.md` 顶部「最高优先级红线」一节，两者冲突时以 `AGENTS.md` 为准。
> 注意 `AGENTS.md` 在 `.gitignore` 中，属本机文档，不会随 git 传播。

### 0.1 已配置机器级硬拦截（PreToolUse hook）

上面的禁令不只是文档约定，已经装了钩子：**命中即拒绝工具调用**。

| 项 | 位置 |
|---|---|
| 拦截逻辑 | `~/.workbuddy/hooks/comtool-run-guard.sh` |
| 注册位置 | `~/.workbuddy/settings.json`、`~/.codebuddy/settings.json` → `hooks.PreToolUse`，`matcher = Bash/PowerShell/Write/Edit` |
| 自测 | `python3 .workbuddy/hooks/test_guard.py`（39 条断言；不启动程序、不碰串口） |
| 批准 | `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（**用户本人在 agent 之外执行**；`0` 撤销） |

- 批准文件 `<配置目录>/combot-run-approval` 第一行是过期 Unix 秒，不存在/过期/损坏 → 全部拒绝；
  **批准只放行"运行本项目"，COM3 操作永远拒绝**。
- 批准文件与批准脚本 AI 无权触碰（会被守卫自己拦掉）。
- 守卫 fail-closed：payload 解析失败或 stdin 为空一律拒绝。
- 已知误拦（改用别的工具即可）：命令文本里出现 `cargo run` / `COM3` / `comtool-rs.exe` 字面量。
  例如 `grep "cargo run" AGENTS.md` 会被拒，请用 Grep 工具；把该短语写进 commit message 也会被拒，
  可先 Write 一个消息文件再 `git commit -F <file>`。

---

## 1. 项目架构概览

本项目采用 **egui 即时模式（immediate mode）UI**，全部状态集中在一个核心结构体 `ComTool` 上，逻辑按职责分到几个模块：

| 层 | 位置 | 职责 | 典型例子 |
|---|---|---|---|
| **状态层** | `struct ComTool`（`src/app.rs`） | 定义功能所需的跨帧/跨调用状态字段 | `connected`、`display_spans`、`color_view` |
| **初始化层** | `impl Default for ComTool`（`src/app.rs`） | 给所有状态字段设默认值（漏了会编译报错） | `color_view: false`、`baud_rate: DEFAULT_BAUD_RATE` |
| **业务逻辑层** | `impl ComTool`（`src/app.rs` / `src/receive.rs` / `src/log_file.rs` / `src/chart.rs`） | 串口生命周期、数据解析、布局构建、日志写盘、数值流解析 | `open_port()`、`receive_data()`、`build_layout()`、`start_logging()` |
| **UI 层** | `impl eframe::App for ComTool`（`src/app.rs`）+ `show_sidebar()`（`src/sidebar.rs`） | 绘制控件并把 UI 事件接到业务方法上 | `ui.button("发送").clicked()` |
| **持久化层** | `src/config.rs` | 决定哪些字段落盘、哪些在启动时重建 | `load()`、`save()`、`restore_runtime_state()` |

### 模块一览

| 文件 | 职责 |
|---|---|
| `src/main.rs` | 入口；`APP_TITLE` 常量；字体回退链配置；`eframe::run_native` |
| `src/app.rs` | `ComTool` 状态、`Default`、`ThemeChoice`、串口生命周期、视图级辅助方法、`eframe::App` 顶层编排 |
| `src/receive.rs` | 接收链路：四份镜像状态、`AnsiDecoder`、字节级可视化、三种视图渲染、`max_log_chars` FIFO 淘汰 |
| `src/log_file.rs` | 实时文件日志：后台写盘线程（`LogSink`）、文件名模板渲染、同名防截断、待写队列上限 |
| `src/chart.rs` | 实时图表数据层：原始字节 → 多通道数值序列（`ChartData`）、跨包分片、样本 FIFO、非数值计数；不依赖 `egui` |
| `src/sidebar.rs` | 右侧边栏；返回 `SidebarAction` 而不是在绘制过程中改状态 |
| `src/panels/mod.rs` | 面板模块声明 + 无头 UI 冒烟测试 |
| `src/panels/menu_bar.rs` | 顶部菜单栏（文件 / 设置 / 视图）；返回 `MenuAction` |
| `src/panels/settings.rs` | 设置窗口：把用户偏好接到真正的控件上 |
| `src/panels/info.rs` | 信息窗口：版本、技术栈、存档路径、快捷键 |
| `src/panels/chart.rs` | 实时图表面板：`egui_plot` 绘制、图例、绘制点抽样（`points_for_drawing`） |
| `src/config.rs` | 持久化策略 + 启动时不变量修复 |
| `src/communicationtrait.rs` | `ByteChannel` trait 与 `ChannelEvent` |
| `src/serial_impl/channel.rs` | 串口实现：后台线程 + 命令/事件双通道 |
| `src/serial_impl/config.rs` | `SerialConfig` 与波特率档位 |

---

## 2. 数据流总览

```
串口后台线程 --(ChannelEvent::Bytes)--> ComTool.event_rx
        │
        ▼
receive_data()                     ← 业务层（src/receive.rs）：拉取原始字节
   ├─ 原始字节 → chart.push_bytes()  → 多通道数值序列（与显示模式无关，见 §2.2）
   ├─ HEX 模式    → 逐字节转文本        → push 进 display_spans
   ├─ 字节查看    → ascii_spans_from_bytes → push 进 ascii_spans
   └─ 文本模式    → decoder.feed() 解析 ANSI 颜色 → push 进 display_spans
        │        （三条分支都同步 append 到 receive_text）
        │        （每条分支的文本同时经 log_append() 投给实时文件日志，见 §2.1）
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

> **同一份接收数据维护四份镜像**：`receive_text`（纯文本，供导出与普通视图）、`display_spans`（彩色片段）、`ascii_text` / `ascii_spans`（字节可视化）。四者必须同步清空、同步追加——`ComTool::clear_receive()` 是唯一的复位入口。裁剪只作用于这四份内存镜像；磁盘上的实时日志文件是另一条旁路（见 §2.1）。

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
| 打开/关闭串口 | `connected`、`serial`、`event_rx`、`last_opened_port` | `open_port()` / `close_port()`（`app.rs`） | 侧边栏「打开串口」/「关闭串口」按钮 |
| 自动重连 | `last_reconnect_attempt` | `auto_reconnect()`（`app.rs`） | 无（每帧自动尝试，500 ms 节流） |
| 接收数据显示 | `display_spans`、`receive_text`、`decoder` | `receive_data()`、`build_layout()`（`receive.rs`） | 接收区滚动区 + 「颜色」勾选框 |
| 字节查看视图 | `ascii_text`、`ascii_spans` | `render_ascii_receive()`（`receive.rs`） | 「字节查看」勾选框 |
| HEX 模式 | `hex_view` | `receive_data()` 的 HEX 分支 | 「HEX / 文本」勾选框 |
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

---

## 6. 开发注意事项

1. **不是所有功能都要加字段**：只有需要"跨帧/跨调用共享"的数据才进结构体。纯计算/纯展示（如格式化字符串）直接写方法或局部变量即可，避免污染状态。

2. **新增字段务必同步 `impl Default`**：egui 结构体大多通过 `Default::default()` 创建，漏初始化会直接编译失败。

3. **新增字段务必同步持久化策略**：见 §3 步骤 ③。`ComTool` 带容器级 `#[serde(default)]`，所以新增字段本身不会导致旧存档读不出来；但如果它是运行时状态却没有 `#[serde(skip)]`，就会把一个无意义的快照写进磁盘，并在下次启动时带着过期值出现。运行时字段还必须加入 `config::restore_runtime_state` 的重建列表。

4. **UI 事件与业务逻辑分离**：把数据处理写在 `impl ComTool` 里，`ui()` 只负责画控件和调用方法。侧边栏尤其注意——它在 `sidebar.rs` 里只记录「用户按了什么」，真正的状态变更由 `app.rs` 统一派发。

5. **渲染用即时数据更新**：`ui()` 开头调用 `self.receive_data()` 拉取最新数据（或由 `render_color_receive` 内部调用），配合 `request_repaint_after` 实现持续刷新。

6. **一次性提示不要每帧追加**：类似"串口已打开/已关闭"这类提示，应在事件发生（如 `open_port`/`close_port`）时追加到 `display_spans`，而不是在渲染函数里按 `self.connected` 每帧写入，避免日志里不断重复。

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

17. **实时文件日志的写盘只能在后台线程**（`log_file.rs::worker_main`）：UI 线程不许对日志文件做任何 `write`。停机靠投 `Halt`，且 `Drop` 里必须用 **`try_send`**——队列满时 `send` 会阻塞，一个卡住的磁盘就能让"停止记录"把 UI 线程永久挂住。文件由 UI 线程 `File::create` 打开后再交给线程：这是刻意不同于 `open_port()`「先乐观置位、失败再回滚」的写法（那个假阳性缺陷还没修），不要改回去。

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
