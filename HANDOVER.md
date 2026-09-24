# 交接文档 — AI_comtool-rs

> 面向下一个会话的接手者。读完这一份应该就能直接开工，不需要回头翻聊天记录。
> 生成时间：2026-09-21

---

## 1. 你在哪 / 两份代码的关系

工程被拆成两个**互相独立**的目录，各自是完整可编译的项目：

| 目录 | 角色 | 当前分支 | 当前提交 | `.git` 体积 |
|---|---|---|---|---|
| `E:\Downloads\Zed\AI_comtool-rs` | **AI 重构版**（本目录，主要工作对象） | `ai-refactor` | 见 `git log --oneline -1`（**刻意不在此处钉死哈希**：每提交一次就会过期，为更新它再提交一次只会立刻再次过期） | 243 KB |
| `E:\Downloads\Zed\comtool-rs` | **原版保留**（已回滚为干净状态） | `master` | `04dc007` | 785 MB |

> 本文件的提交号以撰写时的 `git log --oneline` 为准；要精确核对请自己跑一遍。

### `AI_comtool-rs` 的两个分支

| 分支 | 内容 | 说明 |
|---|---|---|
| `ai-refactor` ← **已检出** | AI 重构成果 | `280ec77`，一个提交，16 个文件 |
| `master` | 原版保持 | `04dc007`，与原版目录的 `master` **同一个提交** |

`master` 分支的存在意义是：随时可以 `git diff master ai-refactor` 看待改了什么，
或 `git checkout master` 回到重构前的状态对照行为。两个项目之间**没有 git remote**
（刻意移除，避免误推误拉）。

### 原版目录里的安全网

`comtool-rs` 的 `git stash list` 里有一条：

```
stash@{0}: On master: AI 重构完整工作区快照（已搬迁至 ../AI_comtool-rs 分支 ai-refactor）
```

那是搬迁前完整工作区的快照，纯保险。**正常情况下不需要它**——同样的内容已经作为
`280ec77` 提交在 `ai-refactor` 分支上。确认无误后可以 `git -C E:\Downloads\Zed\comtool-rs stash drop`。

### 没被搬迁的东西

- **`target/`（3.2 GB 构建缓存）刻意没复制。** 在 `AI_comtool-rs` 首次 `cargo check`
  需要约 1 分钟重新编译依赖，之后正常。原版目录的 `target/` 原样保留。
- **`comtool-rs` 的 `.git` 有 785 MB**，其中约 778 MB 是 **3024 个不可达 blob**
  （历史上误 `git add` 进来的构建产物，`.gitignore` 补上后就悬空了；可达对象只有 ~40 个 blob）。
  AI 副本已经 `git gc --prune=now` 清掉，降到 243 KB。
  原版目录**我没有动**——如果你想顺手清理：`git -C E:\Downloads\Zed\comtool-rs gc --prune=now`
  （这会永久丢弃那些悬空的构建产物，不可恢复；我认为无价值，但这是你的仓库，由你决定）。

---

## 2. 这是什么项目

Rust 串口调试上位机（GUI 串口助手）。技术栈：

| 项 | 值 |
|---|---|
| 语言/版本 | Rust **edition 2024**（需 rustc ≥ 1.85；实测工具链 `rustc 1.97.1`） |
| GUI | `egui` / `eframe` **0.36.1**（`glow` 后端 + `persistence` feature） |
| 串口 | `serialport` 4.10.0（manifest 写 `"4.7.2"` 是 caret 要求，实解析到 4.10.0） |
| 其它 | `rfd`（文件对话框）、`serde`（持久化）、`chrono`（日志文件名与接收时间戳前缀的格式化，P4 起在用）、`egui_plot` 0.37（实时图表，P5 起在用）。`tokio` 与 `egui-chinese-font` 已作为死依赖移除（见 P6 第 1 批） |
| 类型 | 单 binary crate（无 workspace、无 CI、无 lint 配置） |

### 模块结构（`ai-refactor` 分支）

```
src/
├── main.rs                 入口；字体回退链；eframe::run_native；APP_TITLE 常量；挂 `app_icon` 到窗口图标
├── app_icon.rs   (540)     256×256 圆角方波图标，代码绘制、零资源文件、11 条单元测试
│                           （外加 1 条默认跳过的 ASCII 预览测试）
├── app.rs       (2189)     ComTool 状态 + Default + 连接生命周期（串口 / TCP / UDP 三选一）
│                           + eframe::App 编排（含 F4 连接类型分叉与发送链路测试）
│                           + F5：控制线 RTS/DTR（apply_line_state）、发送计数（tx_bytes）、
│                             常用数据区（render_send_presets）、端口热插拔（rescan_ports/note_focus）
├── receive.rs   (1814)     接收链路：四份镜像状态 + AnsiDecoder + 字节可视化
│                           + 可选时间戳前缀（TimestampPrefix）+ 三种渲染
│                           + 显示模式互斥（resolve_display_mode）+ FIFO 淘汰（48 测试）
├── chart.rs      (485)     实时图表数据层：多通道数值流解析 + 跨包分片
│                           + 样本 FIFO 淘汰 + 通道上限（20 测试）
├── log_file.rs   (802)     实时文件日志：后台写盘线程 + 文件名模板渲染
│                           + 同名防截断 + 待写队列上限（14 测试）
├── sidebar.rs    (775)     右侧边栏；返回 SidebarAction 而非在绘制中改状态
│                           + 连接类型单选与「串口参数 / 网络参数」双分支（F4）
│                           + F5：波特率自由输入、控制线 RTS/DTR 一行
│                           + 2026-09-24 界面重构：**不再显示收发字节读数**（搬去状态栏）、
│                             **可用串口列表并入「串口信息」**（原独立折叠区已删）
├── send.rs      (1014)     发送侧纯逻辑：HEX 解析 / 行尾 / 编码 / 历史浏览
│                           + F5：转义解析（parse_escapes/EscItem）、常用数据（SendPresets）（37 测试）
├── status_bar.rs (674)     底部状态栏：彩色圆 + 状态词 + 收发字节数，整组居中；上下留白使其脱离边框（12 测试）
├── strings.rs    (3900)    界面文案表（**15 种语言同宏臂**，N 列转置）+ fill（含逐语言守卫）
├── config.rs     (647)     持久化策略 + 启动不变量修复（16 测试）
├── panels/
│   ├── mod.rs     (720)    面板模块出口（15 条无头 UI 测试，含英文界面、已连接侧边栏、设置窗口滚动、字体档位下拉接线、菜单栏无「发送」菜单 + 发送区仍保有四项设置、顶部工具栏无「⚙ 设置」按钮 + 菜单栏仍保有设置入口）
│   ├── menu_bar.rs(251)    顶部菜单栏：文件 / 设置 / 视图 / 文本；返回 MenuAction；主题/时间戳/编码/发送格式单选共用入口。**故意没有「发送」菜单**（与发送区表头重复，2026-09-24 删）。其「设置」菜单是**打开设置窗口的唯一入口**（中央区那颗重复的「⚙ 设置」按钮同日删）
│   ├── settings.rs(372)    设置窗口外壳（限高 + 纵向滚动区）+ settings_body 内容体（含语言、字体档位下拉）
│   ├── chart.rs   (224)    实时图表面板：egui_plot 绘制 + 绘制点抽样（5 测试）
│   └── info.rs    (52)     信息窗口
├── communicationtrait.rs (310)  ByteChannel trait（open/close/is_active/write/configure/
│                           set_line_state/local_addr）+ ChannelEvent（Bytes/Failed/Closed/Notice）
│                           + 三者共用的后台线程脚手架 `worker` + `#[cfg(test)] mod fake` 替身
├── serial_impl/
│   ├── channel.rs (364)    串口后台线程实现（6 测试）
│   ├── config.rs  (68)     SerialConfig + 波特率档位 + 任意波特率区间（CUSTOM_BAUD_MIN/MAX、clamp_baud）
│   └── mod.rs     (15)     出口
└── net_impl/               F4 新增：网络通道（与 serial_impl 平级，同一套 ByteChannel 契约）
    ├── config.rs  (145)    NetConfig（远端地址 / 端口 / UDP 本地端口）+ 自检（3 测试）
    ├── tcp.rs     (450)    TcpChannel：**同步 connect**、对端关闭走 ChannelEvent::Closed（8 测试）
    ├── udp.rs     (419)    UdpChannel：只绑定不连接、零长度数据报不是 EOF（7 测试）
    └── mod.rs      (41)    出口
```

测试函数合计 **254**（`cargo test` 报 253 passed、1 ignored；1 条 ignored 是图标 ASCII 预览，
需要目视时用 `cargo test app_icon::tests::ascii_preview -- --ignored --nocapture`）。

**权威约定文档**（改代码前先读）：
- `AGENTS.md` — 给 AI/协作者的模块地图与硬性约束（简短，必读）
- `DEVELOPMENT_GUIDE.md` — 中文开发参考：四层架构、数据流、新增功能的完整流程、注意事项 49 条

---

## 3. 当前状态：做完了什么

三项改造已完成，全部有验证证据（详见 `REFACTOR_REPORT.md`）：

1. **通信层独立重写**——为脱离参考实现 `SerialGUI-rs`（GPLv3）的衍生关系。
   抽象层改名 `CommunicationManager`→`ByteChannel`；实现改名 `SerialCommunication`→`SerialChannel`；
   停机机制从「共享标志位 + 每轮轮询」改为「命令通道投递 `Halt`」；状态从 `Arc<Mutex<EPortState>>`
   改为无锁 `AtomicU8` + `Drop` 守卫；发送从 `write_all` 改为 `VecDeque` 增量推进
   （**修掉一个真实数据丢失**：`timeout()` 同时作用于写操作，`write_all` 失败时不报告已写字节数，
   设备侧流控拉低时长报文会被静默截断）。
2. **`app.rs` 纯移动式拆分**——1172 行 → 434 行；拆出 `receive.rs`、`sidebar.rs`，
   `config.rs` 由空文件变成持久化层。等价性用归一化代码单元比对验证 **29/29 逐行等价**。
3. **serde 状态持久化**——只存用户偏好，运行时状态（串口句柄、连接标志、端口列表、接收日志）
   一律 `#[serde(skip)]` 并在启动时重建。

### 质量门禁（P0/P1 改动前 → 改动后）

| 命令 | 前 | P0/P1 后 | P4 后 | P5 后 | P6 第 1 批后 | P6 第 2 批 / P3 后 | 本轮（显示模式互斥修复）后 | F1~F3 后 | F4 后 | **F5 后** |
|---|---|---|---|---|---|---|---|---|---|---|
| `cargo check --all-targets` | 0 error / **12 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** |
| `cargo test` | 7 passed | **19 passed** | **50 passed** | **79 passed** | **90 passed** | **109 passed**（1 ignored，为图标 ASCII 预览） | **115 passed**（1 ignored） | **179 passed**（1 ignored） | **207 passed**（1 ignored） | **230 passed**（1 ignored） |
| `cargo fmt --check` | 2 处差异 | **干净** | **干净** | **干净** | **干净** | **干净** | **干净** | **干净** | **干净** | **干净** |
| `cargo clippy --all-targets` | 未测量 | 9 warning | **8 warning** | **8 warning**（与上轮逐条相同，全在我未改动的既有代码里：`derivable_impls`×2、`manual_range_contains`×3、`explicit_deref`、`unnecessary_map_or`、`redundant_borrow`） | **8 warning**（逐条同上） | **8 warning**（逐条同上） | **8 warning**（逐条同上；本次改动 0 新增） | **8 warning**（逐条同上；F1~F3 共 0 新增） | **8 warning**（逐条同上；F4 期间新增过 1 条 `field_reassign_with_default`，改掉后回到 8 条） | **8 warning**（逐条同上；F5 期间同样新增过 1 条 `field_reassign_with_default`，改掉后回到 8 条） |

跑法：

```bash
cd E:/Downloads/Zed/AI_comtool-rs
cargo fmt && cargo check --all-targets && cargo test
```

230 条测试**都不需要串口硬件**，包括一条走完异步打开失败路径的集成测试、
一条把字节经事件通道注入后核对日志文件内容的集成测试、八条用
`egui::__run_test_ui` 跑的无头 UI 冒烟测试（菜单栏、设置/信息窗口、三种接收视图、
侧边栏、**已连接状态的侧边栏**、图表面板、**常用数据一行**、英文界面），
11 条对应用图标做纯断言的单元测试（尺寸、透明度、渐变、
前景笔画、越界检查、确定性），17 条 P3 时间戳前缀测试（格式、三种模式下
的插入位置、跨批次不重复、程序提示不加前缀、清空后重新标注、不进图表），
F1~F3 带来的 64 条（`send.rs` 发送侧、`app.rs` 用 `FakeChannel` 驱动的发送链路、
`receive.rs` 的编码矩阵、`strings.rs` 的文案表不变量与源码守卫），
F4 带来的 28 条（`net_impl` 的 18 条：配置自检、TCP 的"连上 → 收发 → 对端关闭 →
状态复位"集成测试、UDP 的绑定与零长度数据报语义；`app.rs` 的 10 条：连接类型分叉、
通道复用与重建、提示行名词、UDP 不许说"已连接"、TCP 不自动重连），
以及 **F5 带来的 23 条**（`send.rs` 的 8 条：转义解析的片段切分 / 六种转义 / 三种非法形式 /
`\xNN` 绕过编码 / 字面文本仍走编码 / 行尾仍在字节层、常用数据标签与增删上限；
`app.rs` 的 13 条：打开端口不碰控制线、单根控制线请求原样到达通道、设置失败走
非致命提示、断开后勾选态归位、`Notice` 事件不像掉线、发送计数三种口径、断开归零、
常用数据复用同一通路且不动输入框、未连接/越界拒绝、空与重复与超限拒绝、波特率夹取、
焦点重扫不动所选端口、手动刷新才修正失效选择；`panels::mod` 的 2 条：已连接侧边栏与
常用数据四种状态的无头渲染）。
显示模式的互斥规则另有 6 条测试（两个点击方向、启动修复方向、合法组合不被改动、
冲突存档往返后被修好）。

### 3.1 增量（2026-09-21：P2 菜单栏 + 设置面板，P3 部分提前）

| 项 | 值 |
|---|---|
| 新增模块 | `src/panels/{mod,menu_bar,settings,info}.rs` |
| 新增持久化字段 | `theme` `auto_scroll` `max_log_chars` `repaint_interval_ms` `clear_send_after_send` `default_log_file_name` `check_update_on_start` `show_receive_panel` `show_send_panel`（其中 `check_update_on_start` 已在 P6 第 2 批随"不做版本检查"的决议删除，见 §3.5） |
| 新增运行时字段 | `show_info`（与 `show_settings` 一样 `#[serde(skip)]`） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **34 passed** · clippy 仍是 9 条既有警告（新代码 0 新增） |

- **上游 F 组（顶栏与菜单）4 项全部可用**：File(Info/Quit) · Settings 菜单 · 主题切换
  （浅色/深色/跟随系统）· 面板显隐勾选（至少保留一个可见）。
- **上游 G 组（设置项）6 项中 4 项已真正接线**：日志长度上限 · 发送后清空 · 自动滚动 ·
  重绘间隔（16~1000 ms）。剩下 2 项当时**只有设置项本身**：`default_log_file_name`（P4 已
  被实时文件日志消费）与 `check_update_on_start`（P6 第 2 批决定不做版本检查，该项已连同
  设置窗口里的复选框一起删除，见 §3.5）。当时两者都在界面上带了明确标注，不会让人误以为
  已经生效——这个做法值得保留：**宁可写清"未接线"，也不要让开关假装在工作**。
- **P3 提前完成的部分**：`max_log_chars` FIFO 淘汰（四份镜像各自按同一字符预算裁剪、
  能从句中切开 `ColoredSpan` 并保留其样式、按字符边界切分不破坏 UTF-8）+ `auto_scroll`
  开关。P3 剩下的"可选时间戳前缀"已于 2026-09-21 完成，见 §3.5。
- **P2 的一处偏离**：设置做成了窗口（`egui::Window`）而不是停靠面板，上游是设置面板。
  功能等价，未按上游搬。
- **仍未实测**：磁盘持久化（需实机运行，见 §4②）；P2 的界面外观也只通过了无头渲染
  冒烟测试（证明"画得出来不 panic"），**没有肉眼确认过**。

### 3.2 增量（2026-09-21：P4 实时文件日志）

| 项 | 值 |
|---|---|
| 新增模块 | `src/log_file.rs`（801 行，含 14 条测试） |
| 新增持久化字段 | `log_dir`（日志输出目录，空 = 用平台默认） |
| 新增运行时字段 | `log: Option<LogSink>`（`#[serde(skip)]`；`Some` 即「记录中」） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **50 passed** · clippy **8 条**（全在本轮未改动的既有代码里，**新代码 0 新增**） |

**做了什么**

- **`LogSink`**：写盘方式与串口通道同一套约定——文件句柄由后台线程独占，UI 只通过
  一条 `sync_channel` 投递待写文本，停机靠投递 `Halt`，`Drop` 投 `Halt` 后 `join`
  确保缓冲区落盘。用 `BufWriter`（64 KiB），且**队列一排空就 flush**，所以低速场景
  下文件仍然能被 `tail` 实时读到，而高吞吐时自动合并成大批次写。
- **文件由 UI 线程打开再交给后台线程**：`File::create` 是一次本地系统调用，这样
  「开始记录」能当场回答成功或失败。刻意**没有**沿用 `open_port` 那种「先乐观置位、
  失败再回滚」的写法——那正是 §5 里标为「高」的 `connected` 假阳性缺陷。
- **同名绝不截断**：模板精度只到秒，同一秒内再次开始记录会命中已存在的文件，
  而 `File::create` 会静默清空它。改用 `create_new` + 序号后缀（`name_1`、`name_2`…）。
- **模板安全**：`%Q` 这类非法占位符会退回默认模板（`DateTime::format` 遇到非法
  占位符会 **panic**，而模板来自用户输入和可被手改的存档）；`/ \ : * ? " < > |` 与
  控制字符会被替换为 `_`，所以 `../` 这类模板写不出选定目录。
- **待写队列有上限**（256 条 ≈ 几 MiB）：磁盘跟不上时停下并如实报告，而不是让队列
  无上限增长（串口工具挂机跑一整天是常见用法）。顺带修掉一个真实死锁：
  `Drop` 里必须用 `try_send` 投 `Halt`——队列满时 `send` 会**阻塞**，一个卡住的磁盘
  就能把「停止记录」永久挂住 UI 线程。
- **记录范围**：接收区看到什么就记什么（数据、HEX/字节查看的转义形式、串口开关与
  错误提示都在内），不含发送内容。日志文件**不受** `max_log_chars` 约束；「清空日志」
  也只清内存镜像，不会去截断正在写入的文件。
- **入口三处**：侧边栏「文件日志」折叠区（开始/停止 + 选择目录 + 状态）· 菜单
  「文件 → 开始/停止记录日志」· 设置窗口的文件名模板与输出目录。

**未做 / 未验证**

- 记录中途改文件名模板或输出目录**不会切换当前文件**（下次开始记录才生效），
  界面上没有特别提示。
- 磁盘故障、可移动盘被拔这类路径**没有自动化测试**（要制造「卡住的磁盘」不现实）；
  只覆盖了「队列满必须回报」这一分支本身。
- 日志写入失败后的 UI 表现（写盘线程报 `Failed` → 自动停止 + 提示）**只看过代码
  与类型推导，没有实机触发过**。
- 界面外观仍未肉眼确认（同上）。

### 3.3 增量（2026-09-21：P5 实时图表）

| 项 | 值 |
|---|---|
| 新增模块 | `src/chart.rs`（485 行 / 20 测试）、`src/panels/chart.rs`（223 行 / 5 测试） |
| 新增依赖 | `egui_plot = "0.37.0"`（MIT OR Apache-2.0；依赖 `egui ^0.36`，与本项目 0.36.1 匹配——**注意它的版本号与 egui 并不同步**：0.36 依赖的是 `egui ^0.35`） |
| 新增持久化字段 | `show_chart_panel`（默认 **false**）、`chart_max_samples`（默认 2000）、`chart_auto_bounds`（默认 true） |
| 新增运行时字段 | `chart: ChartData`（`#[serde(skip)]`） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **79 passed** · clippy **8 条**（与上轮逐条相同，**新代码 0 新增**） |

**做了什么**

- **数据层与绘制层分开**：`chart.rs` 只做「原始字节 → 多通道数值序列」，不依赖 `egui`，
  因此可以纯单元测试固定行为；`panels/chart.rs` 只负责画。
- **图表吃原始字节，与显示模式解耦**：喂数据的位置在 `receive_data` 里 HEX / 字节查看 /
  文本三个分支**之前**，所以切到 HEX 看字节时曲线照常推进。图表不是"第五份镜像"，
  它有自己的解析规则、上限与清空入口（`clear_chart`）。
- **输入格式**：一行一组样本、字段用逗号分隔、`\n` 或 `\r\n` 结束一组。第 *k* 个字段
  对应第 *k* 条曲线，同组共享一个 x（**组序号**，不是时间戳）。≥4 通道是验收要求，
  上限 `MAX_CHANNELS` = 16。
- **数据诚实性（三条"宁可断线也不编造"）**：字段少的行，缺的通道该组**不记样本**
  （曲线断开），**不补 0**；无换行的连续逗号流**不**按固定通道数循环分组（通道数只能猜，
  猜错就画错位，明确不支持）；整行没有数值时不产出样本、**也不推进 x**（否则混进来的
  纯文本日志会在曲线上打出等距空洞）。
- **跨包分片**：半行残留留在 `pending` 里等下一块补齐（与 `AnsiDecoder` 跨包拼接 UTF-8
  同一理由）。半行超过 4096 字节即判定格式不符，**整行丢弃**——剩余字节一直丢到换行符
  为止，否则被腰斩的尾部会变成一个假样本。丢弃计入 `discarded_segments` 并在界面上显示。
- **非数值字段计数**（`ignored_fields`）而不是静默跳过，界面显示"忽略 N 个非数值字段"，
  让用户能看出"格式配错了"而不是对着一片空白猜。
- **绘制点封顶**：`points_for_drawing` 把每条曲线抽到 4000 点以内。样本上限是用户可以调到
  10 万的设置项，而 `egui_plot` 每帧都要把点转成它自己的结构——不封顶的话一个设置项就能
  把界面拖死。抽样**保留最后一个点**（最新数据），否则曲线末端被切掉会看起来像"设备停了"。
