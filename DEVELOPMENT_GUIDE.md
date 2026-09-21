# comtool-rs 开发参考文档

> 本项目是一个基于 `eframe`/`egui` 的串口调试工具。本文档描述当前项目的代码组织方式、功能开发的完整流程，以及各层职责划分，作为新增功能的参考。

---

## 1. 项目架构概览

本项目采用 **egui 即时模式（immediate mode）UI**，全部状态集中在一个核心结构体 `ComTool` 上，逻辑按职责分到几个模块：

| 层 | 位置 | 职责 | 典型例子 |
|---|---|---|---|
| **状态层** | `struct ComTool`（`src/app.rs`） | 定义功能所需的跨帧/跨调用状态字段 | `connected`、`display_spans`、`color_view` |
| **初始化层** | `impl Default for ComTool`（`src/app.rs`） | 给所有状态字段设默认值（漏了会编译报错） | `color_view: false`、`baud_rate: DEFAULT_BAUD_RATE` |
| **业务逻辑层** | `impl ComTool`（`src/app.rs` / `src/receive.rs`） | 串口生命周期、数据解析、布局构建 | `open_port()`、`receive_data()`、`build_layout()` |
| **UI 层** | `impl eframe::App for ComTool`（`src/app.rs`）+ `show_sidebar()`（`src/sidebar.rs`） | 绘制控件并把 UI 事件接到业务方法上 | `ui.button("发送").clicked()` |
| **持久化层** | `src/config.rs` | 决定哪些字段落盘、哪些在启动时重建 | `load()`、`save()`、`restore_runtime_state()` |

### 模块一览

| 文件 | 职责 |
|---|---|
| `src/main.rs` | 入口；字体回退链配置；`eframe::run_native` |
| `src/app.rs` | `ComTool` 状态、`Default`、串口生命周期、`eframe::App` 顶层编排 |
| `src/receive.rs` | 接收链路：三份镜像状态、`AnsiDecoder`、字节级可视化、三种视图渲染 |
| `src/sidebar.rs` | 右侧边栏；返回 `SidebarAction` 而不是在绘制过程中改状态 |
| `src/config.rs` | 持久化策略 |
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
   ├─ HEX 模式    → 逐字节转文本        → push 进 display_spans
   ├─ 字节查看    → ascii_spans_from_bytes → push 进 ascii_spans
   └─ 文本模式    → decoder.feed() 解析 ANSI 颜色 → push 进 display_spans
        │        （三条分支都同步 append 到 receive_text）
        ▼
display_spans / ascii_spans         ← 状态层：片段缓存（内存）
        │
        ▼
build_layout() / build_ascii_layout() ← 业务层：转成 egui 的 LayoutJob
        │
        ▼
egui::Label::new(job).wrap()        ← UI 层：渲染多色文本
```

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

> **同一份接收数据维护三份镜像**：`receive_text`（纯文本，供导出与普通视图）、`display_spans`（彩色片段）、`ascii_text` / `ascii_spans`（字节可视化）。三者必须同步清空、同步追加——`ComTool::clear_receive()` 是唯一的复位入口。

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
| 清空接收 | 全部三份镜像 | `clear_receive()`（`receive.rs`） | 侧边栏「清空日志」按钮 |
| 导出接收数据 | `receive_text` | `export_receive_text()`（`app.rs`） | 侧边栏「导出日志」按钮 |
| 发送数据 | `send_text` | `send_data()`（`app.rs`） | 「发送」按钮 / Enter 键 |
| 端口枚举 | `ports`、`selected_port` | `refresh_ports()`（`app.rs`）、`scan_ports()`（`serial_impl`） | 侧边栏「刷新」按钮 |
| 侧边栏显示/收起 | `sidebar_open` | `show_sidebar()`（`sidebar.rs`） | 「☰ 侧边栏」开关 / 侧边栏「>>」按钮 |
| 状态持久化 | 全部偏好字段 | `config::load()` / `save()` | `eframe::App::save()` 自动触发 |

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