- **图例常开**：多通道时"哪条线是哪个通道"只能靠颜色对照。配色 `CHANNEL_COLORS`（8 色循环）
  是**数据编码**，刻意不随主题变化。
- **入口**：视图菜单「面板 → 实时图表」· 设置窗口「实时图表」区（显隐 / 样本上限 /
  自动缩放 / 清空）· 图表面板自身的「清空图表」「自动缩放」。
- 图表面板**默认不显示**：它会在中央区占掉一块高度，而"有没有曲线"取决于设备是否发数值流，
  让新用户一上来就对着空图表没有意义。

**未做 / 未验证**

- **界面外观与交互（缩放、拖动、图例排布）全部没有肉眼确认过**，只有无头渲染冒烟测试。
- 图表**没有"按通道显示/隐藏"这类控件**，通道多了只能靠图例对照。
- 样本上限之外还有一层 4000 点的绘制抽样，两者是不同的上限（前者管内存，后者管每帧成本），
  界面上只暴露了前者——抽样这件事目前只在文档与代码注释里。
- x 轴是组序号而非时间，因此**无法看出真实的时间跨度或采样率**。

### 3.4 增量（2026-09-21：P6 收尾第 1 批 — 依赖清理 / release profile / 应用图标）

| 项 | 值 |
|---|---|
| 变更模块 | `Cargo.toml`、`src/main.rs`、新增 `src/app_icon.rs` |
| 新增依赖 | 无 |
| 移除依赖 | `tokio` 1.53.1（features = `["full"]`，零调用）、`egui-chinese-font` 0.2.0（零调用，中文字体已由 `main.rs::setup_fonts` 读系统字体实现） |
| 新增 Cargo profile | `[profile.release] opt-level = 2`（与上游一致，下调 Rust 默认的 3，缩短 release 构建时间） |
| 新增图标 | 256×256 圆角方形 + 数字脉冲波形，代码绘制，4×4 超采样抗锯齿，11 条单元测试（外加 1 条默认跳过的 ASCII 预览测试） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **90 passed**（1 ignored） · clippy **8 条**（与上轮逐条相同，新代码 0 新增） |

**做了什么**

- **死依赖清理**：`tokio` 启用 `full` 会明显拖慢编译且全项目零调用；串口、日志、未来版本检查都只需要一次性的超时/后台线程，用 std 线程足够。
  `egui-chinese-font` 也未被调用——`main.rs` 里已手写系统字体回退链（读 `C:\Windows\Fonts` 下的 `msyh.ttc` / `simsun.ttc` / `simhei.ttf` 等），不需要再包一层 crate。
- **release 优化级别固定为 2**：不写时 Rust 默认是 3；降到 2 与上游 `SerialGUI-rs` 保持一致，对 GUI 程序来说瓶颈在布局与 IO，少一档优化几乎不影响体感，却能缩短 release 构建时间、减小体积。
- **应用图标零资源文件**：`src/app_icon.rs` 用代码生成 `egui::IconData`（RGBA、256×256、圆角方形 + 两个脉冲的数字波形）。没有 `assets/icon.png`，因此不存在"资源文件缺失/与二进制不同步"的路径；同时所有几何与颜色属性都能被 `cargo test` 断言。
- **图标接进窗口**：`main.rs` 通过 `egui::ViewportBuilder::default().with_icon(Arc::new(app_icon::generate()))` 设置，eframe 在 Windows 上会把 `IconData` 重新编码成 PNG 再交给 `CreateIconFromResourceEx`，所以任务栏与窗口标题栏都能看到图标。此路径不新增依赖（`image` 是 eframe 本来就带的目标平台依赖）。

**未做 / 未验证**

- **图标外观只有代码级预览确认**：256×256 PNG 用 Python 纯标准库 zlib 临时转出看过，圆角、渐变、波形形状正确，但真实任务栏 16~32 px 的小尺寸效果需要实机运行才能最终确认。

### 3.5 增量（2026-09-21：P3 可选时间戳前缀 + P6 第 2 批决定不做版本检查）

| 项 | 值 |
|---|---|
| 变更模块 | `src/receive.rs`（核心）、`src/app.rs`、`src/sidebar.rs`、`src/panels/{settings,menu_bar}.rs`、`src/config.rs`（含测试） |
| 新增持久化字段 | `timestamp_prefix`（`TimestampPrefix::{Off, PerLine, PerBatch}`，默认 `Off`） |
| 删除持久化字段 | `check_update_on_start`（版本检查整体取消，见下） |
| 新增依赖 | 无（复用已在用的 `chrono`） |
| 新增测试 | 16 条（`receive.rs`）+ 1 条（`config.rs`，锁定"已删除字段的旧存档仍能读出来"） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **109 passed**（1 ignored） · clippy **8 条**（逐条同上轮，新代码 0 新增） |

**做了什么（P3 时间戳前缀）**

- 三档设置 `TimestampPrefix`：`Off`（默认，**行为与从前逐字节相同**）/ `PerLine` /
  `PerBatch`。用三值枚举而不是两个 `bool`，因为"不加时间戳、但按行"是非法组合。
  入口有三处：设置窗口「接收」区、视图菜单、以及侧边栏「接收统计」里的当前档位回显。
- 前缀形如 `[12:00:00.123]`（本地时间，毫秒）。格式串是 `receive.rs` 里的常量
  `TIMESTAMP_FORMAT`，`format_timestamp()` 单独抽出来是为了让测试能喂固定时刻——
  `%.3f` 是"小数点 + 三位毫秒"这件事由断言钉住，不靠记忆。
- **插在片段层**（`timestamped_spans()`）：`receive_text` 是由片段拼接而来的，两处各插
  一遍迟早走偏；只插一次，四份镜像自然一致。它因此也出现在导出文件与实时日志里
  （与"日志文件内容 = 接收区内容"这条既有承诺一致）。
- **它是注解，不是数据**：`chart.push_bytes()` 拿到的仍是原始字节，前缀绝不进去
  （`timestamps_do_not_leak_into_the_chart` 盯着这一点）。
- **行首状态不新增字段**：`at_receive_line_start()` 从 `receive_text` 尾部推导
  （空串或以 `\n` 结尾）。三种显示模式、`push_receive_note()`、`clear_receive()` 都在写
  这个镜像，任何平行状态字段都会在某个分支上与它不一致。
- 一批只取**一次** `Local::now()`：同一块数据里的多行本来就同时到达，逐行取时间会造出
  "每隔几毫秒来一行"的假象。**没有内容就不加前缀**，判据是"有非空片段"而不是"片段
  数组非空"（文本模式收到半个多字节序列时片段是空的）。
- 顺手做的一次收敛：`receive_data()` 里三条模式分支的"加时间戳 → 写镜像 → 投日志"
  被合并成一条公共尾部（原先三份重复的拼接逻辑），并**逐分支核对镜像落点**——HEX 优先
  于字节查看，落点是 `!hex_view && ascii_view` 而不是 `ascii_view`；后者会让"两个勾选框
  同时打开"时片段落进另一份镜像，这正是这次改写最容易悄悄引入的回归，已用
  `hex_takes_precedence_over_ascii_like_before` 锁住。

**做了什么（P6 第 2 批：决定不做版本检查）**

- **项目所有者决定跳过版本检查**（2026-09-21）。理由与实现成本无关：内网环境访问不到
  检查地址，一个永远失败的开关比没有这个功能更糟。
- 因此把 `check_update_on_start` 设置项、它在设置窗口「更新」区的那一整个段落、
  以及"将在 P6 接线"的说明一并删除——**界面上不留无效开关**。删除是安全的数据变更：
  反序列化会忽略存档里已经不认识的那个键，`archive_with_a_removed_field_still_loads`
  把这条固定下来（顺带也防止以后有人给 `ComTool` 加 `deny_unknown_fields`，那会让所有
  旧存档读不出来）。
- 若将来要做，重加一个字段 + 一个复选框即可，代码里没有留下半成品。

**未做 / 未验证**

- 时间戳的**界面外观（灰色、与数据的对比度）没有肉眼确认过**；只验证了文本内容正确。
- 三种模式都加前缀，包括 **HEX 与字节查看视图**。在那些视图里前缀是"注解"而不是收到的
  字节，靠方括号与灰色区分；若要一份纯字节记录，请关掉前缀（这也是默认值）。
- clippy 那 8 条既有风格警告**仍未处理**：它们都在本任务未触及的既有代码里（`AnsiDecoder`
  的手写 `Default`、若干 `manual_range_contains` 等），`cargo clippy --fix` 可一键清掉，
  但那会改动本任务范围之外的代码，因此留给下次专门做一次"清警告"提交。

### 3.6 增量（2026-09-21：修复「HEX 接收 + 字节查看」空视图缺陷）

**问题**：接收设置行里「HEX」与「文本」是互斥的，但「字节查看」是自由勾选的。两者同时勾上
时走 HEX 分支（HEX 优先），片段只写 `display_spans`；而「颜色」未勾时渲染的正是
`ascii_spans`——**接收区一片空白**。不是"两种格式混排"，是"什么都没有"。

**做了什么**

- 互斥规则收敛到唯一实现 `ComTool::resolve_display_mode()`（`receive.rs`）：置上用户刚勾中的
  那一项、关掉另一项。参数是"本轮哪个刚被勾上"，而**不是**无条件让 HEX 胜出——否则用户在
  HEX 模式下点「字节查看」会被当场撤销，表现为"点了没反应"，比空视图更难排查。
- 两个界面入口都改走它：接收设置行（`app.rs`）与视图菜单（`panels/menu_bar.rs`，新增
  `display_mode_checkboxes()`）。**规则只有一处定义**，不会出现"菜单里能同时勾上、主界面里
  不能"这种两边不一致。
- **启动修复**：旧存档里两个都可能为 `true`（那个组合曾经能从界面勾出来）。
  `config::restore_runtime_state` 现在调用 `resolve_display_mode(false, false)` 修好它，方向与
  `receive_data()` 的分支优先级一致（HEX 胜出）：与旧版本实际走的解析分支一致，而且修好之后
  给用户的是一份**非空**视图。
- `receive_data()` 里的 `!hex_view && ascii_view` 落点判据**保留**，作为纵深防御：手改存档、
  或将来有调用方绕过 UI 直接置位时，落点仍与分支一致。
  `hex_takes_precedence_over_ascii_like_before` 继续守着它（注释已改写为说明这层防御性质）。

**验证**：新增 6 条测试（`receive::tests` 4 条：两个点击方向各一条、启动修复方向、合法组合
不被改动；`config::tests` 2 条：冲突存档往返后被修好、只开字节查看不受影响）。另外把
`config::tests::customized()` 里 `hex_view` / `ascii_view` **同时为真**的样本改成合法组合——
它此前正是那个非法状态，修复逻辑一上线就会被它撞出来（第一次 `cargo test` 正是这么红的）。

**未做 / 未验证**

- **点击行为没有肉眼确认过**：互斥逻辑由单元测试覆盖，但"点一下是不是真的关掉了另一个"
  只能在实机界面上看。§4② 的手工验证清单已加入这一项。
- 「颜色」与「字节查看」同时勾选时**渲染的是彩色视图**（`color_view` 在渲染顺序里优先），
  此时「字节查看」勾着却不显示对应视图。——👉 **已于 §3.16 修复**，且**当时这段的描述也是错的**：
  不是"被另一视图盖住、不丢内容"，而是彩色视图渲染的是空的 `display_spans`，接收区**一片空白**
  （写进去的那份是 `ascii_spans`）。教训：判断"哪个视图盖住哪个"时必须同时核对**镜像落点**，
  只看渲染分支会得出相反结论。

---

### 3.7 增量（2026-09-21：新联线 F1 + F2 + F3）

计划本体见 `FUTURE_ROADMAP.md`。三项按顺序落地，各自单独一次提交：

| 提交 | 内容 |
|---|---|
| `2e28994` | **F1** 发送侧补齐：HEX 发送 / 行尾 / 定时发送 / 发送历史 |
| `e2039ff` | **F2** 收发字符编码选择：GBK / GB18030 / UTF-16LE |
| （本条提交） | **F3** 多语言真接线 |

**F1 新增 `src/send.rs`**（纯逻辑，不依赖 egui / 串口，因此可以直接单测）：

- `parse_hex()`：允许空格 / 逗号 / 分号 / 冒号 / 连字符分隔与 `0x` 前缀。
  **奇数位、非十六进制字符都返回带原因的 `Err`**，绝不"跳过看不懂的字符"。
- `encode_text()` / `build_payload()`：四入参顺序固定为「文本 → 格式 → 行尾 → 编码」。
- `SendHistory`（上限 50，跳过空串与连续重复，进存档）+ `HistoryNav`（↑/↓ 浏览位置）。
- 定时发送用 **`Instant` 差值**计时（不是每帧累加，否则帧率波动会漂移）；勾上的那一刻
  **只起步计时、不立刻发一包**；断开连接一律 `stop_repeat_send()`，且不跨进程延续。
- 「发送后清空」的语义是**"确实发出去了"才清空**：HEX 解析失败与写失败都保留输入。
  这是 F1 期间由测试撞出来的一个**真实行为修正**（原实现写失败也清）。

**F1 的测试口子**：`communicationtrait.rs` 里新增 `#[cfg(test)] mod fake`
（`FakeChannel` + `FakeHandle`）。它让"发送链路 + 连接生命周期"能在 `cargo test`
里完整跑通——**不需要串口硬件**，也不需要启动 GUI。后续给 `ByteChannel` 加实现
（F4 的 TCP/UDP）时，这个替身是验证 `receive_data()` 分支的现成手段。

**F2 新增依赖 `encoding_rs`**（本阶段唯一新增依赖）：GBK / GB18030 走它的
**增量解码器**，被截断的多字节序列留在解码器内部等下一批，跨包拼接不会退化成逐批
lossy。三个坑：

1. **GBK 与 GB18030 共用解码器、编码器不同**——界面上必须说明这一点，否则用户看到
   两档"好像一样"会犯嘀咕。
2. **UTF-16LE 在 `encoding_rs` 里只支持解码**：它的 `encode()` 会把 UTF-8 字节原样退回。
   所以小端字节由 `encode_utf16()` + `to_le_bytes()` 手写，且**不加 BOM**。
3. 切换编码时 `AnsiDecoder::sync_encoding()` 丢弃残留半截序列，
   `config::restore_runtime_state()` 也会重置，避免把上一编码的字节按新编码解释。

**F3 新增 `src/strings.rs`**（界面文案表）。关键设计：

- 用 `macro_rules! strings!` 把**中英两列写在同一条宏臂里**（`字段名: "中文", "English";`）。
  这比路标原写的"`struct` + 两个 `static`"更强：漏翻译不是"编译错误"，而是**写不出来**。
- `Language::strings() -> &'static Strings`：返回 `'static`，取出来就不借用 `self`，
  于是 `&mut self` 的方法体里可以随手取用。
- **带占位符的文案不能 `format!(s.xxx, arg)`**——`format!` 的格式串必须是字面量
  （变量会报 `format argument must be a string literal`）。改用 `strings::fill(模板, &[&实参])`。
- **接收区 / 日志里的程序提示行不翻译**（"串口已关闭"、"已导出到: …"）。它们会同步写进
  实时日志与导出文件；随界面语言变化会让同一台设备产出的两份日志不可比。
- 删掉了 `TimestampPrefix::label()`（原来硬编码"关闭 / 每行 / 每批"）。领域类型上挂着
  界面词汇，迟早有人顺手用上，那几个词就再也不会被翻译。现在必须显式传文案表。
- **源码级守卫** `strings::tests::ui_sources_do_not_hardcode_chinese`：扫描 5 个纯 UI
  文件（sidebar / menu_bar / settings / info / chart），出现硬编码中文字面量就失败。
  唯一豁免是语言下拉里的「中文」（档位**自名**，切到英文时也要写"中文"）。
  这条测试写完立刻抓到两处真问题，不是摆设（详见该测试与 `contains_token` 的注释）。
  `app.rs` / `receive.rs` / `log_file.rs` **故意不在清单里**——它们按设计保留中文提示行。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **179 passed / 1 ignored** · clippy **8 条**（与之前逐条相同，本阶段 0 新增）。

**未做 / 未验证**：三项都**没有实机运行过**（不启动 GUI、不碰串口是硬规则）。F3 的
"切到 English 后界面全是英文"只由文案表不变量 + 无头渲染冒烟测试保证，
**外观与英文文案的通顺度仍需人工过一遍**——§4② 的手工验证清单已加入这一项。

### 3.8 增量（2026-09-21：F4 — TCP / UDP 客户端）

计划本体见 `FUTURE_ROADMAP.md` §5。这是 F 线最后一项，也是唯一一项**能在门禁内被自动
验完**的网络功能（用 `std::net::TcpListener` 起本地回环，不需要任何硬件）。

**新增 `src/net_impl/`**（与 `serial_impl` 平级，零新增依赖，只用 `std::net`）：

| 文件 | 内容 |
|---|---|
| `config.rs` | `NetConfig`（远端地址 / 端口 / UDP 本地端口）+ `validate()` 与 `remote_addr()`（同步 DNS） |
| `tcp.rs` | `TcpChannel`：**同步 `connect_timeout`**、`set_nodelay(true)`、`read` 返回 `Ok(0)` 发 `ChannelEvent::Closed` |
| `udp.rs` | `UdpChannel`：绑 `0.0.0.0:local_port` 后 `connect` 固化对端；`Ok(0)` 是零长度数据报**不是** EOF |

四个设计取舍（都不是随手写的）：

1. **TCP 用同步 connect，不照抄串口的"乐观打开"**。串口的 `open()` 只拉起后台线程，
   真正的失败要等 `Failed` 事件——那是既有的 `connected` 假阳性缺陷。网络侧反过来：
   `open()` 返回 `Ok` 就**确实连上了**，界面不会先亮"已连接"再变回去。
   代价是 DNS + connect 在 UI 线程上做（最多 3 秒超时），这个取舍写在 `tcp.rs` 模块文档里。
2. **`Connected` UDP socket 在 Windows 上会把 ICMP port unreachable 报成
   `ConnectionReset`**，它必须当作可忽略错误——否则对端没监听时发一次就判死刑。
   `worker::is_transient` 只放三种**所有传输共有**的可忽略错误，UDP 自己补这个特例，
   不往公共函数里塞（否则串口也会跟着忽略一个真故障）。
3. **三者共有的后台线程脚手架抽到 `communicationtrait::worker`**
   （`WorkerCommand` / `StateGuard` / `IDLE` / `ACTIVE` / `is_transient`）。
   "线程退出后状态必须归位""`Halt` 在任何时刻都要能被看到"是三条实现共有的硬约定，
   复制三份迟早有一份走偏。
4. **第一版不给 TCP 自动重连**：`auto_reconnect` 依赖 `last_opened_port`（串口语义），
   对端没启动时反复重试只会攒出一批半死不活的 socket，而界面上完全看不出来。

**`app.rs` 的连接生命周期改造**（`open_port` / `close_port` → `open_connection` /
`close_connection`）：

- 字段 `serial: Option<Box<dyn ByteChannel>>` **改名 `channel`**——它从 F4 起真的可能装
  TCP / UDP，叫 `serial` 会让读代码的人以为这里只可能是串口。
- 新增 `ConnectionKind`（`Serial` / `TcpClient` / `Udp`，`#[default]` 落在 `Serial`，
  旧存档反序列化后行为与以前一致），以及 `net_remote_host` / `net_remote_port` /
  `net_local_port` 三个**落盘偏好**。
- 新增运行时字段 `channel_kind`，引出**"同类型复用、换类型重建"**这条通道复用规则。
  这不是优化：断线重连是"同一个句柄再 `open` 一次"，而测试注入的替身通道一旦被拆掉，
  `app.rs` 里那 20 多条发送链路测试就全部变成对着一个真串口跑。`teardown_channel()`
  把 `channel` 与 `channel_kind` 一起清空，不留半截状态。
- 拆成 `connection_ready()`（唯一的前置条件判定）/ `new_channel()` / `channel_descriptor()`
  （配置 + 提示措辞）三小块，而 **`configure` → `open` → 接管事件通道这三步仍然只有一处实现**。
- 提示行的名词由 `transport_noun()` 给出（`串口` / `TCP` / `UDP`）。提示文本本身**不翻译**
  （理由同 §3.7），但名词必须跟着传输类型走——否则 TCP 掉线会写一句"串口连接已关闭"。

**`sidebar.rs` 的四处界面分叉**：连接类型单选（**连接中锁定**）、参数区
（串口六项 vs 地址 / 端口，另一组整段不画）、只读信息区（端口 + 波特率 vs 目标 +
本地端口）、状态词与按钮文案。锁定单选的第一次出现：切了之后界面显示的参数就与真正
打开的那条通道对不上，而按钮此时是「断开」——点下去断的是**另一种**传输。

**顺带修掉的既有缺陷**：侧边栏连接按钮的文案与动作一直是**反的**（已连接时显示
「打开串口」、点下去却执行关闭）。它从上游继承而来，F3 之前就在
（`e2039ff:src/sidebar.rs:157`），F4 重写这段时一并修正。

**新增 `ByteChannel::local_addr()`**（带默认实现的 trait 方法，只有 UDP 覆盖）：
UDP 的 `local_port` 允许配 0（系统分配），用户**必须**能看见最终分到哪个端口，
否则不知道该让设备往哪回包。侧边栏把它显示在信息区的「本地端口」一行。

**F4 的一个平台坑（已写进测试注释）**：Windows 的 UDP 允许"特定地址"与"通配地址"
占用**同一端口号**共存——先绑 `127.0.0.1:P`、再绑 `0.0.0.0:P` 会成功。
所以"端口被占用要当场报错"那条测试里的占位 socket 必须也绑 `0.0.0.0`，
否则永远测不到冲突。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **207 passed / 1 ignored** · clippy **8 条**（与之前逐条相同；期间新增过
1 条 `field_reassign_with_default`，改掉后回到既有 8 条）。

**未做 / 未验证**：F4 同样**没有实机运行过**。`net_impl` 的 18 条测试里有两条会真的
走本机回环（`std::net::TcpListener` / `UdpSocket` 对发），`app.rs` 里那条"换类型要重建"
的测试会去连 `127.0.0.1:1`（必然被拒）——这些都不碰串口、不启动 GUI。
但"用 `nc -l 1234` 手工收一遍"这条验收标准**只能由所有者亲手做**：§4② 的手工验证
清单已加入 TCP / UDP 两项。TCP Server（多客户端）、命名管道、TLS/SSH 均不在本项内。

### 3.9 增量（2026-09-21：F5 — N1~N6 六项小额改进）

**零新依赖**，六项都是"顺手的活"（详见 `FUTURE_ROADMAP.md` §13.6 的逐项设计与 6 处偏离）：

| 项 | 一句话 | 关键约定 |
|---|---|---|
| **N1** RTS / DTR 手动控制 | 侧边栏「串口参数」下多一行两个复选框 | 打开端口**一条命令都不发**（serialport-rs 在 Windows 上打开时本就把两根线置为 Disable）；勾选态是"**我们请求的电平**"，不是对引脚的测量（驱动只写不读）；失败走**非致命**的新事件 `ChannelEvent::Notice`，不判掉线 |
| **N2** 常用数据 | 发送按钮下方一行：存为常用 / 一键重发 / 右键删除 | 与输入框走**同一条**通路（`payload_for` → `transmit`）；只存内容不存标签（标签由 `preset_label()` 现推）；**满了不淘汰**（与发送历史的 FIFO 相反）；设置窗口里有计数与一键清空 |
| **N3** 转义字符 | 发送格式的第三档 | `parse_escapes()` 返回 `Vec<EscItem>{Literal, Bytes}`——`\xNN` **绕过编码**，字面文本仍走所选编码；非法转义同样"不发、不清空输入框" |
| **N4** 发送字节计数 | **底部状态栏**右侧（2026-09-24 起；原在侧边栏「接收统计」） | 只在 `write` 返回 `Ok` **之后**累加；断开归零；悬停里点明它与「接收字节数」口径不同（那个是当前缓冲区长度，会被上限裁掉） |
| **N5** 任意波特率 | 波特率下拉旁跟着一个可拖可键入的数字框 | 两个控件改的是**同一个** `baud_rate`；`clamp_baud()` 作为第二道防线（存档手改得动）。**2026-09-24 补**：数字框改为**只在下拉选中「自定义…」档时出现**，档位由独立的 `baud_custom: bool` 记录（不从值反推），点击语义集中在 `sidebar::apply_baud_pick()`。详见 `DEVELOPMENT_GUIDE.md` 约定 45 |
| **N6** 端口热插拔 | 切回窗口时静默重扫端口列表 | 规则抽成 `ComTool::note_focus()`（`ui()` 造不出无头测试需要的 `Frame`）；用 `rescan_ports()` 而非 `refresh_ports()`——**连接中不许自动改 `selected_port`**；刻意不做定时轮询 |

**新增的运行时字段**（全部 `#[serde(skip)]`）：`rts` `dtr` `tx_bytes` `was_focused`。
**新增的持久化字段**：`send_presets`（旧存档缺这个键，靠容器级 `#[serde(default)]` 拿默认值）。

**新增的 trait 面**：`ByteChannel::set_line_state()`（带默认实现，返回 `Unsupported`，仅
`SerialChannel` 覆盖）+ `WorkerCommand::SetLineState` + `ChannelEvent::Notice`。
TCP / UDP 的命令循环各补了一个空分支（它们没有控制线）。

**一处顺带的重构**：常用数据那一行从 `eframe::App::ui` 里抽成了
`ComTool::render_send_presets()`（`pub(crate)`）。理由不是好看，是**可测**——
`ui()` 需要一个无头测试构造不出来的 `eframe::Frame`，写在里面等于一次也不会被
`cargo test` 画到。抽出来之后 `panels::tests::preset_row_renders_without_panic`
能把它四种状态都画一遍。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **230 passed / 1 ignored** · clippy **8 条**（与之前逐条相同；期间同样新增过
1 条 `field_reassign_with_default`，改掉后回到既有 8 条）。
（截至 2026-09-24：深色主题修复 +3 条测试、自定义波特率档 +5 条测试、设置窗口滚动 +1 条测试、
全局字体档位 +7 条测试（含 `panels::tests` 两条、`receive::tests` 两条、`app::tests` 三条），
加底部状态栏 +6 条测试，合计 **252 passed / 1 ignored**；
随后状态栏圆点从 emoji 改为自绘实心圆，净 +1 条（删掉 emoji 字形覆盖测试，新增
「画出正确颜色的圆」「尺寸随行高」两条），合计 **253 passed / 1 ignored**。）

**未做 / 未验证**：F5 同样**没有实机运行过**，六项全部只经 `cargo test` 验证。其中三项
**只能由所有者实机确认**（已加入 §4② 清单）：控制线能否真的复位板子、硬件流控档下手动
改 RTS 是否被驱动覆盖、非标波特率驱动是否接受；另外 N6 的焦点跳变在真实窗口上是否
一定产生一次 `false → true` 也未实测。界面外观同样只过了无头渲染的"画得出来不 panic"。

---

### 3.10 增量（2026-09-24：多语种扩展 —— 第 1、2 档 10 种 + 第 3 档 3 种 = 15 种）

前置分析见 `I18N_EXPANSION_RISK.md`（四轴风险排序：字体 / 复数 / RTL / 长度）与
`I18N_TARGET_LANGUAGES.md`（分档与第 0 步前置改造）。本次落地**第 1 档（6 种）+ 第 2 档（4 种）**
共 10 种新语言，语言总数 **2 → 12**；随后**第 3 档（tr / ru / pl）3 种落地，总数 12 → 15**。
第 1、2 档与第 3 档是**两次独立提交**，但共用同一个宏（`langs:` 每批尾部追加列）。

| 档 | 语言 | 变体 | 关键风险轴 |
|---|---|---|---|
| 1 | 繁体中文 / 日语 / 西语 / 意语 / 葡语 / 印尼语 | `ZhHant` `Ja` `Es` `It` `Pt` `Id` | 字体（假名）、长度（西语 +20~30%） |
| 2 | 法语 / 德语 / 韩语 / 越南语 | `Fr` `De` `Ko` `Vi` | 字体（谚文、越南语变音字符） |
| 3 | 土耳其语 / 俄语 / 波兰语 | `Tr` `Ru` `Pl` | **零字体工作**（雅黑全覆盖）、**零复数成本**（`fill()` 零复数逻辑）；新文字系统 = 需补西里尔检测器 |

**宏从"两列"改成"N 列"**。这是本次唯一的结构性改动，也是"第 0 步前置改造"的核心：

- `strings!` 的表头是 `langs: ZH, EN, ZH_HANT, JA, ES, IT, PT, ID, FR, DE, KO, VI, TR, RU, PL;`，
  每条文案后跟**同顺序的 15 个字面量**。行数列数不对由 `@no_leftovers` 报成
  `compile_error!`——加语言漏一列**编译期就红**，不需要运行期测试兜。
- **转置靠"边走边剥"**：`macro_rules!` 既**不能拼接 token**（写不出 `(m m)` → `(m m m)`），
  也**不能在匹配时做算术**（`[$n - 1]` 不成立）。所以"取第 k 列"只能每处理一种语言就从
  每行的字面量里**取走第一个**，剩下的尾巴交给下一个语言（`@emit_langs` 的两个臂）。这一
  设计在隔离环境里用 3 列 / 10 列探针验证过，并验证了"列数过多 → `compile_error!`"。
- 生成符号：`struct Strings`（每条文案一个字段）/ 15 个 `static`（`ZH` `EN` `ZH_HANT` … `PL`）/
  `#[cfg(test)] const LANGS`（列顺序镜像）/ `#[cfg(test)] const ALL`（`(字段名, 各列)`）。

**为什么选"原地追加列"而不是"整块重写"**：重写无法保证 zh / en 两列**逐字节不变**；
原地追加能。`chinese_column_keeps_the_original_wording` 与一个一次性校验脚本
（比对扩展前备份，**199/199 条 zh 与 en 逐字节一致**）共同证明了这一点。第 3 批（tr/ru/pl）
沿用同一手法，扩展后再次校验 **199/199 条前 12 列逐字节一致**（`tmp_verify.py`）。

**新增 9 条守卫测试**（`strings::tests`，全部做过"空转验证"——故意改坏被测机制确认变红）：

| 测试 | 钉住什么 | 空转验证方式 |
|---|---|---|
| `columns_have_no_foreign_script` | 每列不许出现其它语言的文字系统（日语列允许汉字，拉丁列出现 CJK 才是漏翻）；检测器集合含**西里尔** | 意语列塞中文 → 红 |
| `every_language_column_has_the_same_shape` | 每列条目数 = `Language::ALL` 长度 | — |
| `langs_table_matches_language_order` | `LANGS` / 表头 / `Language::ALL` 三处顺序一致（错位会整界面显示另一种语言且**能编译**） | — |
| `every_language_variant_maps_to_a_distinct_table` | 每个变体指向不同 `static`（切了语言界面不变 = 漏改 `strings()`） | — |
| `non_chinese_columns_are_actually_translated` | 非中文列与中文一字不差的比例 < 50% | — |
| `traditional_chinese_is_a_real_traditional_locale` | 繁体列里有 ≥15 个**简体列没有的**汉字 | 令 `hant_index = zh_index` → 红 |
| `template_placeholder_counts_are_pinned` | **全部 15 列**的 `{}` 个数与调用处实参一致（按字段名索引 `ALL`） | 删掉韩语列一个 `{}` → 红，报出"한국어" |
| `third_batch_columns_use_their_own_alphabets` | `ru` 列 ≥100 条含西里尔（实测 195）、`pl` 列 ≥40 条含**波兰独有**字母 `ą ć ę ł ń ś ż`（实测 101）、`tr` 与中文一字不差 <25%（实测 4） | 探针 A（抹掉 ru 全部西里尔）→ 红 0/199；探针 B（抹掉 pl 独有字母）→ 红 0/199；探针 C（整列 pl 抄成 ru）→ 红 |
| `ui_sources_do_not_hardcode_chinese` | 5 个纯 UI 文件里零硬编码中文（**白名单已清空**：自名统一走 `endonym()`） | settings.rs 插中文标签 → 红 |

外加 `every_vietnamese_character_has_a_glyph_in_the_font_chain`：遍历越南语列**实际出现的
全部字符**（非抽样），逐个在字体链里找字形；读不到字体文件时**跳过**（非 Windows 环境）。

**两个被实测推翻的假设**（写下来防止有人再犯）：

1. **"繁体与简体应高度重合"是错的**。实测只有 **22/199** 条一字不差——因为繁体本地化
   **本来就会改术语**（`设置`→`設定`、`字节`→`位元組`、`发送区`→`傳送區`）。那 22 条全是
   纯 ASCII / 数字 / 符号条目。原阈值 ≥50% 是**空转的假判据**，已改成"繁体列里有多少个
   简体列没有的汉字"（实测 31 个不同字形，阈值取 15）。
2. **写死一张"繁体专属字表"不可靠**——极易夹进简繁共用的字（`位` `元` `單` `開` 都不是
   繁体专属）。实测那张表在**故意把整列改回简体**时仍能通过。现已改成**从数据自证**：
   统计"只出现在繁体列、简体列一个都没有"的字符。
3. **`columns_have_no_foreign_script` 的检测器集合有真实覆盖漏洞**（第 3 批时发现）：
   原来只有 假名/谚文/泰文/中日韩 四个检测器，**没有西里尔**——把整列 `ru` 抄成波兰语
   （纯拉丁）时，四个检测器全部不命中，整列被换掉却静默通过。补 `has_cyrillic` 后探针变红。
   教训：**每加一种新文字系统，必须同时补一个"字形归属"检测器**。
4. **"守卫做了空转验证"本身也可能是假的——两个假绿陷阱**（第 3 批时踩到，最隐蔽）：
   本轮跑探针时，A/B/C 三个注入**全部报绿**，一度以为守卫无效。实为探针**压根没跑到测试**：
   - **陷阱 1**：本项目**没有 lib target**（单 binary crate），`cargo test --lib <名字>`
     直接报 `error: no library targets found in package`——一条测试都没跑。旧探针的红判据只找
     `FAILED` / `panicked at`，这个报错两样都不含，于是判绿。
   - **陷阱 2**：`cargo test -- --exact <裸名字>` 匹配不到模块限定路径（须写
     `strings::tests::xxx`），输出 `running 0 tests; 262 filtered out` 然后
     `test result: ok`——**0 条执行也报 ok**，又判绿。
   - 修法：探针改用 `cargo test --bin comtool-rs -- <名字>`（不带 `--exact`），并把
     **执行数 == 0 也判为红**。改后 A/B/C 三条全部正确变红并打印出各自的诊断文案。
   - 教训：**空转验证脚本自身必须先被验证"能看见红"**；只盯 `FAILED` 字样的判据会吞掉
     "编译失败 / 目标不存在 / 全被过滤"这三类"根本没跑"的情形。

**语言自名的落点变了**：早先「中文」白名单是为了让语言下拉在英文界面下仍写"中文"。现在
15 档各有自名（`中文（简体）` / `日本語` / `한국어` / `Tiếng Việt` / `Türkçe` / `Русский` /
`Polski` …），统一放在 `app.rs::Language::endonym()`（该文件**不在**被扫描的纯 UI 文件清单里），
下拉只从 `Language::ALL` 迭代取用。于是 `ui_sources_do_not_hardcode_chinese` 的白名单**清空**——
这比留一个"只豁免中文"的白名单更强：把自名硬写回 UI 文件会直接判红。

**字体链**：`main.rs::setup_fonts` 重写为统一闭包，往 **Proportional 与 Monospace 两个族**
各追加（曾踩过只加一个族、接收区越南语仍是方框）。链 = 微软雅黑（中文，排在 Hack 之后以
保 ASCII 等宽）→ `tahoma.ttf`（越南语 U+1EA0–U+1EF9 + 泰文，实测 0 缺失）→ `malgun.ttf`
（谚文）→ `seguisym.ttf`（符号）。**换任何字体都得不到彩色 emoji**（见约定 48）。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **261 passed / 1 ignored**（252 → 260 → **261**）· clippy **8 条既有**
（每次扩语言均 0 新增）。

**未做 / 未验证**：15 种新语言的**外观与文案通顺度从未有人看过**——只由文案表不变量 +
无头渲染冒烟测试保证。第 3 档 `tr ru pl` **已落地**（本轮）。仍剩：`th`（泰文叠置记号整形
需先做目视探针）、`ar`/`he`（**RTL 策略未决**，且 egui 0.36.1 有 GSUB/GPOS 整形但**无 bidi
重排**，RTL 与拉丁数字混排无保障）、`hi`（**字体挡住**：本机四候选字体全缺 52 个天城文字）。
**复数**（俄语 / 波兰语等需要）**完全未涉及**——`fill()` 是纯位置替换，零复数逻辑，与现有
英文列同等质量（现有英文列本身也没做复数，故本批三语复数成本为 **0**）；真要做复数需先改
`fill()` 的契约。

### 3.11 增量（2026-09-24：界面重构 —— 接收统计下移、可用串口并入串口信息）

**用户要求**：「把接收统计移动到底部状态栏，把可用串口合并到串口信息」。两处都是**布局搬家**，
没有新功能，但"搬家"这类改动最容易只做完加法、忘了减法（同一个数字在窗口里出现两次）。

**做了什么**

| 内容 | 原位置 | 新位置 |
|---|---|---|
| 接收字节数 / 发送字节数 | 侧边栏「接收统计」折叠区 | **底部状态栏右侧**（右对齐，含口径悬停说明） |
| 显示模式 / 时间戳前缀 | 侧边栏「接收统计」折叠区 | **侧边栏「串口信息」**（属"当前设置"，不是读数） |
| 可用串口列表 | 侧边栏独立折叠区「可用串口」 | **并入侧边栏「串口信息」**（接在端口/波特率之后） |
| 「接收统计」折叠头 | 侧边栏 | **整条删除**（连带 `strings.rs` 的 `rx_stats` 条目） |

**状态栏版式**：外层一个 `horizontal`，左侧圆点 + 状态词；右侧再套一层
`Layout::right_to_left` 把读写数推到最右。注意**该布局从右往左排**，所以条目要**反着给**
（先 `tx`、再 separator、再 `rx`）——给成正序会左右颠倒。

> ⚠️ **上面这段版式描述已被 3.12 取代**：用户目视后要求改为"整组居中"，
> 不再是左状态右读数。保留在此仅为记录当时的取舍与那个 `right_to_left` 的坑。

**新增文案**：`rx_bytes_hover`（接收字节数的口径悬停）。`tx_bytes_hover` 早就有了，两条
口径不同必须分别说明（N4 的延续）。为加这一条，`strings.rs` 15 列各补一列，同时删掉
`rx_stats` 一条，**条目总数不变（199）**。

**新增 5 条守卫测试**（全部做过非空转验证——7 个注入探针逐一确认真会变红）：

| 测试 | 钉住什么 | 空转验证 |
|---|---|---|
| `status_bar::tests::status_bar_paints_live_byte_counters` | 状态栏画出两个读数，且是 `receive_text.len()` / `tx_bytes` 的真实值 | 删掉 rx 行 → 红；把 rx/tx 调换 → 红 |
| `status_bar::tests::status_bar_byte_counters_change_with_the_data` | 读数随数据变（写死常量必失败） | 把 `rx_bytes` 写死 0 → 红 |
| `sidebar::tests::sidebar_no_longer_shows_the_byte_counters` | 读数**已离开**侧边栏（搬家的减法一半） | 把两行加回侧边栏 → 红 |
| `sidebar::tests::available_ports_render_inside_the_serial_info_section` | 端口名画出来 + 旧的「接收统计」标题不再出现 | 删掉端口列表 → 红；把「接收统计」标题加回 → 红 |
| `sidebar::tests::sidebar_keeps_display_mode_and_timestamp_lines` | 显示模式/时间戳前缀**留**在侧边栏（没被一起搬走） | 删掉显示模式行 → 红 |

**一个断言被自己驳回的教训**：`available_ports_render_inside_the_serial_info_section` 最初
想断言"界面里不含「可用串口」四个字"来证明独立折叠头没了——但合并后这四个字**仍被用作
串口信息区内的小标题**（`ui.weak(s.available_ports)`），该断言恒假。改为断言
**端口名出现 + 旧「接收统计」标题不出现 + 串口信息标题出现**这三条，才既抓得住"列表丢了"
也抓得住"旧标题又回来了"。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **266 passed / 1 ignored**（261 → +5）· clippy **8 条既有**（0 新增）。

**未验证**：只过了无头渲染（"画得出来不 panic"）与绘制文字断言，**外观从未目视**——
状态栏右侧留白是否够、合并后的串口信息区是否过长，都需要实机看一眼。
（3.12 已记录：状态栏这一项随后经用户目视，判定"正确"，并据观感追加了两条要求。）

### 3.12 增量（2026-09-24：状态栏加高 + 整组居中）

**用户反馈**：「目测审查通过正确，把底部状态栏加高，然后让底部状态栏的信息居中，
视觉上脱离底部的边框和顶部的边框」。即 3.11 的成果**观感合格**，但版式再调三处：
更高、居中、与上下边框拉开。

**做了什么**（全部在 `src/status_bar.rs`）

| 要求 | 实现 |
|---|---|
| 加高 | `Frame::NONE.inner_margin(Margin::symmetric(BAR_PADDING_X, BAR_PADDING_Y))`，`BAR_PADDING_Y = 10`（编译期断言 > 0）。上下各留 10 点，内容在栏内垂直居中 |
| 居中 | 整组（圆点 + 状态词 + 分隔符 + 两个读数）**水平居中**，取代原先的"左状态右读数" |
| 脱离边框 | 同上留白 + `show_separator_line(false)` 关掉面板自带分隔线（那是"贴上面那条边框"的主要观感来源）。**窗口最外框**归操作系统/egui，不去动它 |

**居中为什么是"算出来的"而不是"声明出来的"**——本轮最值得记的一条：

试过并**实测全部失败**（无头渲染下整组文字落笔位置都停在 x=0，贴左）：

1. `ui.with_layout(Layout::left_to_right(..).with_main_align(Align::Center))`
2. `ui.vertical_centered(...)` 里放 `horizontal`
3. `ui.allocate_ui_with_layout(full_width, Layout::top_down(Align::Center))`
4. `ui.horizontal_centered(...)`

根因：这些 API 只**重排"区域矩形"**，而形状一旦 emit 就不会回移；且这一行默认
**撑满可用宽度**，"居中一条满宽的行"本就没有可居之处。实测整组中心一直是 146（贴左），
屏心是 500。

**采用的解法**：先量后画。
- `status_bar_row_width()`：把**完全相同的一行**画进一个 `invisible()` 的探测 `Ui`
  （`UiBuilder::new().max_rect(..).layout(left_to_right).invisible()`），读 `min_rect().width()`。
- 真正落笔时 `ui.add_space((avail - row_width) / 2)`，确定性、可断言。
- `status_bar_row()` 抽成单一方法：这一行**要画两遍**（量 + 画），两遍内容必须一致，
  否则量出的宽度与实画不符、居中就漂。

**两个踩过的二次坑**：
- 探测 `Ui` 若**可见**，会把圆点与文字**再落一遍**——界面上是重影，测试里"圆点数 1 变 2、
  中心从 500 偏回 136"。必须 `invisible()`：只抑制绘制，布局照常，`min_rect` 仍准。
- 探测 `Ui` 若不指定 `left_to_right` 布局，则每个控件各占一行，量到的是"最宽那个控件"
  （13.4）而不是整行之和（249.6）。

**新增 2 条几何守卫测试**（读的是**绘制出的文字包围盒**，不只是文字内容）：

| 测试 | 钉住什么 | 空转验证（注入 → 变红） |
|---|---|---|
| `status_bar::tests::status_bar_is_taller_than_one_line_of_text` | 文字底边到屏幕底边的空隙 ≥ `BAR_PADDING_Y`（= 下留白） | 去掉 `Frame` 的 `inner_margin` → 红 |
| `status_bar::tests::status_bar_content_is_horizontally_centred` | 整组文字包围盒中心与容器中心相距 ≤ 12 点 | `pad` 写死 0 → 红；`pad` 改成贴右 → 红；行宽写成常量 → 红 |

**关于 12 点容差的诚实说明**：测试只取 `Shape::Text` 的包围盒，而这一行最左边是**圆点**
（几何图形，不是文字），其矩形比圆本身宽 ⇒ "文字中心"天然比"含圆点的整组中心"偏右约 9.4 点。
容差 12 覆盖了这个固定偏差 + separator 的零头。关键是**错误实现（贴左/贴右/右对齐）的偏移
都是数百点**，12 这条线足以把它们全拦下——容差必须远小于错误实现的偏移才有效。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **268 passed / 1 ignored**（266 → +2）· clippy **8 条既有**（0 新增）·
4 个注入探针全部变红后还原校验回绿。

**未验证**：加高后的留白是否"透气而不空旷"、居中后与上方中央区的视觉关系，仍需实机看一眼
（见 §4② 第 66 项）。

### 3.13 增量（2026-09-24：删掉顶部菜单栏的「发送」菜单）

**用户反馈**：「顶部的发送和发送区的右边功能重叠了，把顶部的发送取消」。经追问确认，
要删的是**顶部菜单栏的「发送」菜单**（不是发送区那颗发送按钮、也不是设置窗口里的分节标题）。

**重叠的实情**：`menu_bar.rs` 的「发送」菜单装的是——发送格式单选、行尾单选、
定时发送 + 间隔、发送历史；与「发送区」表头那一行右侧的控件**逐项重复**，连排列顺序都一样
（格式 → 行尾 → 定时 → 历史）。两处绑的是**同一批字段**（直接写 `self.*`），本就不存在
不同步；因此多出来的那套只剩干扰：用户改了一处会怀疑另一处是否跟着变了。

**做了什么**：

| 文件 | 改动 |
|---|---|
| `src/panels/menu_bar.rs` | 删掉整个 `ui.menu_button(s.send_title, ..)` 块，原位留一段注释说明**删了什么、为什么删、什么条件下复活**（`show_send_panel == false` 时若仍需改格式，先想清楚"没有发送区时改格式给谁用"）。模块头文件注释同步改为「文件 / 设置 / 视图 / 文本」 |
| `src/app.rs` | 把发送区表头那一行抽成 `ComTool::render_send_settings_row(&mut self, ui)`（原先内联在 `eframe::App::ui` 里）。理由与 `render_send_presets` 完全相同：`ui()` 要一个无头测试造不出的 `eframe::Frame`，写死在里面等于**永不被 `cargo test` 画到**。抽出来后它是格式/行尾/定时/历史的**唯一入口** |

**没有删任何功能**：格式 / 行尾 / 定时 / 历史四项一个不少，全在发送区表头那一行。
`send_format_radios` / `send_line_ending_radios` / `send_history_menu` / `MAX_SEND_HISTORY`
的 import 与定义都保留（发送区仍在用），删除后编译 0 warning 即是证据。
`s.send_title`（「发送」）也保留：中央区的发送按钮与设置窗口的分节标题都还用这个字。

**新增 2 条守卫测试，成对才成立**（`src/panels/mod.rs`）：

| 测试 | 钉住什么 | 空转验证（注入 → 变红） |
|---|---|---|
| `menu_bar_has_no_send_menu` | 顶层按钮行**不含** `send_title`，且**仍含** 文件/设置/视图/文本 | 把「发送」菜单加回来 → 红；把菜单栏砍到只剩「文件」 → 红（证明"仍含"那一半是活的，不是恒真） |
| `send_area_header_keeps_the_send_settings` | 画 `render_send_settings_row`，其中仍有格式选项（HEX/Escape）、行尾选项（LF/CRLF）、定时发送勾选框 | 删掉行尾那一组 → 红；整行不画 → 红 |

中英两列各跑一遍。

**两个专门避开的空转写法**（都值得记）：
1. **不能断言"整帧里不含「发送」二字"**——`send_title` 在中央区发送按钮、设置窗口分节标题
   里都出现，这些文字**不在菜单栏那一帧里**，但一旦把断言写成"整帧不含"，将来菜单加回来
   也照样绿。所以只驱动 `show_menu_bar`（新写的 `menu_bar_texts` 辅助函数），
   判据就是菜单栏自己的落笔文字。
2. **不能靠"子项没画出来"来证明菜单被删**——菜单是**折叠**的，未点击时格式/行尾这些子项
   根本不 emit。真正能区分的是**顶层按钮那一行**里还有没有「发送」这个按钮。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **270 passed / 1 ignored**（268 → +2）· clippy **8 条既有**（0 新增）·
4 个注入探针全部变红后还原校验回绿 · `cargo build --release` 后 exe 时间戳
（15:14:44）**晚于**最后一次 `src/` 改动（15:14:05），用户目视到的是含本次改动的程序。

**未验证**：菜单栏少一项后的观感（左侧是否显得空）需实机看一眼（见 §4② 第 67 项）。
菜单栏现有的 文件 / 设置 / 视图 / 文本 四项功能未动。

### 3.14 增量（2026-09-24：删掉中央区顶部的「⚙ 设置」按钮）

**用户反馈**：「侧边栏对面的设置和顶部的设置功能重合了，保留顶部的设置，删除侧边栏对面的设置」。
经追问确认，要删的是**中央区最上面那一行右侧的「⚙ 设置」按钮**
（用户点明了 "⚙ 设置" 三个字，见对话）——不是顶部菜单栏的「设置」菜单。

**重叠的实情**：两处**做的完全是同一件事**——都只是把 `self.show_settings` 置 `true`，
点开的是同一个 `egui::Window`。区别只在位置：菜单栏的「设置 → 打开设置窗口」在顶部，
中央区工具栏那颗按钮在侧边栏旁边（"对面"）。

**做了什么**：

| 文件 | 改动 |
|---|---|
| `src/app.rs` | 把中央区那一行抽成 `ComTool::render_top_toolbar(&mut self, ui)`（原先内联在 `eframe::App::ui` 里），**只保留左侧那颗侧边栏显隐开关**；删掉右侧 `with_layout(right_to_left)` 里的「⚙ 设置」按钮。抽出来仍是为可测：`ui()` 要一个无头测试造不出的 `eframe::Frame` |
| `src/strings.rs` | **删掉 `settings_button` 条目**（15 列）——它只被那颗按钮用，删按钮后它成了 dead code（`cargo check` 直接报 `field settings_button is never read`）。用的是编译器的警告当判据，不是"我看好像没人用了" |

**为什么 `strings.rs` 也必须改**：这条是本轮值得记的一点——删了唯一的使用点之后，
**编译器**（而不是人工排查）指出了文案表里的死条目。`strings!` 生成的字段带 `pub(crate)`，
`dead_code` lint 照样覆盖，所以"漏删文案"会以 `0 warning` 门禁失败的形式暴露出来。

**没有删任何功能**：设置窗口仍可从**菜单栏「设置 → 打开设置窗口」**打开——那是现在**唯一**
的入口。`s.settings_title`（「设置」）保留：菜单栏顶层菜单名与设置窗口标题都用它。

**新增 2 条守卫测试，成对才成立**（`src/panels/mod.rs`）：

| 测试 | 钉住什么 | 空转验证（注入 → 变红） |
|---|---|---|
| `top_toolbar_has_no_settings_button` | 只驱动 `render_top_toolbar`，其中**不含** `⚙ 设置` / `⚙ Settings` 字面量，且**仍含**侧边栏显隐开关 | 把按钮加回来 → 红；整行不画 → 红（证明"开关仍在"那一半是活的） |
| `menu_bar_still_offers_settings` | 只驱动 `show_menu_bar`，其中**仍含**「设置」菜单，且仍含文件/视图/文本 | 删掉「设置」菜单 → 红；删掉「文本」菜单 → 红 |

中英两列各跑一遍。

**判据用字面量而非字段引用**：`settings_button` 已从文案表删除，引用不到，所以测试里直接写
`"⚙ 设置"` / `"⚙ Settings"`。副作用是它同时钉住了"文案表里没把这一条加回来"——若有人只把
按钮加回 `render_top_toolbar`、又顺手把条目加回 `strings.rs`，两处都会撞在这条上。

**又一次要用"只驱动被测那一块"**（与 §3.13 同一个坑）：**不能**断言"整个界面里不含「设置」二字"
——菜单栏顶层菜单标题就叫「设置」（`s.settings_title`）、设置窗口自己的标题也是它。
所以判据必须落在 `render_top_toolbar` 这一块自己的落笔文字上。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **272 passed / 1 ignored**（270 → +2）· clippy **8 条既有**（0 新增）·
4 个注入探针全部变红后还原校验回绿 · `cargo build --release` 后 exe 时间戳
（15:34:27）**晚于**最后一次 `src/` 改动（15:33:52）。

**未验证**：顶部那一行只剩一颗侧边栏开关后，右侧会留白（原先那里是那颗按钮），
观感需实机看一眼（见 §4② 第 68 项）。设置窗口的打开方式少了**一个**入口，功能未少。

### 3.15 增量（2026-09-24：修掉 §5 的「既有缺陷三连」）

**用户指令**：「把123该做的就做了吧，实施」——`GAP_ANALYSIS_VS_COMTOOL.md` §0 里
列为「真正值得做的 4 项」中的前三项（第四项「发送文件」的硬前置就是①，留待下轮）。
三项是一条**依赖链**，一起做最便宜。

#### ① 待发队列背压（`src/communicationtrait.rs` + 三个通道实现）

**问题**：串口 / TCP 的 `outgoing` 是**无界** `VecDeque`。设备长期不可写
（驱动缓冲满 / 硬件流控掐住 / 对端不读）时用户每点一次发送就追加一段，只增不减，
最终吃光内存；「定时发送」10 ms 一包会把这个过程加速到分钟级。

**做法**：把上限与判定放进**共享的** `worker` 模块，三个实现都调它——各写一份
迟早有一份走偏，而且偏的那份不会有人发现。

| 新增 | 说明 |
|---|---|
| `worker::MAX_PENDING_BYTES = 1024 * 1024` | 1 MiB。串口工具的现实负载是"几十字节的指令 + 偶发几百 KB 的固件块" |
| `worker::PendingLedger` | **两侧共享**的在队字节数（`Arc<AtomicUsize>`）。上限判定就落在这里：`reserve` 用 CAS 循环，边界闭（恰好等于上限接受），拒绝时把在队字节数带出来拼消息 |
| `worker::submit(ledger, &Sender<WorkerCommand>, bytes, noun)` | 三个通道的 `write()` 共用：**先预约额度 → 再投递 → 投递失败退还** |
| `worker::retire_finished_chunk(&mut queue, finished, &ledger)` | 队首整段写完后**出队 + 归还额度**——两件事必须一起做，合成一处 |
| 三个 `worker_main` | `Transmit` 只做 `push_back`，**不再判上限**（判定只有一处实现；判两遍迟早分叉成两套口径） |

**两个判据上的取舍**：

* **按合计字节限流，不按条数**。一条 4 MiB 报文与四千条 1 KiB 指令对内存的压力相同，
  按条数会放过前者、误伤后者（`ledger_counts_total_bytes_not_entries` 钉住）。
* **数的是"交出去、但一个字节都还没写出去"的那一段**（含还在 mpsc 里没被取走的），
  不是"后台队列里的"——压着没发出去的就是压着。

#### ①附 背压的判定从后台线程挪到调用线程（同日第二轮，亦即本节初版的订正）

初版把上限判在**后台线程的队列**上（`push_bounded` / `accept_transmit`，拒绝时发一条
`Notice`）。当天读码就发现**信息回路是单向的**：判定结果回不到调用线程，`write()`
一律返回 `Ok`，于是 `app::ComTool::transmit` 照常 `tx_bytes += len`、`send_data` 照常
清空输入框——用户会看到「本条未发送」的提示与**仍在增长**的发送计数并存，刚敲的内容
还被清掉。这与 `transmit` 注释里 N4 那句"排队时计数会显示出一个并不成立的『已发送』"
直接矛盾（异步投递下 `Ok` 本身只是"已排队"）。

**改成账本之后两个症状一起消失，而且 `app.rs` 一行都不用改**——它本来就是"`Ok` 才记账、
`Ok` 才清空"。`push_bounded` / `accept_transmit` 及其 5 条测试已删除，换成 `worker_tests`
的 9 条账本测试。

**`Notice` 那条路随之退场**：队列满不再由后台线程报，而是 `write()` **当场**返回
`Err(待发队列已满（在队 N 字节，上限 1048576 字节），本条未发送)`，走 `app` 既有的
"发送失败"通路显示在接收区。文案刻意与原来逐字一致（同一个判据不该长出两种说法），
差别只在**当场说**。`ChannelEvent::Notice` 本身仍在用（串口设置 RTS / DTR 失败那条
非致命提示）。

⚠️ **一处必须记住的耦合**：`open()` 里必须 `ledger.reset()`。通道对象在连接类型不变时
会被**就地复用**（重连 = 同一个句柄再 `open` 一次），而 `close()` 时队列里那些**确实没
写出去**的数据的额度不会被归还——不清的话，用户第二次连上会"什么都没发就报队列满"。
三个实现各有一条 `reopening_the_channel_clears_the_stale_quota` 钉住。

#### ② 自动重连退避（`src/app.rs::auto_reconnect` + `src/receive.rs`）

**问题**：固定 500 ms、永不放弃。串口被别的程序占用时每秒重试两次、每次新建一个
后台线程，注定失败还刷屏。

**做法**：

* 纯函数 `reconnect_backoff(failures) -> Duration`：`500ms × 2^failures`，封顶 30 s。
  抽成纯函数是为了能断言**绝对时长**（500/1000/2000/4000/8000/16000/30s/30s），
  只断言"单调递增"会被"每次 +1ms"骗过。
* 新字段 `reconnect_failures: u32`（`#[serde(skip)]`，**纯运行时不落盘**）。
  在 `receive_data` 的 `Failed` 分支 `saturating_add(1)`，在 `Bytes` 分支**清零**
  ——收到的字节是**不可伪造**的成功证据（串口 `open()` 是乐观的，`connected`
  区分不了"打开了"和"待会儿才失败"）。
* `MAX_RECONNECT_ATTEMPTS = 6`（`pub(crate)`，`receive.rs` 要引用）。到顶后
  `auto_reconnect` 彻底停手，`receive_data` 在**刚跨过阈值那一帧**留一条
  「自动重连已停止（连续失败 6 次），请手动点「连接」重试」——只在跨线时提示一次。
* `close_connection` 把计数清零：主动断开是这一轮的结束，下次该是干干净净的一次尝试。
* `Closed`（对端**正常**关闭）**不计**失败：它多半是用户主动关的或设备正常下线，
  算进去会让"关掉再开"一次比一次慢。

用户重新点「连接」即可恢复（`open_connection` 里……实际是 `close_connection` 清，
断线路径则由 `Bytes` 清）。

#### ③ `connected` 假阳性（串口侧）——本次最核心的一改

**问题**：串口 `open()` 只要把后台线程 `spawn` 起来就返回 `Ok`，`open_connection`
随即无条件 `connected = true`。打开一个**不存在**的端口时，界面先亮一下「已连接」、
等 `Failed` 回来再跳回「未连接」；用户在亮着的那一瞬点发送，字节交给一个不存在的
端口。`FEATURE_GAP_AND_PLAN.md` §4 标为「高」，一直挂着"要引入'正在打开'这个中间
状态，属独立课题"。

**做法**：中间状态**落到现有的三态位**上（`AtomicU8` 当初就是为此用 `u8` 而非 `bool`）。

| 新增/改动 | 说明 |
|---|---|
| `worker::OPENING = 2` | 第三态：「后台线程已拉起，端口还没打开」 |
| `SerialChannel::open()` | **先置 `OPENING` 再 spawn**。顺序即契约：UI 置位必须先于 spawn（否则 `open()` 返回后的空档看着像 `IDLE` = "没在跑" = "失败了"），也必须先于 worker 的 `ACTIVE`（否则端口开得快时会被踩回 `OPENING`） |
| `worker_main` | `open_port()` **成功之后**才 `state.store(ACTIVE)`；失败直接 `Failed` 返回，`StateGuard` 归 `IDLE` |
| `ByteChannel::is_ready()` | 新 trait 方法，**默认转发 `is_active()`**，**只有 `SerialChannel` 覆盖**。网络通道的 `open()` 在调用线程里就做完 connect / bind，返回 `Ok` 即已就绪，两者恒等——给它们各编一个实现没有意义，还会多出漂移点 |
| `ComTool::channel_is_ready()` | 唯一判据：`self.channel.as_ref().is_some_and(\|c\| c.is_ready())`。**不按具体类型 downcast**——`is_ready` 就是那唯一的岔口 |
| `ComTool::opening()` | `self.connected && !self.channel_is_ready()` |
| `ComTool::settle_connection_state()` | 每帧一次（`ui()` 里紧跟 `receive_data()` 之后）。**只升不降**：通道变为不就绪 ≠ 掉线，那要由 `Failed`/`Closed` 说（只有它们知道"为什么"）。幂等靠 `announced_open` 一次性记账位 |
| `announced_open: bool`（`#[serde(skip)]`） | "已打开/已连接那条提示是否补过"。通道就绪会持续很多帧，没它提示每帧刷一条。凡是 `connected` 变假的地方（`teardown_channel` / `Failed` / `Closed`）都清它，好让重连再补一次 |
| `open_connection()` | 串口那句成功提示改成「**正在打开** …」，真"已打开"交给结算补。网络通道仍在当场报"已连接"，并把 `announced_open` 置真 |

**发送闸门全部改看"通道就绪"**（`send_data` / `send_preset` / `tick_repeat_send`）
——继续看 `connected` 就是把同一个缺陷挪到下一层：界面显示"已连接"、用户点发送、
字节没有去处。`connected` 的语义收窄为"**已经发起过一次打开**"的记账位
（决定 `close_connection` 有没有活干、`auto_reconnect` 要不要动、断线提示怎么分叉）。

**界面新增第三档「正在打开」**：

| 位置 | 呈现 |
|---|---|
| 侧边栏状态行 | `status_opening`（「状态: 正在打开」）——与 `status_connected` 并列的新一档 |
| 侧边栏按钮旁 | `opening_hint`（「正在打开…」）。**与「请先选择端口」是两回事**：那个说"点下去不会有反应"，这个说"点了、正在等" |
| 侧边栏连接按钮 | 「正在打开」归到**已发起**那一侧（显示「关闭串口」）——否则按钮读作「打开串口」却执行关闭，又一次"文案与动作相反" |
| 状态栏 | `statusbar_opening`（短词「正在打开」）+ **琥珀色**圆点（`ConnectionIndicator::is_pending`） |

`ConnectionIndicator` 新增 `is_pending` 字段（不是把颜色挤进 `label`）：**亮度**与
**文案**是两个维度——"正在打开"的灯语义上是"还没亮"，而文案又与"未连接"完全不同。
`color()` 三态：绿（通）/ 琥珀（等待）/ 红（不通）。

**为什么「正在打开」必须单独一档**：并进"已连接"就是缺陷本身；并进"未连接"则会让
一次失败的打开看起来像"根本没试过"（界面上是先「正在打开」、随后「未连接」+ 失败说明）。

**新增测试 19 条**（272 → 291 passed / 1 ignored）：

| 文件 | 测试 |
|---|---|
| `communicationtrait.rs::worker_tests`（+5） | ~~`push_bounded_accepts_within_capacity` / `..._rejects_over_capacity_without_mutating`（断言**队列未被改动**，"先 push 再报错"的半吊子实现过不了）/ `..._boundary_is_inclusive` / `..._counts_total_bytes_not_entries` / `accept_transmit_reports_notice_on_overflow`~~ —— **这 5 条已作废**，同日第二轮换成账本方案的 9 条（见 ①附 与 ④），保留在此只为解释"为什么 291 这个数字后来变了" |
| `serial_impl/channel.rs`（+3） | `is_ready_is_false_until_the_port_is_actually_open` / `open_transitions_through_the_opening_state`（**直接读状态位**并禁止 `ACTIVE`——只看结果的版本会被"存 `IDLE` 而不是 `OPENING`"骗过）/ `is_ready_agrees_with_is_active_outside_the_opening_window` |
| `app.rs`（+10） | `reconnect_backoff_doubles_until_capped`（断言绝对时长）/ `channel_failure_increments_reconnect_failures` / `receiving_bytes_resets_reconnect_failures` / `reconnect_gives_up_after_max_attempts` / `explicit_close_resets_reconnect_failures` / `serial_open_does_not_claim_connected_before_the_port_is_ready` / `sending_is_blocked_while_the_port_is_still_opening` / `settling_never_downgrades_an_established_connection` / `settle_wording_follows_the_connection_kind_not_the_channel_kind` / `settle_announces_open_only_once_per_connection` |
| `status_bar.rs`（改写 1 条 + 新增 1 条） | `indicator_covers_all_four_states` → **`indicator_covers_all_five_states`**；`status_bar_agrees_with_the_sidebar_on_opening` |

**测试替身新增"异步就绪"模式**：`FakeHandle::defer_ready()` / `signal_ready()`，
让替身能扮**串口**（`open()` 后仍不就绪）。没有这一档，替身一律同步就绪，
串口与 TCP 的行为不可区分，"UI 不得乐观置位"这条约定永远测不到。

**空转验证（5 个注入，每个至少命中一条）**：

| 注入 | 变红的测试 |
|---|---|
| 串口 `open()` 存 `ACTIVE` 而不是 `OPENING` | `is_ready_is_false_until_...`、`open_transitions_through_the_opening_state` |
| 发送闸门回退成 `if !self.connected` | `sending_is_blocked_while_the_port_is_still_opening` |
| `opening()` 恒为 `false` | `serial_open_does_not_claim_connected_...`、`sending_is_blocked_...` |
| 结算去掉 `announced_open` 记账位 | `settle_announces_open_only_once_per_connection` |
| 状态栏去掉 `opening` 那一档 | `indicator_covers_all_five_states` |

⚠️ **一条诚实的记录**：第一个注入**不**命中那两个 app 层测试——因为替身的
`is_ready()` 故意镜像它自己的 `active` 标志、与 `SerialChannel` 的内部实现无关。
这是**设计如此**：那两条测的是"app 如何处置一个未就绪的通道"，`SerialChannel`
自己的状态机由 serial 那三条覆盖。**不是漏测，但值得写明**，免得后来人以为
"注入没全红 = 探针坏了"。

**探针脚本踩到的坑（值得记）**：`shutil.copy2` 会连 mtime 一起复原，还原后若内容
与注入前逐字节相同，**cargo 判定未变化 → 不重编 → 后续测试跑的是带注入的旧二进制**，
于是"还原后仍绿"这条复核假红。修法是还原后显式 `touch` 一次源文件。

**门禁**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **291 passed / 1 ignored**（**当日第三轮后为 302**，见 ④）· clippy **8 条既有**（0 新增）·
`cargo build --release` 后 exe 时间戳（16:18:04）**晚于**最后一次 `src/` 改动（16:17:08）。

**未验证（需实机 · 见 §4② 新增第 69~71 项）**：串口打开失败的**那一瞬**是否
确实先显示「正在打开」再跳「未连接」（而不跳过中间态）；琥珀色圆点的观感；
以及"打开极快时是否根本不闪中间态"（预期：正常端口几乎看不到「正在打开」，
这是**对的**——它只在真的等的时候出现）。

**顺带的影响**：`GAP_ANALYSIS_VS_COMTOOL.md` §0 里「真正值得做 4 项」已完成 3 项，
只剩「发送文件」（其硬前置①已完成，可以立项了）。

#### ④ TCP 写侧改为非阻塞（同日第三轮：修掉 §5「TCP 写侧没有写超时」那一行）

**问题**：`TcpChannel::open()` 只调了 `set_read_timeout`，**写侧是阻塞 socket、没有
`SO_SNDTIMEO`**（串口那边 `.timeout(10ms)` 在 Windows 上同时落成
`WriteTotalTimeoutConstant`，所以**只有 TCP 有这个问题**）。两条后果：

1. 对端**彻底停读**时 `send` 永不返回（实测灌满约 2.75 MB 内核缓冲后 **2 分 36 秒**无进展）
   ⇒ worker 卡在 `write` 上 ⇒ 它再也不会回到主循环，**1 MiB 上限形同虚设**，
   而真正无限增长的是**无界的 mpsc 通道**（背压①白做了）。
2. `close()` 是「投 `Halt` 后 `join()`」，worker 只能在两轮 I/O 之间看到 `Halt`
   ⇒ UI 线程一直等它：对端慢读时实测单次 `write` 阻塞 **1.168 s**，点「断开」就得等这么久；
   彻底停读时**等于卡死**。

**为什么选「非阻塞写」而不是「加一行写超时」——实测决定的，不是偏好**：

> ⚠️ Windows 上带 `SO_SNDTIMEO` 的阻塞 `send` 超时后，数据**可能已经进入内核缓冲并发出**，
> 而调用方只拿到 `WSAETIMEDOUT`、**拿不到已写字节数**；worker 于是重发整片。
> 实测（64 KB 一片、片内含片号、超时 10 ms、对端停读 2 秒）：同一片在接收端出现 **126** 遍
> （片 42 的词数 1032192 = 126×8192）。**这不是"没修好"，是"这么修会更糟"**——它会污染
> 串口流本身。

非阻塞写没有这个歧义：`Ok(n)` 就是**准确**的已写字节数，`WouldBlock` 单独一档，
既不阻塞也不重发。

**做法**：

| 改动 | 说明 |
|---|---|
| `open()` | `set_read_timeout(…)` → `stream.set_nonblocking(true)`（另加 `set_nodelay(true)`，小包指令不该被 Nagle 攒着） |
| `IDLE_SLEEP = 1 ms`（常量） | 非阻塞后**读侧不能再用 `set_read_timeout`**（阻塞性是**每 fd** 的，读写一起变），空转必须自己节拍。它同时决定 `close()` 的最大响应延迟——两个用途写在同一处注释里 |
| `worker_main` 的 `idle` 标记 | 每一轮开始时若"上一轮一个字节都没动过"，`sleep(IDLE_SLEEP)`。`Ok(n)` 与收到 `Transmit` 都清 `idle`——有活干时不该被节拍拖慢（回环上大流量能跑满，不该被压到 1000 包/秒） |
| `write()` | 走与串口同一套 `submit(&self.ledger, …)`，队列满**在调用线程当场**返回 `Err` |

**UDP 刻意不改**（同日做了对照实测，不是顺手省略）：向一个**从不 `recv`** 的对端发
4000 × 60 KB 报文（`SO_SNDBUF=4096`）→ **0 次失败**、单次 `send` 最坏 **0.1 ms**；
同一实验的 TCP 对照组**挂了 2 分 05 秒**。⇒ UDP 没有"对端停读 ⇒ worker 卡住"这条路径，
改非阻塞只会换来一个不必要的节拍与一圈新代码。本机网卡**出口**被打满时 `send` 仍可能阻塞
（未实测），已作为残余风险记在 §5。

**新增测试 11 条**（**291 → 302 passed / 1 ignored**；净数 = 新增 16 − 删除 5）：

| 文件 | 测试 |
|---|---|
| `communicationtrait.rs::worker_tests`（+9，同时**删掉**原 `push_bounded` 那 5 条） | `ledger_hands_out_quota_up_to_the_cap` / `ledger_counts_total_bytes_not_entries` / `ledger_release_returns_the_quota` / `ledger_reset_clears_the_quota` / `retiring_a_finished_chunk_pops_it_and_returns_the_quota` / `retiring_on_an_empty_queue_is_a_no_op` / `submit_refuses_when_the_ledger_is_full_and_queues_nothing`（⭐ 断言 `rx.try_recv().is_err()`——**一个字节都没交给 worker**，"先投再报错"的半吊子实现过不了）/ `submit_hands_the_bytes_to_the_worker_and_charges_the_ledger` / `submit_returns_the_quota_when_the_worker_is_gone` |
| 三个通道（各 +2，共 +6） | `oversized_payload_is_refused_at_the_call_site` ×3（串口 / TCP / UDP，都断言**在调用点**就被拒）/ `reopening_the_channel_clears_the_stale_quota` ×3 |
| `net_impl/tcp.rs`（另 +1） | ⭐ `close_returns_promptly_even_when_the_peer_never_reads`——对端**一个字节都不读**，`close()` 也必须 3 秒内返回 |
| `app.rs` | **0 条**——刻意的：判定挪到调用线程后，app 既有的 `tx_bytes_counts_only_writes_that_succeeded` / `tx_bytes_resets_on_teardown` / `disconnected_app_writes_nothing` 就是**因为对的原因**通过的，一行都不用改 |

⚠️ **那条 `close_returns_promptly…` 第一版是假绿，值得记**：它一开始只是"投一批、看 `close()`
回不回得来"，可对端不读时**内核缓冲**才是真正会堵住 `write` 的东西，而账本满只说明
"有 1 MiB 压着没写出去"。结果是**内核还有余量把积压全吃掉**，worker 根本没被卡住，
测试于是空转（注入"撤掉非阻塞"它照样绿）。修法是先**灌满内核缓冲**：持续投 2 秒、
要求投出去 **≥ 2 MiB** 且**至少被拒一次**，否则直接 `assert!` 报"这条测试没测到要测的东西"。

**空转验证（4 处注入，跑 `.workbuddy/tmp_probe_ledger.py`）**：

| 注入 | 变红的测试 |
|---|---|
| G1 TCP 撤掉 `set_nonblocking`、换回阻塞 + 读超时 | `close_returns_promptly_even_when_the_peer_never_reads` |
| G2 `retire_finished_chunk` 只出队、不归还额度 | `retiring_a_finished_chunk_pops_it_and_returns_the_quota` |
| G3 `submit` 不再预约额度（判定退回后台线程） | `submit_refuses_…` + 三个 `oversized_payload_is_refused_at_the_call_site`（共 4 条） |
| G4 TCP 的 `open()` 不再 `ledger.reset()` | `reopening_the_channel_clears_the_stale_quota` |

四处**全部按预期变红**，还原后 `302 passed / 0 failed`。

**探针脚本本轮又踩到两个记录常新的坑**：① 把测试尾部的 `error: test failed, to rerun pass …`
当成了编译错误 ⇒ 三条真红的注入被误报成"注入过于激进"（只认 `error[E` / `could not compile`）；
② G1 第一次真绿是**探针自己**的 bug——填充循环在第一次账本拒绝时就停了，内核缓冲压根没满
（与上面那条测试是同一个根因，只是发生在工装侧）。

**门禁（本轮结束）**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **302 passed / 1 ignored / 0 failed** · clippy **8 条既有**（0 新增）·
`cargo build --release` 时间戳晚于最后一次 `src/` 改动。

---

### 3.16 增量（2026-09-24：修掉「颜色 + 字节查看」导致的接收区空白）

**来源**：用户**实机目视审查**时发现——「颜色」与「字节查看」同时勾选后，接收区**内容丢失**
（一片空白）。这直接推翻了同一天阶段报告里"只是被另一视图盖住、不丢内容"的判断。那个判断错在
**只读了渲染分支、没有核对镜像落点**（`receive_data()` 到底写的是哪一份）。用户是对的。

#### 根因：三处判定不同源

| 位置 | 判据 | 看过 `color_view` 吗 |
|---|---|---|
| 片段构造（`receive_data` 的 `if/else` 链） | `hex_view` → `ascii_view` → 文本 | ❌ |
| 镜像落点（`to_ascii_mirror`） | `!hex_view && ascii_view` | ❌ |
| 渲染分支（`app.rs` 中央区） | **`color_view` 最优先** → `ascii_view` → 其他 | ✅ |

于是 `color_view = ascii_view = true` 时：数据被写进 `ascii_spans`（前两处一致地选了字节模式），
界面画的却是 `display_spans`——**那一份从来没有人写过**。屏幕上一片空白，用户观察到的就是
"日志内容丢了"。

**它为什么能从全部自动化测试里溜过去**：既有测试测的都是"镜像写对了没有"，而这次**镜像写对了、
画错了**；"画的是哪一份"当时没有任何自动化判据，因为渲染分发内联在 `eframe::App::ui()` 里，
测试造不出 `eframe::Frame`。

#### 修法

1. **唯一判定**：新增 `receive::DisplayMode`（`Color` / `Bytes` / `Hex` / `Text`）与
   `ComTool::display_mode()`。「片段构造」「镜像落点」「渲染分发」三处都读它。
   `display_mode()` 只在**非法组合**（手改存档）时才有多解，方向与启动修复一致
   （`hex` 胜 `ascii`、`ascii` 胜 `color`）；`hex + color` 是**合法**组合，返回 `Color`
   以保住它改动前的外观（彩色的 HEX 文本）。
2. **渲染分发抽成函数**：`ComTool::render_receive_area(ui)`。写在 `ui()` 里的话测试够不着
   （`DEVELOPMENT_GUIDE.md` 约定 3），"画的是哪一份镜像"就只剩人工目视一条路。
3. **互斥纳入「颜色」**：`resolve_display_mode` 增加第三个参数 `color_just_enabled`，规则仍是
   **最后被勾上的胜出**；「颜色」与「字节查看」互斥（「HEX + 颜色」照旧允许）。
   启动修复一并覆盖 `ascii + color`，**字节视图胜出**——只开字节查看是合法配置，且颜色只是
   文本模式的修饰，在转义字节上本就没有意义。
   两个入口（接收设置行、`panels/menu_bar.rs` 的「视图 → 显示模式」）都走同一裁决。菜单那条
   入口原先把「颜色」的复选框画在裁决函数**之外**，这是它能被同时勾上的直接原因。

#### 测试（302 → 308，+6）

| 文件 | 测试 |
|---|---|
| `receive.rs` | ⭐ `no_display_combination_leaves_the_receive_area_empty`——穷举**全部 8 种开关组合**，每种都真喂一批数据，断言"当前模式下渲染读的那份镜像非空"，且刻意**不做互斥裁决**（要覆盖"互斥没兜住"的组合） |
| `receive.rs` | ⭐ `the_receive_area_paints_the_mirror_of_the_current_mode`——三份镜像摆上互不相同的标记，从**画出来的字**反推画的是哪一份。这条才真正覆盖渲染分发 |
| `receive.rs` | `enabling_the_byte_view_turns_color_off` / `enabling_color_turns_the_byte_view_off`（方向对称：刚点的那项必须留下）/ `stale_color_and_byte_view_are_repaired_in_favour_of_the_byte_view` |
| `config.rs` | `byte_view_wins_over_color_on_load`；`ascii_view_alone_survives_round_trip` 顺带修正样本（显式关掉 `color_view`，否则它测的其实是非法组合，与测试名不符） |

#### 非空转验证：4 处注入全部变红

探针脚本 `.workbuddy/tmp_probe_display_mode.py`（不随仓库走，与上两轮的 `tmp_probe*.py` 一致）。

| 注入 | 靶测试 | 结果 |
|---|---|---|
| G1 渲染分发改回旧的 `if self.color_view { … } else if self.ascii_view { … }` | `the_receive_area_paints_the_mirror_of_the_current_mode` | ✅ 红 |
| G2 互斥里去掉「勾字节查看时关颜色」 | `enabling_the_byte_view_turns_color_off` | ✅ 红 |
| G3 启动修复不覆盖 `ascii + color` | `byte_view_wins_over_color_on_load` | ✅ 红 |
| G4 镜像落点恒不写字节镜像 | `no_display_combination_leaves_the_receive_area_empty` | ✅ 红 |

⚠️ **一处如实记录（不要记串）**：旧落点表达式 `!hex_view && ascii_view` 与新写法
`mode == DisplayMode::Bytes` **语义上等价**（`display_mode()` 的 `Bytes` 分支就是
`ascii_view && !hex_view`）。也就是说**本次真正改动的是渲染分支**，落点只是从"手工与分支对齐"
换成"读同一个判定"——它的价值在**将来**（新增视图时只有一处会漏），不在本次修复本身。
**不要把"三处判定收敛了"讲成"三处都错了"。**

**门禁（本增量结束）**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **308 passed / 1 ignored / 0 failed** · clippy **8 条既有**（0 新增）·
`cargo build --release` 后 exe 时间戳（18:13:56）**晚于**最后一次 `src/` 改动（18:13:39）。

---

## 4. 未决事项

### ① GPLv3 合规（**已降为最低优先级**）

> **2026-09-21 项目所有者决定**：合规决策排到**最低优先级**，先按功能计划推进（P5、P6）。
> 因此这一项从"必须先拍板"变成了"待办清单的最后一条"。
>
> **这只表示"决策被推迟"，不表示"风险已消除"。** 在做任何对外分发（发布 / 交付 /
> 上架）之前必须回来二选一——仓库目前仍是**无许可证状态**（无 `LICENSE`，
> `Cargo.toml` 无 `license` 字段），私下自用不受影响。

代码层面已消除与 `SerialGUI-rs` 之间逐字相同的表达，但**法律意义上的 clean-room 需要
「重写者从未接触过原实现」**——这件事只能在有第二个人参与时满足，同一个会话里做不到。

两条路，必须二选一（分发前）：

- **接受 GPLv3**：补 `LICENSE`（GPLv3 全文）+ `NOTICE`，`Cargo.toml` 加 `license = "GPL-3.0-only"`。代价是不再能闭源分发。
- **隔离重写**：找一位**没读过 `SerialGUI-rs`** 的人，只依据 `src/communicationtrait.rs` 的文档注释
  与需求重新实现 `serial_impl/channel.rs`。这个抽象本身只有 5 个方法，独立重写完全可行，约 1~2 天。

### ② 磁盘级持久化**从未实测**（**当前最高优先的待办**）

> **⚠️ 让用户实机 / 目视之前的强制前置条件（用户明确要求，2026-09-24）**
>
> 本节及 `§4②` 下列各项都是"请人工跑一遍"的清单。**在向用户提出这些请求之前，
> 必须先跑 `cargo build --release`**——`fmt` / `check` / `test` / `clippy` 都**不产出
> 可执行文件**，只跑这四样时用户双击到的可能是**不含本次改动**的旧 exe，他会看着旧
> 界面找新功能，然后得出"没做出来"的结论。
>
> - 判定"编译过没有"的唯一依据是**时间戳比对**：`target/release/comtool-rs.exe` 的时间戳
>   须晚于最后一次 `src/` 改动，且与 `git log -1 --format=%cd --date=iso` 对得上。
> - exe 在 `.gitignore` 里、**不随仓库走**：清过 `target/` 或换机器后都要现场重建。
> - 禁止话术："你自己 build 一下" / "跑过测试了可以看了"。
> - 另注：清单里的 `cargo run --release` 属**运行本项目**，按 §7 第 0 条仍需用户本人批准，
>   由用户自己执行；`cargo build --release` 则在默认允许清单内，不需要批准。
>
> 详见 `DEVELOPMENT_GUIDE.md` §6 约定 49。

单元测试走的是内存版 `eframe::Storage`，覆盖了 `save → set_value → RON → get_value → load`
全路径，但**真实文件读写没验证过**。原因：这是 GUI 程序，`save()` 只在退出或周期性自动保存时触发，
无法在不打开窗口的情况下测试。

请手动跑一遍：

1. `cargo run --release`
2. 改波特率为 `921600`、勾选「字节查看」、收起侧边栏
3. **正常关闭窗口**（不要用任务管理器强杀——可能跳过 `save()`）
4. 确认 `%APPDATA%\ComTool-RS\data\` 下生成了存档文件
5. 再次 `cargo run --release`，确认上面三项被恢复

**同一趟可以把 P4 一起验掉**（顺带做的事）：

6. 侧边栏展开「文件日志」→ 点「开始记录」→ 应看到日志文件名与输出目录；
   打开该文件，确认「开始记录日志: …」这一行已经落盘
7. 有数据来往时观察文件是否实时增长（低速下也应能立刻看到，因为队列一排空就 flush）
8. 点「停止记录」→ 文件末尾应出现「已停止记录日志」；界面恢复「未记录」
9. 再点一次「开始记录」→ 同一秒内会产生 `…_1.txt` 而不是覆盖上一个文件
10. 把文件名模板改成 `%Q`（非法占位符）→ 设置窗口里的预览应立即显示退回默认模板的名字

**同一趟也可以把 P5 图表验掉**（需要设备按「一行一组、逗号分隔」发数值，没有设备时
用任意串口助手手工发下面的文本即可）：

11. 视图菜单 →「面板 → 实时图表」→ 中央区应出现图表面板（首次为「等待数据…」）
12. 发 `1,2,3,4\n` → 应出现 4 条不同颜色的曲线，图例显示 CH1~CH4，通道数显示 4
13. 继续发几行不同数值 → 曲线应向右推进；勾掉「自动缩放」后手动滚轮缩放，
    视图应保持不变（不再被自动拉回）
14. 发一行 `1,2\n`（只有 2 个字段）→ CH3/CH4 应**断开**而不是掉到 0 ——
    这是刻意的：补 0 会画出从未到达的数据
15. 发一行 `hello\n` → 曲线不动，界面出现「忽略 1 个非数值字段」的提示
16. 点「清空图表」→ 曲线清空、通道数与样本数归零
17. 把设置里的「样本上限」改成 10，再连发 20 行数据 → 图表最多保留 10 个点/曲线

**同一趟也可以把 P3 时间戳前缀验掉**（设置窗口「接收 → 时间戳前缀」，或视图菜单）：

18. 选「每行」→ 之后每行开头应出现灰色的 `[HH:MM:SS.mmm]`；发一条不带换行的数据，
    再发一条以换行结尾的数据 → 前缀应只在行的开头出现，**不会**在同一行的后半段重复
19. 选「每批」→ 一个数据块只标注一次：同一块里的第二行**不带**前缀（与「每行」对照）
20. 分别切到 HEX 视图与字节查看视图（这两个现在**互斥**，需要分别切换，见下面 23~26）→
    前缀仍然出现（它是注解，不是数据）；把设置改回「关闭」→
    接收区内容应恢复成纯数据，与开启前完全一致
21. 开着「每行」时点侧边栏「清空日志」，再发一行 → 清空后的第一行应重新带上前缀
22. 开着前缀时开始记录日志 → 日志文件里的每一行也应带前缀（与接收区一致）

**同一趟也可以把显示模式互斥验掉**（本轮修复项，见 §3.6）：

23. 接收设置行先勾「HEX」，再点「字节查看」→ **「HEX」应自动取消**，接收区显示转义后的
    字节（不是空白）；反过来先勾「字节查看」再点「HEX」→「字节查看」应自动取消
24. 视图菜单 →「显示模式」里同样试一遍 → 行为必须与接收设置行一致（两处走的是同一条规则）
25. 退出程序再重新打开 → 显示模式保持你最后的选择，且**不会**停在"接收区空白"的状态
26. 若手边正好有修复前留下的存档（`hex_view` 与 `ascii_view` 都是 `true`），直接启动 →
    接收区应有内容（HEX 视图），而不是空白的字节视图

**同一趟也可以把 F1 / F2 / F3 一起验掉**（见 §3.7）：

27. **F1 发送格式**：发送区选「HEX」，输入 `48 656c` → 预览应显示「将发送 3 字节」
    （**三个**字节，不是把 `48 656c` 拼起来）；输入 `4`（奇数位）或 `zz` →
    预览应显示红色的「HEX 解析失败: …」，点「发送」**不清空**输入框
28. **F1 行尾**：切到「CRLF」+ HEX 模式 → 发出的应是 `0D 0A` 两个**字节**（用字节查看视图确认）
29. **F1 定时发送**：勾上「定时发送」的**那一刻不该有数据出去**；到点后才开始周期性发送；
    点「关闭串口」→ 勾选框应自动取消（重连后不会自己接着发）
30. **F1 发送历史**：光标在输入框首行按 ↑ → 取上一条历史；连按 ↑ 逐条往前；
    自己改一下内容再按 ↑ → 从编辑态重新开始
31. **F2 编码**：把编码切到「GBK」，鼠标走「发送」→ 用字节查看视图确认发出的是 GBK 字节
    （不是 UTF-8）；把「HEX 接收」打开看设备回显，中文应能正确解出（**这一条必须实机确认**）
32. **F3 语言**：设置窗口把语言切到 English → 菜单栏 / 侧边栏 / 设置窗口 / 信息窗口 /
    图表面板 / 发送区应**全部为英文，无中文残留**；再切回中文 → 与改动前**逐字一致**
33. **F3 不翻译的部分**：English 下打开一次串口再关闭 → 接收区里的「已打开 …」/「串口已关闭」
    应**仍然是中文**（这是决策：这些行会进日志文件与导出文件，不能随界面语言变）
34. English 下重开程序 → 语言选择应被记住（`language` 已落盘）
34a. **多语种扩展（2026-09-24，§3.10）**：设置窗口语言下拉现在有 **15 档**，每档写**自己的名字**
    （`中文（简体）` / `中文（繁體）` / `日本語` / `Español` / `Italiano` / `Português` /
    `Bahasa Indonesia` / `Français` / `Deutsch` / `한국어` / `Tiếng Việt` / `English` /
    `Türkçe` / `Русский` / `Polski`）。
    逐档切过去，重点看三件事：
    - **无方框 □**：切到 `日本語` 看假名、`한국어` 看谚文、`Tiếng Việt` 看变音字符
      （`Tiếng Việt` 的 `ế` `ệ` 等）、`Русский` 看西里尔字母、`Polski` 看 `ą ć ę ł ń ś ż`、
      `Türkçe` 看 `Ç ğ ı İ ö ş ü`、`Português` 看 `ê` / `ç`——**接收区（等宽字体）**
      与界面文字（比例字体）都要看，两个族的字体链不同。
    - **无残留中文**：每种语言下菜单栏 / 侧边栏 / 设置窗口 / 信息窗口 / 图表 / 发送区
      都不该出现中文（**唯一例外**是语言下拉里的档位自名）。
    - **文案不溢出**：西语 / 葡语 / 德语 / 波兰语比中文长 20~40%，看设置窗口与侧边栏的标签
      有没有被挤断或截断（`font_scale` 调到「大」再看一遍）；**俄语 / 波兰语的西里尔与变音
      字母横向更宽**，同样要看一遍长标签。
    切换语言**立即生效、不需要重启**（与 `language_applies_immediately` 的说明一致）。
34b. **越南语字体回退**：`Tiếng Việt` 档下接收区若收到带变音的数据，字形应正常显示。
    这是 `tahoma.ttf` 兜底生效的实机证据——自动化测试只验证了字体文件里有字形，
    **没有验证 egui 真的取到了它**。

**同一趟也可以把 F4 验掉**（见 §3.8）。先在本机开一个 `nc -l 1234`（或任意 TCP 监听），
再开一个 `nc -u -l 1234` 给 UDP 用——两者不能同时占 1234，按下面顺序分两次做：

35. **F4 连接类型分叉**：侧边栏「连接类型」选「TCP 客户端」→ 端口列表 / 波特率 /
    数据位 / 停止位 / 校验 / 流控**整段消失**，换成「远端地址 + 远端端口」；
    「可用串口」列表也一并隐藏（它现在并进了「串口信息」区，不是独立折叠头）；
    状态那一行的标题从「串口信息」变成「连接信息」
36. **F4 TCP 收发**：远端地址 `127.0.0.1`、端口 `1234` → 点「连接」→ 接收区出现
    「已连接 127.0.0.1:1234」，状态变「已连接」；双方互发文本（含中文，切 GBK 试一次）
    与 HEX；**对端 `nc` 退出** → 接收区应出现「TCP连接已关闭」且状态回到「未连接」
    （不能卡在「已连接」）
37. **F4 TCP 连不上**：把端口改成没人监听的（例如 `1`）→ 点「连接」→ 应立即失败并显示
    「连接失败: …」，**状态不能先变「已连接」再变回来**（网络侧是同步 connect）
38. **F4 UDP 绑定**：切到「UDP」，远端端口填监听端口 → 点「绑定」→ 接收区出现
    「UDP 已绑定，目标 …」；状态显示「**已绑定**」而不是「已连接」；「本地端口」留 0 时，
    连接后信息区那一行应显示**系统实际分配**的端口号（不是 0）
39. **F4 UDP 收发**：向 `nc -u -l` 那一端发一条文本 → 对端应收到；对端回一条 → 接收区应显示
40. **F4 连接中锁定类型**：连接状态下点「连接类型」的另一档 → 应点不动，悬停给出一句
    解释；先「断开」再切 → 切得动，且参数区跟着换
41. **F4 按钮文案**：未连接时按钮是「打开串口 / 连接 / 绑定」（按类型变），
    已连接时是「关闭串口 / 断开」——**文案与动作必须一致**（这条是 §3.8 修的既有缺陷）
42. **F4 落盘**：选 TCP、填好地址端口、退出程序再打开 → 连接类型与地址端口都应被记住
    （连接本身不会自动恢复，这是刻意的）
43. **F4 不碰串口**：全程不打开任何串口；把「连接类型」切回「串口」后，
    串口那六项参数应原样回来，行为与 F4 之前一致

**同一趟也可以把 F5 的 N1~N6 验掉**（见 §3.9）。**这一组里有三条只能实机确认**（N1 的
实机行为、N5 的非标波特率、N6 的焦点跳变），其余都是"看得见就说明对"：

44. **N1 打开不拉线**（关键风险项）：拿一块带自动复位电路、且**不**想被复位的板子 →
    点「打开串口」→ 板子**不该**复位；此时侧边栏「控制线」的两个勾选框都应是**未勾选**
45. **N1 手动控制**（关键风险项）：勾上 RTS → 用万用表/示波器看对应引脚应变高（或对
    一块 ESP32 手动走一遍下载时序：拉 DTR → 动 RTS → 松开，看能否进下载模式）；
    取消勾选应变低。**注意勾选框表达的是"我们请求的电平"，不是引脚测量值**
46. **N1 硬件流控档**：把流控改成「硬件」再勾/取消 RTS → 观察是否被驱动覆盖
    （`serialport` 在 Windows 上取 `RtsControl::Enable`，理论上手动 `EscapeCommFunction`
    仍生效，但**没有硬件验证过**，这一条就是去确认它）
47. **N1 断开归位**：勾上两根线后点「关闭串口」→ 下次打开时两个勾选框都应回到未勾选
48. **N2 常用数据**：输入框写 `AT+RST` → 点「存为常用」→ 出现一个按钮（标签 `AT+RST`，
    悬停显示完整内容）；点它 → 应发出**与直接点发送完全一样**的字节（含当前行尾与编码）；
    但**输入框内容不变**、也不该进发送历史
49. **N2 置灰理由**：输入框清空 → 「存为常用」置灰，悬停说「输入框为空」；
    输入框内容与某条重复 → 悬停说「已在列表中」；存满 16 条 → 悬停说「上限 16 条」
50. **N2 右键删 / 满不淘汰**：右键某个条目 → 该条消失；存满 16 条后再点「存为常用」→
    设置窗口里计数停在 `16 / 16`，**第一条不会消失**
51. **N2 落盘**：存两条后退出程序再打开 → 按钮还在（设置窗口里的计数也对）
52. **N3 转义**：格式选「转义」，输入 `AT\r\n` → 预览应是 **4 字节**（`41 54 0D 0A`），
    不是 6 个字符；输入 `\xC4\xE3` + GBK → 用字节查看确认发出的就是 `C4 E3` **两个**字节
    （没有被 GBK 再编码一遍）
53. **N3 非法转义**：输入 `\q` 或 `\x4` 或末尾孤立的 `\` → 预览显示红色错误，
    点「发送」**不发也不清空**输入框
54. **N4 发送计数**：侧边栏「接收统计」→「发送字节数」；发一次 `AT`（2 字节 + 行尾）→
    数字应按实际发出字节数增长；清空输入框或解析失败时**数字不动**；点「关闭串口」→ 归零
55. **N4 与接收口径不同**：接收字节数会随着日志上限被裁而变小/停止增长，发送字节数只增不减
    ——悬停提示里写明了这一点
56. **N5 任意波特率 / 自定义档**（需实机确认）：下拉里选 9600 → 旁边**没有**数字框；
    再打开下拉点最下面的「自定义…」→ 数字框**出现**且初始值是 `250000`（一个列表里没有
    的值，省去删字符）→ 把它改成 `250000` 或别的非标值 → 打开串口后设备应能正常通信。
    **关键的一条回归**：在自定义档里把数字改成 `9600`（列表里恰好有的值）→ **数字框必须还在**，
    下拉显示文本仍是「自定义…」，不能自己跳回 9600 那一档
57. **N5 越界夹取**：把数字框拖到 0 以下 / 拖到很大 → 应停在 1 与 12000000 之间，界面上
    不会出现 0
57b. **N5 档位随存档走**（可选）：在下拉里选 115200，退出程序；再启动 → 下拉应显示 `115200`
    且**不显示**数字框。反过来选自定义 + 一个非标值退出再进 → 下拉应显示「自定义…」且数字框
    里是你输的值
58. **N6 热插拔**（需真实窗口确认）：程序开着、鼠标点到别处（或最小化）→ 插上一个 USB 转
    串口 → 切回程序窗口 → 「可用串口」列表**自动**出现新端口，**且你原先选中的端口没被改掉**
59. **N6 手动刷新仍会修正**：在「端口」里选一个已经拔掉的端口 → 点「刷新」→
    这时才应该自动改选到第一个可用端口（与上一条的差别是刻意的）
60. **N3 的一处一致性**：设置窗口里选「转义」时，下面显示的应是**转义**的说明而不是 HEX 的
    （三档的说明互不相容，只显示当前那一档）
61. **深色主题下接收区可读性**（2026-09-24 修；颜色已可自动化验证，外观仍需人看）：
    把主题切到「深色」，接收区里的**普通文本、彩色、字节查看三种视图**都必须看得清。
    修之前三处默认色都硬编码成 `Color32::BLACK`，深色下是黑底黑字、整块不可见；
    修复后默认色取自 `ui.visuals().text_color()`，已由三条测试钉住
    （`receive::tests::default_text_color_follows_theme`、`ascii_view_default_color_follows_theme`、
    `explicit_ansi_color_overrides_theme_default`）。
    **仍需人眼确认的两件**：
    - 深色主题下时间戳灰与前缀灰、不可打印字节灰（`TIMESTAMP_COLOR` / `ASCII_NONPRINT_COLOR`，
      都是中灰）是否偏暗到看不清——**这两个刻意没改**（它们是数据编码，不是装饰），觉得暗请反馈；
    - 浅色主题下同样看一遍（修的时候不能只顾深色）。
62. **设置窗口滚动条**（2026-09-24 加；可滚性已自动化验证，外观需人看）：
    打开设置窗口后确认——
    - 窗口高度被**限制在一屏以内**（约 720px），底部「恢复默认目录」按钮不再被顶出屏幕；
    - 内容右侧**出现纵向滚动条**，可以一直滚到底部的「偏好随程序退出自动保存」那一行；
    - **滚动条出现/消失时，控件不该左右横向抖动**（这是 `auto_shrink([false, true])` 要防的：
      宽度不随滚动条收缩）；
    - 滚到中间后关闭窗口再打开，**预期重新回到顶部**（滚动位置不跨次保留；
      若发现停留在上次位置，属可接受行为但请反馈，会加一次复位）；
    - 把窗口**手动拉高**（右上角可拖）后，滚动条应随内容是否溢出自动出现/消失。
    自动化部分由 `panels::tests::settings_body_renders_inside_a_short_viewport` 覆盖
    （在 120px 高的可视区里画内容体，断言内容确实溢出）。**它证明不了外观**——
    滚动条长得对不对、有没有遮挡控件，只能人眼看。

63. **全局字体档位**（2026-09-24 加；接线已自动化验证，观感需人看）：
    设置窗口中段「字体大小」下拉，四档（小 / 标准 / 大 / 特大）逐档切一遍，确认——
    - **标题、正文、按钮、等宽文字一起变**；尤其注意**接收区和发送框**有没有跟着变
      ——它们是唯一原先写死过 14px 的地方，也正是「全局」这个词最容易名不副实的位置；
    - 切到「特大」后，**标签列（`LABEL_WIDTH = 104`）是固定像素宽**，某些较长标签
      （如「日志长度上限」）可能与右侧控件挤在一起——这是已知的取舍，记下是哪几行、
      挤压到什么程度，再决定要不要把标签列也改成随档位缩放；
    - **深色与浅色主题下各看一遍**（`all_styles_mut` 改的是两份 Style，如果只有一份变了，
      切主题时字号会跳回去——这正是那条约定要防的，但只有人眼能发现「切了才发现」）；
    - 打开设置窗口上下滚动时，**滚动条宽度是否随字号变化**而挡住控件；
    - 关闭程序再打开，**档位应当保持在上次选的那一档**（已由 `config` 往返测试覆盖，
      这里确认端到端真的落盘了——尤其确认换了档位后确实触发了一次保存）。
    自动化部分由三条测试覆盖：`app::tests::font_scale_baseline_matches_egui_defaults`、
    `app::tests::font_scale_is_idempotent_across_frames`、
    `panels::tests::font_scale_dropdown_is_wired_to_the_field`，外加
    `receive::tests::monospace_font_id_follows_the_style_not_a_constant` 钉住等宽那一路。
    **它们都证明不了「字真的变大了、变得好不好看」**——断言的是字号数值与接线关系。

64. **底部状态栏**（2026-09-24 加；**颜色 / 大小 / 垂直对齐三项已由用户目视确认通过**）：
    主界面最底部应当出现一条常驻状态栏。
    - **未连接时应看到红色圆点 + 未连接**；打开串口/连接成功后应变成 **绿色圆点 + 已连接**；
    - **两个圆点的颜色是否正确（绿得干净、红得不刺眼）、两颗圆点大小是否一致、
      是否与旁边文字在同一水平线上——这三项已由用户于 2026-09-24 目视确认正常。**
      初版用 🟢/🔴 两个 emoji 字符，界面上是**两个大小不一、且都是灰黑色**的圆——
      根因是 egui 只光栅化字体的矢量轮廓、不读 COLR/CPAL 彩色图层，换任何字体都得不到
      彩色。现已改为用 `Color32` 直接画实心圆，**不再依赖任何 emoji 字体**；
    - **UDP 连接类型下应当显示「已绑定」而不是「已连接」**（UDP 无连接，绑定成功≠对端在线，
      显示「已连接」会让人以为掉线会被发现）；与侧边栏的状态行措辞应当一致；——**未验**
    - 状态栏应当**横跨整个窗口宽度**（左边对齐到侧边栏左缘、右边到窗口右缘），
      而不只是中央区那一块——若左右露出缝隙，说明绘制顺序错了（必须在中央区之前画）；——**未验**
    - **把字体档位切到「特大」**，圆点应当跟着变大、且仍然小于行高（不应顶到相邻行）；——**未验**
    - 圆点悬停时应出现文字提示（圆点本身对读屏软件没有意义，有意义的字挂在悬停上）；——**未验**
    - 切到英文界面后应显示 `Connected` / `Not connected`（圆点颜色不变）。——**未验**
    - **版式为"整组居中"**（取代 3.11 的"左状态右读数"）：圆点、状态词、两个读数
      作为**一整组**出现在状态栏**水平居中**处，见第 66 项逐条比对。——**未验**
    自动化部分由 8 条测试覆盖（四态判据、颜色随状态变且方向正确、跟随语言、与侧边栏不矛盾、
    真的画出文字、真的画出一个正确颜色的圆、尺寸随行高、加高、居中）。**它们都证明不了
    「看起来顺不顺眼」**——高度、留白、分隔线是否合适，只能人眼看。

66. **状态栏加高 + 整组居中（2026-09-24 加；几何已自动化验证，观感需人看）**：
    本条是 3.12 的目视项。请打开程序看最底部那条状态栏：
    - **应当比一行文字明显高**：内容上下各有一段空白，不再像被压在一条窄缝里。
      若看着仍是"紧贴上下边"，说明留白没生效——本应上下各留 `BAR_PADDING_Y = 10` 点；
    - **整组应当在水平方向居中**：圆点 + 状态词 + 两个读数作为一整组，左右两边到
      窗口边缘的距离**看起来相等**。若整组明显偏左或偏右，居中就没做对；
    - **应当与上方中央区"脱开"**：状态栏顶部**不再有一条横贯的分隔细线**
      （已 `show_separator_line(false)` 关掉），内容与中央区之间只靠留白分隔。
      **窗口最外那一圈边框属于操作系统/egui，仍然存在，这是正常的、也不该去掉**；
    - **把字体档位切到「特大」再切回「小」**：留白与居中应当始终成立（留白是固定点数，
      内容随字号变、居中按实测宽度重算，所以两组下都该居中）；
    - **切到英文界面**（`Not connected` 比「未连接」长得多）：整组应当**重新居中**，
      而不是因为文案变长就偏向一边——居中是每帧按实测行宽算的，语言切换不应破坏它。
    自动化部分由 2 条**几何**测试覆盖：`status_bar_is_taller_than_one_line_of_text`
    （量文字底边到屏幕底边的空隙 ≥ `BAR_PADDING_Y`）与
    `status_bar_content_is_horizontally_centred`（量整组文字包围盒中心与容器中心的距离 ≤ 12 点）。
    两者都做过**注入验证**：去掉留白 / 把偏移写死 0 / 偏移改成贴右 / 行宽写死常量，
    四种破坏各自都让对应测试**变红**。**但它们证明不了"这个高度好不好看、居中后协不协调"**
    ——留白 10 点是拍板的取舍（太小看不出来、太大显空旷），是否合适只能人眼看。

65. **界面重构（2026-09-24 加；结构已自动化验证，观感需人看）**：
    本轮把三处内容搬了家，请按下面三条逐一比对，确认**搬完了、且没有搬重或搬丢**：
    - **收 / 发字节数搬到了底部状态栏最右侧**：应看到形如 `发: 4096 字节 │ 收: 0 字节`
      的右对齐文字，**左边仍是圆点 + 状态词**，两不打架；把接收区清空后「收」应归零，
      发送若干字节后「发」应跟着涨（与旧「接收统计」区里的两个数字是同一对，
      只是换了位置——**右侧边栏里不应再有这两个数字**）。悬停文字仍可用；
    - **「可用串口」整块并进了「串口信息」区**：串口信息区里先显示端口 / 波特率，
      再以小号灰字列出可用端口名（如 `COM7`）；**侧边栏不应再单独出现一个「可用串口」折叠头**
      ——若仍能折叠出两个标题，说明旧的没删干净。串口为空时应显示占位文案；
    - **「显示模式」「时间戳前缀」两行现在也挂在「串口信息」区末尾**：确认这两行仍随
      勾选项实时变化（HEX / 字节查看互斥那一套没被搬坏），且切到 UDP/TCP 等非串口连接时
      显示的是「连接信息」标题而不是「串口信息」。
    自动化部分由 5 条测试钉住结构（状态栏真的画出两个字节计数器、计数器随数据变化、
    侧边栏不再出现字节计数器、端口列表画在串口信息区里、显示模式/时间戳两行仍在），
    另有 7 个注入探针确认这些断言**非空转**（把被测内容换掉后确实变红）。
    **它们只能证明"内容出现在该出现的位置"，证明不了"搬完好不好看"**——两行右对齐会不会
    与左边文字贴太近、串口信息区被塞进端口列表后会不会过长，只能人眼看。

67. **菜单栏删掉「发送」菜单（2026-09-24 加；结构已自动化验证，观感需人看）**：
    本条是 3.13 的目视项。请打开程序看最顶部那条菜单栏：
    - **菜单栏顶层应当只有四项**：文件 / 设置 / 视图 / 文本。**不应再有「发送」**
      ——若还能点出「发送」并展开格式/行尾/定时/历史，说明删得不干净；
    - **发送区那一行仍然是这四项设置的唯一入口**：进发送区，确认表头那一行右侧
      仍有 格式单选（文本/HEX/转义）、行尾单选（无/LF/CRLF）、「定时发送」勾选框
      （勾上后出现间隔数值框）、以及发送历史下拉。**点几下确认真的能改**
      ——只"看到"不能证明接线还在（自动化只证明了它们被画出来）；
    - **功能区不应显得空旷**：删掉一项后菜单栏左半部分会短一截，看是否可接受。
    - **切到英文/其它语言**：四项菜单名应当照常显示，不应出现方框或空位。
    自动化部分由 2 条测试成对覆盖：`menu_bar_has_no_send_menu`（只驱动菜单栏，
    断言顶层不含 `send_title` **且**仍含另外四项）与
    `send_area_header_keeps_the_send_settings`（驱动发送区表头那一行，断言格式/行尾/
    定时仍在）。
    两者都做过**注入验证**：加回「发送」菜单 / 把菜单栏砍到只剩「文件」/ 删掉行尾那一组 /
    整行不画，四种破坏各自都让对应测试**变红**。**但它们证明不了"少一项后好不好看"**
    ——只能人眼看。

68. **中央区顶部删掉「⚙ 设置」按钮（2026-09-24 加；结构已自动化验证，观感需人看）**：
    本条是 3.14 的目视项。请打开程序看中央区最上面那一行：
    - **右侧不应再有「⚙ 设置」按钮**：那一行现在只有左边一颗「☰ 侧边栏」开关，
      右侧是空的。若还能看到齿轮图标 + 「设置」字样，说明删得不干净；
    - **设置窗口仍能打开，且只能从顶部菜单栏打开**：点菜单栏的
      **设置 → 打开设置窗口**，应当照常弹出设置窗口（语言 / 字体档位 / 日志目录等都在）。
      **这一条务必亲手点一下**——那颗按钮删掉后，菜单栏是**唯一**入口，
      只"看到菜单在"不能证明它接得通；
    - **右侧留白是否可接受**：删掉按钮后那一行右半部分会空出来，
      看是否显得别扭、是否想把侧边栏开关挪过去（这属于主观取舍，请给意见）；
    - **切换侧边栏显隐**：左边那颗「☰ 侧边栏」开关应当照常把右侧边栏收起/展开。
    自动化部分由 2 条测试成对覆盖：`top_toolbar_has_no_settings_button`（只驱动那一行，
    断言按钮文案不出现**且**侧边栏开关仍在）与 `menu_bar_still_offers_settings`
    （只驱动菜单栏，断言「设置」菜单仍在）。
    两者都做过**注入验证**：加回按钮 / 那一行整个不画 / 删掉「设置」菜单 / 删掉「文本」菜单，
    四种破坏各自都让对应测试**变红**。**但它们证明不了"右侧留白好不好看"**——只能人眼看。

69. **「正在打开」这一档真的会出现吗（2026-09-24 加；本次③ 的核心目视项）**：
    自动化只能证明状态机的**取值**对（`open_transitions_through_the_opening_state`），
    证明不了**用户看得见**。请这样看：
    - **正常端口**：点「连接」。预期**几乎看不到**「正在打开」——端口打开极快，
      中间态一闪就过（或者压根没渲染到那一帧）。**这是对的**，不是缺陷；
      界面应当直接显示「已连接」，圆点是绿的。
    - **打开一个会被拒绝的端口**（被别的程序占有，或拔掉设备后留着的那个 COM 号）：
      预期先看到「**正在打开**」（状态栏短词 + 侧边栏「状态: 正在打开」，
      圆点**琥珀色**、不是绿也不是红），随后跳成「未连接」（红）+ 接收区一条
      「串口错误: 打开 COMx 失败: …」。**关键是：全程不该出现绿色「已连接」**。
      若闪了一下绿，说明 ③ 没修干净，请立刻回报。
    - **按钮文案**：在「正在打开」的那一刻，侧边栏按钮应当已经读作「**关闭串口**」
      （不是「打开串口」），旁边挂一句「正在打开…」。若按钮还是「打开串口」，
      说明文案与动作又反了。
    - **发送闸门**：在「正在打开」那一刻点「发送」，**不该**有任何字节出去、
      「发送字节数」**不该**增长、输入框**不该**被清空。这是本次修的核心行为。
    - **琥珀色的观感**：圆点颜色与绿/红是否区分得开、在浅色与深色主题下是否都看得清。
    自动化部分由 5 条测试覆盖（serial 三条 + app 两条），并做过 5 个注入验证
    （详见 §3.15）。**它们证明不了"用户看得见这一档"**——只能人眼看。

70. **自动重连退避与"停手提示"（2026-09-24 加；行为可用假设备验，观感需人看）**：
    - **退避**：让串口断开（拔线 / 关设备）后不手动重连，观察接收区：
      重试不该再是"每秒两次"的刷屏，而应当**一次比一次稀疏**（0.5s → 1s → 2s → 4s → 8s → 16s）。
      重试本身**不产生接收区文字**（旧实现也没有），所以判据是"界面没有变慢变卡"与
      "插回去之后能在合理时间（≤ 退避间隔）内自动连上"。
    - **停手**：连续失败 6 次后，接收区应当出现**恰好一条**
      「自动重连已停止（连续失败 6 次），请手动点「连接」重试」，
      此后**不再有任何重连尝试**（界面上看不出来，但 CPU 应当安静下来）。
      若这条提示**重复出现多次**，说明"只在跨线那一帧提示"没做对。
    - **恢复**：停手之后手动点「连接」并让它连上，之后**再**拔线，
      应当重新开始自动重连（计数已清零）。
    - **正常关闭不算失败**：TCP 下对端正常关闭走 `Closed`，**不该**被算进退避
      （这条只能在 TCP 上验，见下）。

71. **待发队列背压（2026-09-24 加）—— 已提供脚本，不必手点**：
    - **首选**：`python tools/backpressure_check.py --help`。**零硬件**——脚本在回环上监听，
      程序用「TCP 客户端」连上来，脚本按 **40 KB/s 慢读**把排空速度掐住。脚本自带内存采样
      与判定，收尾给出还需你目视的三条。三步：
      ① `--write-payload payload.txt` 生成载荷；② 启动程序；③ `--mode tcp --port 23232`。
    - **为什么"慢读"就能造出这个场景**：排空速度是有限的。**写侧现在是非阻塞的**
      （2026-09-24 从阻塞改过来，见 §3.15 ④ 与 §5 那一行），所以不管对端是慢读还是
      **一个字节都不读**，`send` 都不再会卡住 worker：内核缓冲满时立刻 `WouldBlock`，
      worker 让出 1 ms 再试，队列于是以「入队 − 排空」的速率涨到 1 MiB。
      脚本 `--self-test` 的第 3 项刻意用**阻塞** socket 把"旧行为会卡死"复现出来
      （实测灌满约 2.7 MB 内核缓冲后 2 分 36 秒无进展），用来解释当时为什么要改。
    - **判据（通过 = 五条全中）**：① 接收区**反复**出现「…: 待发队列已满（在队 … 字节，
      上限 1048576 字节），本条未发送」；② 界面**仍显示「已连接」**（绿点），不跳「未连接」；
      ③ 定时发送**没有被自动取消**（复选框还勾着）；④ 进程内存**不持续增长**
      （脚本的「不 OOM 判据」：提交量增长 ≤ 32 MB 且末 10 秒斜率 ≤ 1 MB/s）；
      ⑤ **状态栏「发送字节数」不再把被拒的包记上**（≈ 对端实际读到的量 + 在队的不超过 1 MiB），
      且「发送后清空」若开着，输入框在**被拒时不该被清空**（2026-09-24 修，见 §3.15 ①附）。
      ⚠️ 第 ⑤ 条是**改判据**：旧版 §4② 把"计数照涨"写成"已知缺陷、不属于本项判据"，
      现在它已经是缺陷本身被修掉了，所以反过来成为判据。
      ②③ 是刻意不判掉线的理由；④ 是"不 OOM"的核心。
    - **串口路线（可选）**：`--mode serial --serial-port COM7 --baud 115200`（需要一对虚拟
      串口，例如 com0com；或程序连另一端、脚本持这一端）。串口另有 `write` 的 10 ms 超时，
      两条路都通。**只开内存看门狗**：`--mode watch`（程序独占一个真实适配器时用）。
    - **本会话实测（用替身程序，不是被测程序）**：上限 1 MiB 时 → 试投 2803 包 = 接受 499 /
      **拒绝 2304 包**、在队峰值**正好 1048576 字节**、内存**零增长（Δ0.0 MB）**（判据 ✅ / 退出码 0）；
      把上限放开后 → **0 条**拒绝、在队峰值 **76.5 MB**、内存 **+57.5 MB、+3.14 MB·s⁻¹**
      （判据 ❌ / 退出码 1）。两个方向都跑过 ⇒ 判据既不空转、也不假红。
      记账侧也自洽：1 MiB 那档替身记账 **16,351,232** 字节 vs 对端实读 **16,318,464**（差 32,768 =
      还在内核里没被读走的那一片），不再有"记账虚高"。
      ⚠️ **替身已按新结构重写**（同日晚）：旧版把上限判在 worker 侧、UI 线程从不看队列满不满
      ——那正是刚修掉的旧结构。不重写的话，`--read-rate 0` 那一档下替身内存会一直涨，
      脚本会给出与真程序**相反**的判定。（重写后两个方向都复跑过，数字见上。）
      ⚠️ 更早那次还抓到**工装自己的一处假绿**：队列里 2048 条一度指向同一个 32 KB 对象
      （Python 的 `bytes` 会被复用），64 MB 队列只占几百字节内存，内存判据被骗成绿色。
      替身已改成每包新分配（真实程序每次 send 都是一条新的 `Vec<u8>`）。
    - **脚本自检**：`python tools/backpressure_check.py --self-test` 三项全 OK / 退出码 0，
      其中第 3 项刻意用**阻塞** socket 复现"旧行为会卡死"：完全不读时写入 **2,752,512** 字节后
      2.6 秒无进展（≈ 之前记的 2.75 MB 内核缓冲），慢读时最坏一次 **1.167 s** 仍返回
      （≈ 之前记的 1.168 s）。**改过脚本后必须重跑这一项。**
    - **手点配方（不想用脚本时）**：波特率 **1200**、文本模式粘 **2~8 KB**、勾定时发送、
      间隔 **10 ms**，几秒后开始报满（耗时由**入队速度**决定，包越大越快；排空上限就是波特率，
      所以波特率越低余量越大，115200 也行）。恢复：取消勾选，队列按波特率慢慢排空，
      想立刻清空就断开重连。
    - **"计数照涨"那个缺口已修**（2026-09-24）：原先判在后台线程、`write()` 只投 mpsc 就
      返回 `Ok`，于是状态栏照涨、输入框照清；现在判定挪到调用线程（`PendingLedger`），
      `write()` 当场返回 `Err`，`app.rs` 一行未改就同步了。实测差额原是"界面 78.7 MB vs
      真实入队 12.8 MB"，现在两者应当一致。见 §3.15 ①附。
    - 单元测试侧已覆盖：`worker_tests` **9 条**（账本 / 出队归还 / `submit` 拒绝且不投递）
      + 三个通道各 2 条（超限在调用点被拒 / 重连清账）+ TCP 的
      `close_returns_promptly_even_when_the_peer_never_reads`（对端不读也要能立刻断开）。
      非空转验证用 4 处注入跑过，**四处全部按预期变红**（见 §3.15 ④）。

72. **显示模式互斥与"不空白"（2026-09-24 加，用户目视发现后修）**：
    - **怎么做**：接收设置行（或菜单栏「视图 → 显示模式」）里有三个勾选框——
      「颜色」「HEX 接收」「字节查看」。**两两试一遍**，共 6 种勾法：
      ① 颜色 + 字节查看；② 字节查看 + 颜色（顺序反过来）；③ HEX + 字节查看；
      ④ 字节查看 + HEX；⑤ 颜色 + HEX；⑥ HEX + 颜色。
    - **判据（通过 = 三条全中）**：① **接收区永远不空白**——这是本项的核心，
      曾经「颜色 + 字节查看」同时勾上会让接收区一片空白（见 §3.16）；
      ② 后勾的那一项**留下**、与它冲突的那一项**自动取消**（"最后点的胜出"，
      不是"某一项永远赢"——那样会出现"点了没反应"）；③ 互斥只在**两两冲突**时发生：
      「HEX + 颜色」是合法组合，勾了第二个之后两个都应当仍然勾着。
    - **两个入口必须表现一致**：菜单栏那条与接收设置行那条走的是同一裁决，
      任一处出现"菜单能同时勾、主界面不能"就是回归。
    - **顺带验存档**：若手边有修复前留下的存档（`color_view` 与 `ascii_view` 都是 `true`），
      直接启动 → 应当显示**字节查看**（`ascii_view` 胜出、`color_view` 被关掉），
      接收区有内容而不是空白。
    - 自动化侧已覆盖：穷举 8 种开关组合的"不空白"回归测试 + 无头渲染断言
      "画出来的是当前模式那一份镜像"，共 6 条（见 §3.16）。**非空转验证 4 处注入全红。**

### ③ 仓库状态

`AI_comtool-rs` 的 `ai-refactor` 分支工作区是干净的。
`comtool-rs` 也是干净的（等于 HEAD）。

---

## 5. 已知缺陷与残余风险

| 严重度 | 问题 | 位置 | 说明 |
|---|---|---|---|
| ~~**高**~~ **已修** | ~~**接收缓冲无上限**~~ | `receive.rs` 的 `trim_receive_logs` | 已于 2026-09-21 修复：新增 `max_log_chars`（默认 30000，0 = 不限）FIFO 淘汰，四份镜像各自按同一字符预算裁剪，支持从句中切开 `ColoredSpan`。接收日志仍不参与持久化（退出时保存大日志只会变慢，无明确需求） |
| ~~**高**~~ **已修** | ~~**`connected` 假阳性**（串口）~~ | `app.rs::open_connection` / `serial_impl::channel` | **已于 2026-09-24 修复**：串口通道引入第三态 `worker::OPENING`——`SerialChannel::open()` 先置 `OPENING` 再 spawn，后台线程**真正打开端口之后**才升到 `ACTIVE`。新增 `ByteChannel::is_ready()`（默认转发 `is_active()`，只有串口覆盖），`ComTool` 据此新增 `opening()` 与 `channel_is_ready()`；`open_connection` 不再无条件置位，发送闸门（`send_data` / `send_preset` / `tick_repeat_send`）全部改看"通道就绪"而不是 `connected`。界面新增第三档「正在打开」（侧边栏 `status_opening` + 状态栏 `statusbar_opening`，琥珀色圆点）。成功提示延后到 `settle_connection_state()` 补（一次性记账位 `announced_open`）。见 §3.15 |
| ~~中~~ **已修** | ~~自动重连无退避~~ | `app.rs::auto_reconnect` | **已于 2026-09-24 修复**：新增 `reconnect_backoff(failures)` 纯函数（500 ms 起、指数翻倍、封顶 30 s）+ `reconnect_failures` 计数（`#[serde(skip)]`）+ `MAX_RECONNECT_ATTEMPTS = 6` 上限；到顶后彻底停手并在接收区留一条说明。**只对串口生效**（F4 起对 TCP / UDP 直接返回）。见 §3.15 |
| ~~中~~ **已修** | ~~待发队列无上限~~ | `communicationtrait::worker` | **已于 2026-09-24 修复**：新增共享上限 `MAX_PENDING_BYTES = 1 MiB` 与**两侧共享**的 `PendingLedger`（`Arc<AtomicUsize>`）；三个通道的 `write()` 统一走 `worker::submit`（先预约额度 → 再投递 → 失败退还），`worker_main` 只 `push_back`、不再自己判上限。满了**在调用线程当场**返回 `Err`（不判掉线、不停定时发送），而不是继续吃内存或静默丢弃。见 §3.15 ① 与 **①附**（初版是"后台线程判 + 报 `Notice`"，当天订正为"调用线程判"） |
| ~~低~~ **已修** | ~~**待发队列被拒时状态栏的发送字节数仍累加、输入框仍被清空**~~ | `communicationtrait::worker::submit` + 三个通道的 `write()` | **已于 2026-09-24 修复**：根因是"队列满"判在**后台线程**里做，而 `write()` 只把命令投进 mpsc 就返回 `Ok` ⇒ app 层照常 `tx_bytes += len`、`send_data` 照常清空输入框，用户看到「本条未发送」的提示与**仍在增长**的发送计数并存。改法是把这个判定**挪到调用线程**——新增两侧共享的 `PendingLedger`，`write()` 在**投递之前**就 `reserve(len)`，失败即当场返回 `Err`。**`app.rs` 一行未改**（它本来就是"`Ok` 才记账、`Ok` 才清空"），两个症状一起消失。旧的 `Notice` 通路退场，但文案逐字保留（差别只在"当场说"）。见 §3.15 ①附 |
| 中 | 写路径瞬态错误静默重试 | `channel.rs::worker_main` | `WouldBlock` / `Interrupted` 直接重试是合理的，但设备长期不接受数据时用户得不到提示。背压修好后已有"队列已满"的 `Notice` 兜底，但队列**未满而长期写不动**时仍无提示。建议侧边栏显示「待发字节数」 |
| ~~中~~ **已修** | ~~**TCP 写侧没有写超时 ⇒ 背压不生效，还可能卡住界面**~~ | `net_impl/tcp.rs::open` / `worker_main` | **已于 2026-09-24 修复，走的是「非阻塞写」那条路**：`open()` 把 `set_read_timeout` 换成 `stream.set_nonblocking(true)`（另加 `set_nodelay(true)`），worker 循环加了 `idle` 标记与显式的 `IDLE_SLEEP = 1 ms`（非阻塞 socket 上 `set_read_timeout` 无效——阻塞性是**每 fd** 的），`WouldBlock` 时让出时间片后下一轮再试。<br>**修掉的两条后果**：① 对端彻底停读时 worker 不再卡在 `write`，1 MiB 上限真正生效（`TcpChannel` 现在跑的是与串口同一套 `PendingLedger` 账本，队列满**在调用线程当场**返回 `Err`）；② `close()` 里那个 `join()` 不再无限等——实测对端完全不读时 `close()` 也能在毫秒级返回（`close_returns_promptly_even_when_the_peer_never_reads` 钉住，3 秒上限）。代价：读侧不能再靠 `set_read_timeout`，改成 1 ms 轮询（这就是 `IDLE_SLEEP` 的另一个用途，同时决定 `close()` 的最大响应延迟）。<br>⚠️ **为什么没走"加一行写超时"**（已实测排除）：Windows 上带 `SO_SNDTIMEO` 的阻塞 `send` 超时后，数据**可能已经进入内核缓冲并发出**，而调用方只拿到 `WSAETIMEDOUT`、拿不到已写字节数；worker 于是重发整片。实测（64 KB 一片、片内含片号、超时 10 ms、对端停读 2 秒）：同一片在接收端出现 **126** 遍（片 42 的词数 1032192 = 126×8192）。非阻塞写没有这个歧义——`Ok(n)` 就是**准确**的已写字节数。串口也没有（驱动不接受就一个字节都不排队），所以那里 `.timeout(10ms)` 同时落成 `WriteTotalTimeoutConstant` 是安全的。<br>**UDP 刻意不动**（同日的对照实测）：向一个从不 `recv` 的对端发 4000 × 60 KB（`SO_SNDBUF=4096`）→ **0 次失败**、单次 `send` 最坏 **0.1 ms**（TCP 对照组挂了 **2 分 05 秒**）⇒ UDP 没有"对端停读 ⇒ worker 卡住"这条路径。见 §3.15 ④ 与 `tools/backpressure_check.py` 的 `--self-test` |
| 低 | UDP 的 `send` 仍可能被本机网卡出口堵住 | `net_impl/udp.rs::write`（刻意保持阻塞） | 与 TCP 那条**不是同一个问题**：本机到对端的**网卡出口**被打满时（例如 100 Mbps 网卡上持续满速发大包），`send` 仍会阻塞。实测在回环上向从不 `recv` 的对端发 4000 × 60 KB 是 **0 次失败 / 最坏 0.1 ms**，所以没有"对端停读 ⇒ worker 卡住"这条常被踩到的路径，这是 UDP 不改非阻塞的依据。但要构造出出口饱和需要真实网卡与真实流量，**未实测**，因此保留为已知残余风险；真出现时应按 TCP 同样的方式改非阻塞（`WouldBlock` 对 UDP 是"本机队列满、报文未发出"，语义清晰、无重发歧义） |
| ~~低~~ **已清理** | ~~2 个未使用依赖~~ | `Cargo.toml` | 已于 P6 第 1 批移除：`tokio`（features = `["full"]`，零调用，明显拖慢编译）与 `egui-chinese-font`（零调用，中文字体已由 `main.rs::setup_fonts` 读系统字体实现）。`chrono` / `egui_plot` / `rfd` / `serde` / `serialport` 仍是活跃依赖 |
| ~~低~~ **已修** | ~~侧边栏连接按钮的**文案与动作是反的**~~ | `sidebar.rs` | **已于 2026-09-21（F4）修复**：原来「已连接」时按钮显示「打开串口」、点下去却执行关闭，从上游继承而来（`e2039ff:src/sidebar.rs:157` 即在）。F4 重写这段时一并改成同向，并按传输类型分叉文案。见 §3.8 |
| 低 | 8 条 clippy 风格警告 | `app.rs` / `receive.rs` | 2026-09-24 按 `cargo clippy --all-targets` **逐类现场重数**（此前这一行写的是 `derivable_impls`×2，与实测不符，已订正）：`manual_range_contains`×3（`manual RangeInclusive::contains implementation`）、`derivable_impls`、`unnecessary_map_or`、`redundant_borrow`、`wrong_self_convention`（`methods called from_* usually take no self`）、deref 冗余（`derefed type is same as origin`）各 1 条，共 **8 条**。全部位于**本任务未触及**的既有代码（P4 顺带修掉了 `send_data` 里那条 `collapsible_if`，因为该函数被重写）。`cargo clippy --fix` 可一键清掉，但那会改动范围之外的代码，因此留作一次专门的"清警告"提交（P6 收尾不再处理） |
| 低 | 记录中途改配置不切换文件 | `log_file.rs` | 记录过程中改「文件名模板」或「输出目录」不会切到新文件，下次开始记录才生效；界面上没有特别提示 |
| 低 | **图表不支持无换行的连续逗号流** | `chart.rs::push_bytes` | 通道语义（第 k 个字段 = 第 k 条曲线）要求「一行一组」；无换行时通道数只能靠猜，猜错就画错位。因此这类数据在超过 4096 字节后被**整行丢弃**并在界面计数。属于明确的设计取舍，不是待修 bug |
| 低 | 绘制抽样（4000 点/曲线）未在界面暴露 | `panels/chart.rs` | 样本上限（可配）与绘制抽样（固定 4000）是两层不同的限制，界面只显示了前者。用户设到 10 万时会看到"曲线比样本数少"，目前只能靠文档解释 |
| 低 | 图表 x 轴是组序号而非时间 | `chart.rs` | 串口数据没有可靠时间基准，因此看不出真实时间跨度或采样率。若将来要显示"每秒多少个样本"，需要在解析端额外维护时间戳 |
| 低 | 图表解析在面板隐藏时也照常进行 | `receive.rs::receive_data` | 刻意如此：打开面板时立刻能看到已有曲线，而不是"打开后才从零开始"。代价是即使用户从不看图，每次接收仍会做一次扫描与少量解析 |
| ~~低~~ **已修** | ~~「HEX 接收」与「字节查看」同时勾上时**字节查看视图是空的**~~ | `app.rs` 接收设置行 / `panels/menu_bar.rs` / `receive.rs` | **已于 2026-09-21 修复**：两者改为互斥，规则收敛在 `ComTool::resolve_display_mode()`（两个界面入口 + 启动修复都走它），`receive_data()` 里的落点判据保留作纵深防御。见 §3.6 |
| ~~低~~ **已修** | ~~「颜色」与「字节查看」同时勾选时，显示的是彩色视图~~ | `receive.rs::display_mode` / `app.rs` 的渲染分发 | **已于 2026-09-24 修复**（用户实机目视审查发现，见 §3.16）。<br>⚠️ **上一版把这一行判错了**：原写"不丢内容，只是被另一视图盖住"——**不对**。真实情况是数据只写进 `ascii_spans`，而彩色视图渲染的是**从来没人写过**的 `display_spans` ⇒ 接收区**一片空白**，用户看到的就是"日志内容丢了"。错因是**只读渲染分支、没核对镜像落点**。<br>根因：三处判定不同源——片段构造与镜像落点只看 `hex_view` / `ascii_view`，渲染分支却把 `color_view` 排在最前。修法：① 新增 `ComTool::display_mode()` 作为**唯一判定**，「片段构造 / 镜像落点 / 渲染分发」三处都读它（渲染分发另抽成 `render_receive_area()`，否则写在 `ui()` 里测不到）；② 把「颜色」纳入 `resolve_display_mode` 的互斥（与「字节查看」互斥，**最后被勾上的胜出**），启动修复一并覆盖 `ascii + color`（字节视图胜出）。「HEX + 颜色」是**合法**组合，行为保持不变 |
| 低 | 时间戳前缀在 HEX / 字节查看视图里是"注解"而不是收到的字节 | `receive.rs` | 刻意如此（三种视图一致 + 日志文件一致），靠方括号与灰色区分。要一份纯字节记录请关掉前缀（默认就是关闭）。实时日志文件同样会带上前缀 |

---

## 6. 后续开发计划

> **注意：现在有两份计划文档，基线不同、数字不可混用。**
>
> | 文档 | 基线 | 覆盖率口径 | 状态 |
> |---|---|---|---|
> | `FEATURE_GAP_AND_PLAN.md` | `SerialGUI-rs` v1.4.0（Rust + egui，GPLv3） | 约 **90%** | P0~P6 **已收口**（唯一明确不做的是版本检查） |
> | `FUTURE_ROADMAP.md` | `Neutree/COMTool`（Python + PyQt5） | 约 **49%**（26/53 项） | **新开的计划线**：F1~F3 已完成，F4 待做，插件架构明确不做 |
>
> 两者**没有血缘关系**——`Neutree/COMTool` 只是同名同类的另一个项目、规模大一个量级，
> 拿它当基线时覆盖率自然掉到一半，不代表项目退步。下面 §6.1 是收口的旧线，
> §6.2 是新的 F 线。

### 6.1 旧线（P0~P6，相对 `SerialGUI-rs`）

完整计划见 **`FEATURE_GAP_AND_PLAN.md`**（含逐项功能对照表、上游 33 项能力清单、合规分析）。
该文档里 §7 的阶段划分与当前进度的对应关系：

| 阶段 | 内容 | 状态 |
|---|---|---|
| **P0** 架构拆分 | `app.rs` 拆为模块 | ✅ **已完成** |
| **P1** 配置与持久化 | `AppSettings` + `ComTool` 派生 serde + `save()` | ✅ **已完成**（未做磁盘实测，见 §4②） |
| **P2** 菜单栏 + 设置面板 | 顶部面板：File(Info/Quit) / Settings / 主题切换 / Byte mode / 面板显隐互斥。把 `config.rs` 的持久化字段接上真正的设置 UI | ✅ **已完成**（见 §3.1；G 组中「默认文件名」「启动检查更新」两项只有设置项，等 P4/P6 消费） |
| **P3** 接收链路补齐 | `max_log_string_length` FIFO 淘汰（**三镜像同步淘汰，`ColoredSpan` 需支持部分切分**）；`auto_scroll` 开关；可选时间戳前缀 | ✅ **已完成**：FIFO 淘汰与 `auto_scroll` 在 P2 期间提前做完；**时间戳前缀见 §3.5** |
| **P4** 实时文件日志 | Start/Stop Log 开关 + 默认文件名 `LogFile_%Y-%m-%d_%H-%M-%S.txt`（chrono 现成）；**写入必须走独立线程 + `BufWriter`，绝不在 UI 线程 `write_all`** | ✅ **已完成**（见 §3.2；落在 `src/log_file.rs` 而非计划里的 `src/panels/file_log.rs`，另加了输出目录选择与同名防截断） |
| **P5** 实时图表 | 建议引 `egui_plot` 而非移植上游手绘代码；支持逗号分隔多通道；`stream_buffer` 处理跨包分片 | ✅ **已完成**（见 §3.3；落在 `src/chart.rs` + `src/panels/chart.rs`，多通道上限 16、绘制点封顶 4000、数据不足时断线而不补 0） |
| **P6** 收尾 | Info 弹窗；版本检查（**默认关闭或 3s 超时静默失败**，内网要能跳过）；应用图标；`[profile.release] opt-level = 2`；清理未使用依赖 | ✅ **除版本检查外全部完成**：Info 弹窗 ✅；依赖清理 ✅（移除 `tokio`、`egui-chinese-font`）；release profile ✅；应用图标 ✅（代码绘制、11 测试）；**版本检查：所有者决定不做**，`check_update_on_start` 与它的设置段落已一并删除（见 §3.5）。阶段内只剩 §5 里那 8 条既有 clippy 警告未清 |

**覆盖率参考**：改动前相对 `SerialGUI-rs` 约 40%（核心链路约 90%）；P0/P1 完成后全局约 45%；
P2 完成约 75%（可日常使用）；P4 完成约 80%；P5 完成约 88%（功能对齐上游）；
**P6 两批完成约 90%**（唯一明确不做的是版本检查）。

**计划里还剩什么**：P0~P6 的**功能项已全部落地或明确取消**。剩下的都不是功能开发：
① §4② 的磁盘持久化实机验证（**必须由所有者亲手跑**，AI 不运行本项目）；
② §4① 的 GPLv3 合规二选一（已降为最低优先级，仅对外分发前必须拍板）；
③ §5 里那 8 条既有 clippy 警告的清理（`cargo clippy --fix` 一键，建议单独一次提交）；
④ §5 里「颜色 + 字节查看同时勾选」的界面行为决策（后者被盖住；已不丢内容，属同类小问题）。

### 6.2 新线（F1~F5，相对 `Neutree/COMTool`）

见 **`FUTURE_ROADMAP.md`**（含逐项设计约束、验收标准、排除说明）。按性价比排序：

| 序 | 项 | 工期 | 状态 / 备注 |
|---|---|---|---|
| **F1** | 发送侧补齐（HEX 发送 / 行尾 / 定时发送 / 发送历史） | 1~2 天 | ✅ **已完成**（`2e28994`）。新增 `src/send.rs`（纯逻辑可单测）+ `FakeChannel` 测试替身；「发送文件」仍未做（需先有待发队列背压） |
| **F2** | 编码选择（GBK / GB18030） | 2~3 天 | ✅ **已完成**（`e2039ff`）。新增依赖 `encoding_rs`；走增量解码，未退化成 lossy |
| **F3** | 多语言真接线 | 2~3 天 | ✅ **已完成**。新增 `src/strings.rs`（中英同宏臂，漏翻译写不出来）+ 源码级守卫测试；接收区/日志内的提示行**不翻译**。**2026-09-24 扩到 15 种语言**（§3.10 第 1、2 档 10 种 + 第 3 档 `tr ru pl` 3 种，见 §3.10） |
| **F4** | TCP / UDP 客户端 | 3~5 天 | ✅ **已完成**。新增 `src/net_impl/{config,tcp,udp}.rs`（零新依赖，只用 `std::net`）+ `communicationtrait::worker` 共用脚手架；`ChannelEvent::Closed` 的 `#[allow(dead_code)]` 已消掉（TCP 真的会发）；`app.rs` 的 `serial` 字段改名 `channel`，连接生命周期改为三类型分叉 |
| **F5** | N1~N6 六项小额改进 | 2~3 天 | ✅ **已完成**。零新依赖。N1 RTS/DTR 手动控制（新增 `ChannelEvent::Notice` 承载"非致命失败"）、N2 常用数据（`SendPresets`，满了不淘汰）、N3 转义档（`SendFormat::Escape` + `parse_escapes`）、N4 发送字节计数（`Ok` 之后才累加）、N5 任意波特率（下拉旁 `DragValue` + `clamp_baud`）、N6 焦点触发重扫端口。逐项设计与 6 处偏离见 `FUTURE_ROADMAP.md` §13.6 |
| ~~⑤~~ | ~~插件架构（**扩展机制本身**）~~ | — | **明确不做**：与"单 binary、全部行为可单测"的形态互斥 |
| **D1~D3** | SSH / terminal 插件 / 协议脚本 | — | ⏸️ **待定开发（未排期）**：不是被否决而是时机未到，各自有明确的触发条件。SSH 复用 F4 的连接类型分叉框架；协议脚本建议先试"配置化协议预设表" |

**F 线已全部落地（F1~F5）**。~~另有 2 条**既有缺陷**仍留着（不在任何阶段内）：
`connected` 假阳性（只影响串口）；自动重连无退避~~ —— 这两条已于 **2026-09-24 连同待发
队列背压一并修完**（§3.15，测试 272 → 302），F4 当初"未动它"的自动重连也补上了。
F1 的「发送文件」仍未做，其前置条件**待发队列上限（背压）现已完成** ⇒ 可以立项开工了
（`GAP_ANALYSIS_VS_COMTOOL.md` §0 里"真正值得做 4 项"的第 4 项）。

**F5 之后 COMTool 差距线的三条主结论**：① 串口参数一整轴已经没有可补的格子；
② 接收侧 8 项全满且两项超出 COMTool；③ 剩下的缺口集中在"平台化"（TCP/UDP 服务端、
插件机制、SSH）与"发布工程"（打包 CI、自动更新）——**都不适合与串口助手本体挤在一起做**。

本文件未列出的新功能（例如图表按通道显示/隐藏、x 轴改成真实时间基准）仍不在任何计划内。

---

## 7. 硬性约定（改代码前务必遵守）

来自 `AGENTS.md` 与 `DEVELOPMENT_GUIDE.md`，这里只列最容易踩的：

0. **⛔ 未经项目所有者逐次批准，禁止运行本项目（`cargo run` / 直接跑 exe），禁止以任何方式
   占用或打断 COM3。** 开发阶段只用 `cargo fmt` / `cargo check --all-targets` / `cargo test`
   验证（230 条测试均不需要串口硬件）。需要实机运行时停下来请求批准，由所有者自行启动。
   完整条文在 `AGENTS.md` 顶部与 `DEVELOPMENT_GUIDE.md` §0。
   **该规则已配置机器级硬拦截**：PreToolUse 钩子 `~/.workbuddy/hooks/comtool-run-guard.sh`
   （注册于 `~/.workbuddy/settings.json` 与 `~/.codebuddy/settings.json`），命中即拒绝工具调用，
   fail-closed；自测 `python3 .workbuddy/hooks/test_guard.py`；限时批准由所有者本人执行
   `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（批准只放行"运行"，COM3 永远拒绝）。

1. **串口读写绝不能在 UI 线程。** 一切 IO 都在 `SerialChannel` 的后台线程；UI 只投递消息。
   停机靠向命令通道发 `Halt`，**不要改回共享标志位轮询**——那正是先前版本死锁风险的来源。
2. **不要用 `write_all` 发送。** 发送走 `VecDeque` 增量推进，理由见 §3 第 1 条。
3. **接收三镜像必须同步。** `receive_text` / `display_spans` / `ascii_text` / `ascii_spans`
   要一起清空、一起追加。唯一的复位入口是 `ComTool::clear_receive()`。
4. **新增 `ComTool` 字段必须做两件事**：① 在 `impl Default` 里初始化（漏了直接 E0061）；
   ② 决定持久化归属——用户偏好保持可序列化，运行时状态加 `#[serde(skip)]`
   **并**加进 `config::restore_runtime_state()` 的重建列表。
5. **一次性提示不要每帧追加。** 像「串口已打开/已关闭」应在事件发生时追加到 `display_spans`，
   而不是在渲染函数里按状态每帧写。
6. **三种接收视图共用 `SCROLL_AREA_ID`**，切换模式时滚动位置延续，`clear_receive()` 也用它复位。
7. **不要从 `SerialGUI-rs/` 抄代码。** 那是 GPLv3 的参考实现，已被 `.gitignore` 排除；
   它的存在只是为了对照功能。`SerialGUI-rs/` 内部有**独立的嵌套 git 仓库**，别在它里面提交东西。
8. **日志长度上限只有一处实现**：`trim_receive_logs()`，在 `receive_data()` 末尾无条件调用。
   四份镜像**各自**按同一字符预算裁剪（三种模式写入的镜像不同，彼此没有下标对应关系），
   必须按字符边界切分，且要能把边界上的 `ColoredSpan` 从中间切开并保留其样式。
9. **绘制顺序：菜单栏 → 侧边栏 → 中央区**（`CentralPanel` 必须最后；中央区内部是
   接收区 → 图表 → 发送区）。egui 面板按添加顺序抢占空间。面板显隐有"至少保留一个"
   的不变量（`visible_panel_count() >= 1`），改动时必须同时维护：`visible_panel_count()`、
   视图菜单勾选框、`restore_runtime_state` 兜底。**当前面板有四个**：
   `sidebar_open` / `show_receive_panel` / `show_chart_panel` / `show_send_panel`。
10. **主题偏好存自有枚举 `ThemeChoice`**，不要直接把 `egui::ThemePreference` 写进存档——
    外部枚举取值一旦变动就旧存档读不出来。
11. **实时文件日志的写盘只能在后台线程**（`log_file.rs` 的 `worker_main`），UI 线程不许
    出现对日志文件的 `write`。停机靠投 `Halt`，且 `Drop` 里必须用 **`try_send`**：队列
    满时 `send` 会阻塞，「停止记录」会把 UI 线程永久挂住。文件由 UI 线程 `File::create`
    打开后交给线程——这是刻意与 `open_connection` 的「乐观置位」不同的写法，别改回去。
12. **接收镜像的每一次追加都要同步投给日志**：走 `push_receive_note()`（同时写镜像与日志）
    或 `log_append()`（只写日志），不要在 `receive_data` 里直接 `self.receive_text.push_str()`
    绕开日志——那会让日志文件悄悄少一段数据。
13. **「记录中」的唯一事实来源是 `log.is_some()`**，不要再加一个 `bool logging` 字段；
    查询用 `is_logging()` / `log_path()` / `log_written_bytes()`。
14. **时间戳前缀是显示注解，不是数据**：它在片段层插入（`timestamped_spans()`，因为
    `receive_text` 由片段拼接而来，插两次迟早走偏），因此会进导出与文件日志，但
    **绝不进 `chart.push_bytes()`**。行首状态从 `receive_text` 尾部推导
    （`at_receive_line_start()`），**不要另设平行字段**；一批只取一次 `Local::now()`；
    没有内容（无**非空**片段）时一个前缀都不加。改动 `receive_data()` 的分支时，务必
    核对**镜像落点与分支一一对应**：HEX 优先于字节查看，落点是 `!hex_view && ascii_view`。
15. **「HEX 接收」与「字节查看」互斥，规则只有一处**：`ComTool::resolve_display_mode()`
    （`receive.rs`）。界面两个入口（接收设置行、视图菜单）与启动修复都调它——不要在某个
    界面上就地写 `if hex { ascii = false; }`，规则一分散就迟早出现"菜单里能同时勾上、
    主界面里不能"。参数是"本轮哪个刚被勾上"，**不要改成无条件让 HEX 胜出**：那样用户点
    「字节查看」会被当场撤销，表现为"点了没反应"。原因见 §3.6：两个同时为真时字节视图
    渲染的 `ascii_spans` 自始至终没人写过，接收区是**一片空白**。
16. **界面文案一律走 `self.language.strings()`，不许硬编码中文**（F3 起）。新增文案在
    `src/strings.rs` 的 `strings!` 宏里**同时写中英两列**；带 `{}` 的文案用
    `strings::fill(s.字段名, &[&实参])`，**不能**写 `format!(s.字段名, 实参)`
    （`format!` 的格式串必须是字面量）。`strings::tests::ui_sources_do_not_hardcode_chinese`
    会扫描 `sidebar.rs` 与 `panels/*`，硬编码中文直接测试失败。**接收区 / 日志里的程序提示行
    不翻译**——它们会写进实时日志与导出文件，随语言变化会让两份日志不可比。
17. **发送只有一条路**：按钮 / Enter / 定时发送都走 `ComTool::send_data()`。它清空输入框的
    前提是"**确实发出去了**"（`clear_send_after_send && !write_failed`）；解析失败与写失败
    都必须保留输入。发送侧新逻辑优先放进 `send.rs`（纯逻辑，可单测）。测发送链路用
    `communicationtrait::fake` 的替身通道，不要试图用真实串口。
18. **收发共用一档编码 `text_encoding`**（F2 起），解码**必须**走 `encoding_rs::Decoder`
    增量解码，不能"收到一批就 lossy 解一次"。GBK 与 GB18030 **共用解码器**（只有编码器不同）；
    UTF-16LE 在 `encoding_rs` 里**只支持解码**，小端字节要手写且不加 BOM。切档时丢弃上一批
    残留的半个字符（`AnsiDecoder::sync_encoding()` + `restore_runtime_state()` 两处）。
19. **建连接只有一条路**：UI 按钮与自动重连都走 `ComTool::open_connection()`（F4 起）。
    它按 `connection_kind` 建串口 / TCP / UDP，但 **`configure` → `open` → 接管事件通道
    这三步只有一处实现**——不要为某一种传输另写一遍（漏了事件通道 = "连上了收不到数据"，
    漏了 configure = "参数怎么改都没用"）。前置条件统一由 `connection_ready()` 判定，
    提示措辞由 `channel_descriptor()` 给；**前置条件不满足时什么都不做**，
    尤其不能乐观置位 `connected`。
20. **`channel` 与 `channel_kind` 同生共死**（F4 起）：通道对象的复用规则是
    "**同类型就地复用、换类型拆掉重建**"（`open_connection()`），两者必须一起清空
    （`teardown_channel()`）。这条规则同时服务两件事——串口断线重连是"同一个句柄再 `open`
    一次"，而 `app.rs` 里 20 多条发送链路测试靠的就是"注入的替身通道不被拆掉"。
    把复用改回"每次新建"，那些测试会静默地改成对着一个真串口跑。
21. **串口专属控件必须随连接类型分叉**（F4 起）：端口 / 波特率 / 数据位 / 停止位 / 校验 /
    流控在 TCP / UDP 模式下**整段不画**；连接类型单选在**连接中锁定**（否则界面显示的参数
    与真正打开的通道对不上，而按钮是「断开」，点下去断的是另一种传输）。
    **UDP 不许说"已连接"**——它没有连接，说"已连接"会让用户以为对端在线、以为掉线会被发现；
    状态词用「已绑定 / 未绑定」，按钮用「绑定 / 断开」。
22. **提示行里的名词要跟着传输类型走**：`ComTool::transport_noun()`（`串口` / `TCP` / `UDP`）。
    提示文本本身**不翻译**（见约定 16），但 TCP 掉线写一句"串口连接已关闭"是把日志发给别人
    看时的误导。
23. **网络侧的可忽略错误不要塞进公共函数**：`worker::is_transient` 只放三种所有传输共有的
    （`TimedOut` / `WouldBlock` / `Interrupted`）。某个传输特有的（Windows 上 connected UDP
    socket 把 ICMP port unreachable 报成 `ConnectionReset`）由该实现自己补——塞进公共函数
    会让串口那边也跟着忽略一个真故障。
24. **UDP 的 `Ok(0)` 不是 EOF**：零长度数据报是合法数据报。TCP 的 `Ok(0)` 才是对端关闭
    （发 `ChannelEvent::Closed`）。两者语义相反，改任一侧时别看错另一侧。

---

## 8. 环境与工作方式备注

- **`.opencode/`（61 MB）** 是 OpenCode AI 工具的配置与 node_modules，已随目录搬迁。
  `opencode.json` 配了三套角色：`executor`（改代码，默认）/ `planner`（只规划）/
  `expert`（攻难题），默认走 DeepSeek-V4-Flash。
- **`.gitignore` 忽略**：`SerialGUI-rs/`、`opencode.json`、`target/`、`echo_raw.txt`、
  `.workbuddy/`（**唯一例外：`hooks/test_guard.py`**）、`.opencode/agents/*.md`、`.opencode/prompts/*.md`。
  - `AGENTS.md` **原先也在此列，现已被移出**（2026-09-21）：它随仓库传播，不再只是本机文档。
    取消排除的原因很简单——它记的是"怎么改这个项目"的硬约定，而不是本机环境，放在仓库外
    等于换台机器就没人看得见。**同一批条文在 `DEVELOPMENT_GUIDE.md` §0/§6 与本文 §7 各有一份，
    三处仍需同步。**
  - `.workbuddy/hooks/test_guard.py` 也在同一天开了例外（守卫拦截规则的行为自测，39 条断言）。
    写法是四行而不是一条否定规则：`.workbuddy/*` + `!.workbuddy/hooks/` + `.workbuddy/hooks/*`
    + `!.workbuddy/hooks/test_guard.py`——**父目录被排除时无法单独重新包含其中的文件**，
    必须先否定父目录让 git 愿意进去。第三行把 `hooks/` 重新收窄成"只放行名单"，
    将来往该目录多丢文件时不会顺手一起进仓库。其余 `.workbuddy/` 内容（含 `memory/`）仍全部忽略。
  - ⚠️ 但**被测试的那个脚本本体仍不进仓库**：`comtool-run-guard.sh` 与批准脚本在
    `~/.workbuddy/hooks/`（用户级，不在本仓库内）。所以新克隆的仓库能读到规则、也能读到
    自测脚本，却**没有可测的对象**——要复现拦截必须另行取得这两个脚本。
    **文档与自测随仓库走，并不等于保护随仓库走。**
- **`echo_raw.txt` / `test_out.txt`**：根目录下的零散调试文件，已随搬迁复制，内容无关紧要。
- **持久化存档位置**：`%APPDATA%\ComTool-RS\data\`（eframe 按 `run_native` 的标题自动创建，
  由 `persistence` feature 启用）。
- **`core.autocrlf = true`**：仓库开启了 CRLF 自动转换。跨平台协作时注意行尾。

### 一个可复用的验证工具

上一个会话为了证明「拆分是纯移动、零行为变化」，写了一个脚本并对每个代码单元做
归一化比对（剔除注释/空行/缩进/属性行 + 显式改名映射 + 断言在目标文件中作为连续子序列出现）。

它已沉淀为技能，**下次做任何「拆分 / 搬动 / 提取方法」类重构都可以直接用**：

```
~/.workbuddy/skills/refactor-equivalence-proof/
├── SKILL.md                        方法说明、坑位、局限
└── scripts/verify_pure_move.py     通用版脚本（改顶部 CONFIG 区即可用）
```

---

## 9. 快速上手

```bash
# 1. 进项目
cd E:/Downloads/Zed/AI_comtool-rs

# 2. 确认分支与状态（应为 ai-refactor，工作区干净）
git branch --show-current && git status --short

# 3. 建立门禁基线（应为 0 warning / 302 passed / 1 ignored / fmt 干净）
cargo fmt && cargo check --all-targets && cargo test

# 4. 看清两个版本的差异
git diff master ai-refactor --stat

# 5. 读这三份文档再动手
#    AGENTS.md / DEVELOPMENT_GUIDE.md / REFACTOR_REPORT.md

# 6. 计划内的功能项已全部落地（P0~P6 与 F1~F5），已知缺陷也已在 2026-09-24 全部收口
#    （§5 现在只剩"低"档设计取舍，见 §3.15）。**下一件活是 §4② 的实机验证清单**
#    （72 项 + 34a/34b/57b，共 75 条；全项目从未在真机上跑过一次），都需所有者亲手跑。
#    之后是 §5 里那 8 条既有 clippy 警告的清理；GPLv3 合规已降为最低优先级（§4①）。
```

如果你要对照原版行为：`git -C E:\Downloads\Zed\comtool-rs log -1`（原版停在 `04dc007`），
或在本仓库 `git checkout master`。
