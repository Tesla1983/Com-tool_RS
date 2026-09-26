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
| GUI | `egui` / `eframe` **0.36.2**（`glow` 后端 + `persistence` feature） |
| 串口 | `serialport` **4.10.1**（manifest 写 `"4.7.2"` 是 caret 要求，实解析到 4.10.1） |
| 编码 | `encoding_rs` **0.8.42**（manifest 写 `"0.8.35"`，实解析到 0.8.42） |
| 其它 | `rfd`（文件对话框）、`serde`（持久化）、`chrono`（日志文件名与接收时间戳前缀的格式化，P4 起在用）、`egui_plot` 0.37（实时图表，P5 起在用）。`tokio` 与 `egui-chinese-font` 已作为死依赖移除（见 P6 第 1 批） |
| 版本策略 | **manifest 只写"最低要求"，实际版本由 `Cargo.lock` 钉住**，升级补丁版只动 lock、不动 manifest。2026-09-26 一次性升级四个补丁版（技术债报告第 3 项），详见 §3.25 |
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

338 条测试**都不需要串口硬件**，包括一条走完异步打开失败路径的集成测试、
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
| 新增依赖 | `egui_plot = "0.37.0"`（MIT OR Apache-2.0；依赖 `egui ^0.36`，与本项目 0.36.2 匹配——**注意它的版本号与 egui 并不同步**：0.36 依赖的是 `egui ^0.35`） |
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
需先做目视探针）、`ar`/`he`（**RTL 策略未决**，且 egui 0.36.2 有 GSUB/GPOS 整形但**无 bidi
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

### 3.17 增量（2026-09-24：修掉"浅色主题下状态栏没变色"）

**触发**：用户目视审查（本会话第 6 轮）——「UI效果错误，底部状态栏在浅色的情况下没变色」。

**根因不在配色表，而在状态栏原先根本没画背景。** 面板用的是 `egui::Frame::NONE`，
它的 `fill` 是 `Color32::TRANSPARENT`（已读 `egui-0.36.2/src/containers/frame.rs` 的常量定义
确认）。`Frame::paint` 无论如何都会 emit 一个 `RectShape`，所以"背景"这个矩形是存在的、
只是**透明的**——透出来的东西是 `eframe::App::clear_color` 的默认值：

```rust
// eframe-0.36.2/src/epi.rs:248
fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
    egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
    // _visuals.window_fill() would also be a natural choice
}
```

**参数名就叫 `_visuals`——它拿得到主题却不用**，返回恒定的深灰 (12,12,12)。
于是：深色主题下面板底色是 gray(27)，两者几乎一样 ⇒ **这个缺陷一直没暴露**；
浅色主题下面板底色是 gray(248)，这条带子还是 (12,12,12)，而浅色主题的文字色是
gray(80)（`widgets.noninteractive.fg_stroke`）——**深字压在近黑底上**，所以用户看到的是
"整条状态栏没变色、也像没内容"。

**这也是它躲过全部自动化测试的原因**：原有状态栏测试断言的是圆点数、文字内容、文字位置、
下留白——没有一条问"这块面板自己的背景是什么颜色"。而"没画背景"这个状态**测不出来**，
因为没画和"画了个透明的"在形状列表里长得一样。

**改法（两处，一主一兜底）**：

| 落点 | 改动 | 判据 |
|---|---|---|
| `status_bar.rs` 的 `.frame(...)` | `Frame::NONE` → **`Frame::side_top_panel(ui.style())`**，只覆盖内边距（沿用我们的 `BAR_PADDING_X/Y`） | 面板背景 = **该主题的 `panel_fill`**（浅 248 / 深 27）。用 `side_top_panel` 而不是手工 `.fill(...)`：它正是 egui 面板不给 frame 时自己会用的那一份（`Panel::resolve_frame`），底色的唯一来源留在 egui 那边 |
| `app.rs` 的 `impl eframe::App` 新增 `clear_color()` | 返回 `visuals.panel_fill.to_normalized_gamma_f32()` | **兜底**：以后任何没被面板盖住的区域都跟着主题，不会再露一条深色。`clear_color` 拿到的是 `global_style().visuals`（实测 eframe 调用点：`glow_integration.rs:694` / `wgpu_integration.rs:813`），所以它真的能读到主题 |

两条是**纵深防御而非重复**：主修让状态栏自己负责背景（症状消失），兜底让"没人画的区域"也不
再是写死的深色（同一类问题的下一个入口）。**注意 `side_top_panel` 在 0.36.2 里不带描边**
（源码只有 `inner_margin` + `fill`），所以没有因此多出一条边线；`show_separator_line(false)`
仍然生效，`Frame` 的 `inner_margin` 是**覆盖**不是叠加，状态栏高度不变——既有的"下留白"
与"整组居中"两条测试原样通过。

**新增测试 2 条**（308 → **310**）：

| 文件 | 测试 | 断言 |
|---|---|---|
| `status_bar.rs` | `status_bar_background_follows_the_theme` | 无头渲染两种主题，从形状列表里取出**横跨整幅宽、贴住屏幕底边**的那个矩形（即面板背景），断言它 == `Visuals::{light,dark}().panel_fill`。断言"等于该主题的面板底色"而不是"两色不同"：后者会被"换个写死的浅灰/深灰"骗过（注入 G2/G4 就是这两种写法） |
| `app.rs` | `window_clear_color_follows_the_theme` | `clear_color` 在浅/深两套 `Visuals` 下分别等于对应的 `panel_fill.to_normalized_gamma_f32()`，且两值不等 |

**非空转验证 4 处注入全红**（脚本 `.workbuddy/tmp_probe_theme_bg.py`，不随仓库走）：
G1 状态栏 frame 回到 `Frame::NONE`（→ 填充是 `#00000000`，这正是改动前的实测值）·
G2 背景写死成 gray(27)（深色下恰好正确、浅色下露馅）· G3 窗口底色退回 eframe 默认实现 ·
G4 窗口底色写死成 gray(248)（浅色下恰好正确、深色下露馅）。还原后 310 全绿。

**门禁（本增量结束）**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **310 passed / 1 ignored / 0 failed** · clippy **8 条既有**（0 新增）·
`cargo build --release` 后 exe 时间戳（18:41:17）**晚于**最后一次 `src/` 改动（18:40:30）。

---

### 3.18 增量（2026-09-24：「打开串口」按钮重做为自绘全宽按钮）

**触发**：用户目视审查——「把打开串口按钮的UI重新做一下，让它更像一个按钮，先出渲染，然后让我
定稿」。按用户要求分两阶段：**先出渲染稿（不动代码）→ 用户定稿 → 再实现**。定稿结果：形态
**C**（全宽实心 + 左状态点 + 右箭头）、未连接**绿**、已连接**红实心**。

**原实现为什么"不像按钮"**（`sidebar.rs` 的连接/断开那一段）：

```rust
let button = egui::Button::new(label).selected(is_connected);   // ← 就这一行
```

宽度、高度、填充、描边、圆角一个都没给，全吃 egui 默认值。实测（本机
`egui-0.36.2/src/style.rs`）：`button_padding` 只有 `vec2(4.0, 1.0)` ⇒ 高度≈文字行高、
宽度随文案长度；深色 `widgets.inactive` 是 `weak_bg_fill = gray(60)` 压在 `panel_fill`
`gray(27)` 上、`corner_radius = 2`、**无描边**；已连接走 `selection.bg_fill`（深色主题
`rgb(0,92,128)`）。结论：**问题不在配色难看，而在没有任何一处告诉用户"这里可以按"**。

**改法**：新增 `draw_connect_button()` 自绘（`ui.allocate_exact_size` + `Painter`），量全部显式
给死：全宽 × 高 32 × 圆角 6 × 内边距 12 × 状态点直径 9 × 三角箭头 5×9。右侧箭头**画几何图形
而不是 `▸` 字符**——符号字形要靠字体兜，`epaint` 只取矢量轮廓，字体没覆盖到就是一个方框
（接收区那条教训对界面文字同样成立）。自绘的三条理由写在函数文档里：① 要全宽固定高度
（文案走 15 种语言，德语/俄语远长于中文）；② 按钮内部要排三个元素，`Button` 只画居中文字；
③ 底色按*动作语义*取，用 `Button::fill` 得在每个调用点重算主题与悬停态。

**配色与语义——本增量最需要记住的一条**：按钮底色报的是「**按下去会做什么**」，与底部状态栏
圆点的「**现在处于什么状态**」**同色不同义**：

| 动作档（`ConnectButtonTone`） | 底色 | 含义 |
|---|---|---|
| `Idle` | 绿 `#1A7F4B`（浅色主题 `#17794A`） | "按下去会连上" |
| `Opening` | 琥珀 `#F5A623`（**两套主题共用，就是状态栏那个琥珀**） | "正在进行" |
| `Connected` | 红 `#B3261E`（两套主题共用） | "按下去会断开"（准破坏动作） |

琥珀刻意与 `status_bar::ConnectionIndicator::pending` **取同一个值**，并有测试钉住相等——同一个
含义在项目里只能有一处颜色定义。悬停更亮、按下更暗，与 egui 默认控件同向（深色
`inactive 60 → hovered 70 → active 55`），所以手感不与旁边的下拉框、单选钮打架。

**按钮是四档状态，不是两档**：断开 / 悬停-按下 / **「正在打开」**（`ComTool::opening()`：串口的
`open()` 只拉起后台线程，通道尚未就绪）/ 已连接。

> ⚠️ **「正在打开」时按钮文案仍是「关闭串口」，不是「正在打开」。** 那时 `is_connected` 也为真、
> 按下去确实是关闭 ⇒ 显示"打开串口"就是 F4 修过的"文案与动作相反"。那一态要说的是"正在进行"，
> 由**琥珀底色 + 状态点 + 下方小字**交代。定稿渲染稿里我把这一态画成了"正在打开"四个字，
> **是错的**，实现时按上面这条纠正。

**一处必然的布局后果**：按钮改全宽后，原先与它同行的那两条提示（`opening_hint`「正在打开…」、
`need_port_to_connect`「请先选择端口」）**放不进同一行了**（同行只剩几个字宽），已移到按钮
**下方**单独一行。这不是可选项，是"让它更像按钮"的代价。

**判据落在哪**（新增测试 7 条，310 → **317**）：

| 测试 | 断言 |
|---|---|
| `connect_button_tone_covers_all_three_states` | `(false,*)→Idle`、`(true,true)→Opening`、`(true,false)→Connected` |
| `connect_button_label_stays_on_the_close_side_while_opening` | 「正在打开」时文案 == `close_port_button`（**F4 那类缺陷的守卫**），外加其余五格 |
| `connect_button_fill_follows_the_action_not_the_status` | **通道大小**方向断言（未连接 G 最大 / 已连接 R 最大）+ 悬停更亮 + 按下更暗 + 按下与悬停同时为真时以按下为准。**只断言"两色不同"会被反色骗过** |
| `connect_button_opening_tone_reuses_the_status_bar_amber` | 琥珀 == 状态栏 `pending` 的 `color()`（两套主题都断言） |
| `connect_button_paints_a_full_width_background_in_the_sidebar` | 侧边栏这一帧里存在"高 32 且宽 ≥ 240"的填充矩形，且底色 == Idle 档 |
| `connect_button_hint_sits_below_the_button` | 提示文字的 `pos.y >= 按钮.max.y` |
| `connect_button_dot_is_hollow_only_before_connecting` | 未连接：圆 fill 透明 + 描边 1.5 + 描边色 == 文案色；另两档实心且墨色正确 |

**测试踩的三个坑（都留在了注释里）**：

1. **尺寸指纹会撞车**：按钮靠"高 32"认出来，但侧边栏右上角那个 `>>` 收起按钮
   （`min_size(vec2(32.0, 32.0))`）**也是 32 高**——第一版就抓错了它（报"按钮宽 32"）。
   判据改成"高 32 **且**宽 ≥ 240"，两条缺一不可。
2. **两次绘制不能跨帧比位置**：按钮矩形与提示文字的位置要互相比对，若分两次 `run_ui` 取，
   比较的就不是同一帧（主题/画布/绕行任何一处不同都会差几个点）。已合并为一个
   `painted_frame()`，一次绘制同时取回矩形与文字。
3. **`ComTool::default()` 默认已经选好 COM1** ⇒ `connection_ready()` 为真 ⇒
   「请先选择端口」这条提示根本不画。断言前必须 `app.selected_port.clear()`，否则测的是
   "一个不存在的东西"。

**非空转验证 7 处注入全红**（临时改动，逐一还原）：

| 注入 | 改什么 | 红了哪条 |
|---|---|---|
| G1 | 删掉 `painter.rect_filled` | 全宽判据 + 状态点判据，均 panic"没画背景矩形" |
| G2 | 按钮宽度限到 130 | 全宽判据（日志里能看到 `[729,250]-[859,282]` 这个 130×32 的矩形） |
| G3 | `connect_dot_is_hollow` 恒 false | 空心判据（读到 `#FF_FF_FF_FF`，应透明） |
| G4 | `connect_button_label` 把"已发起"当未连接 | 文案同向判据（复现 F4："打开串口" vs "关闭串口"） |
| G5 | 绿红对调 | **方向断言**红（`#B3_26_1E` 不偏绿） |
| G6 | 琥珀 `#F5A623` → `#F5A624` | 同源断言精确到 1 位色值 |
| G7 | 提示画到按钮**上方** | 位置判据（提示 y=250，按钮底边 y=300） |

还原后 **317 全绿**。

**门禁（本增量结束）**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **317 passed / 1 ignored / 0 failed** · clippy **8 条既有**（0 新增）。
**`cargo build --release` 本次未成功**：`failed to remove target/release/comtool-rs.exe:
拒绝访问 (os error 5)`——该 exe 当时正在运行（PID 23852），文件被占用。exe 时间戳仍是
**18:41:17**，**早于** `src/sidebar.rs` 的 **19:07:17** ⇒ **约定 49 未满足**，需要关闭程序后
重跑一次。详见 §4② 第 74 项。

**i18n 未动**：按钮文案仍取 `strings.rs` 原有条目（`open_port_button` / `close_port_button` /
`connect_button` / `disconnect_button` / `bind_button`），15 种语言的字表**一个字都没改**，
条目数不变。自绘文字放不下时按比例缩到 11px（**不截断**——截断后看不出按钮是干什么的）。

> ⚠️ **本节有三处已被 §3.19 修订，读到这里请接着往下看**：① "全宽"→ 改成贴合内容宽；
> ② 文字下限 11px → 14px；③ 本节末尾那次"`release` 未成功"是程序正在运行所致，
> 同日 19:13 已重跑成功。

### 3.19 增量（2026-09-24：按用户实机目视反馈修订连接按钮 —— 收窄 + 放大字号）

**触发**：用户第一轮目视后反馈两句——「打开窗口这四个字太小了，把字放大」「按钮不需要那么长
贴到右边框」。（用户口述的"打开窗口"就是按钮文案「打开串口」，四个字。）这两条都是**观感**
问题，不是功能缺陷，所以改的是量而不是结构。

**改了五个量，全在 `sidebar.rs`**：

| 项 | 原值 | 新值 | 为什么 |
|---|---|---|---|
| `CONNECT_TEXT_SIZE` | 14.0 | **16.0** | 用户觉得字小 |
| `CONNECT_TEXT_MIN_SIZE` | 11.0 | **14.0** | 跟着基准上调；下限取"**不低于正文**" |
| 按钮宽度 | `ui.available_width()`（全宽 ≈ 264） | **内容宽**，夹在 `[130, 可用宽 − 24]` | 用户觉得太长、贴右边框 |
| 新增 `CONNECT_BUTTON_MIN_WIDTH` | — | 130.0 | 极短文案（UDP 的「绑定」）兜底，否则算出来不到 90，像标签 |
| 新增 `CONNECT_BUTTON_RIGHT_MARGIN` | — | 24.0 | 长译文下保证右边缘不贴面板边界 |

**⚠️ 一个被实测纠正的事实（原注释写错了）**：`CONNECT_TEXT_SIZE` 的旧注释说"与 egui 正文档
（`TextStyle::Body`，14.0）一致"——**这是错的**。egui 0.36.2 的 `TextStyle::Body` 实测是
**13.0**（运行期打印 `TextStyle::Body.resolve(ui.style()).size` 得到，不是凭记忆），所以原来的
14 只是"比正文大 1 点"，放进实心色块里肉眼几乎无差别——用户的判断是对的。注释已更正。

**宽度怎么算**（`draw_connect_button` 里，三步顺序不能换）：

```rust
// ① 先按基准字号量文案。量文本用的 painter 只在块内活着——出了块才 allocate_exact_size，
//    因为两者都要借 ui，同时借会冲突
let natural_text = {
    let painter = ui.painter();
    painter.layout_no_wrap(label.to_owned(), FontId::proportional(CONNECT_TEXT_SIZE), ink)
        .size().x
};
// ② 内容宽 = 左右留白 + 状态点 + 点与文案间距 + 文案 + 文案与箭头间距 + 箭头
let content_width = CONNECT_BUTTON_PADDING_X * 2.0 + CONNECT_DOT_DIAMETER + CONNECT_DOT_GAP
    + natural_text + CONNECT_TEXT_ARROW_GAP + ARROW_WIDTH;
// ③ 上限也夹一次最小宽度：侧边栏拖到最窄（200）时 available_width() 本身就小，
//    不夹的话下面 clamp 的上下界会颠倒
let width_ceiling = (ui.available_width() - CONNECT_BUTTON_RIGHT_MARGIN)
    .max(CONNECT_BUTTON_MIN_WIDTH);
let width = content_width.max(CONNECT_BUTTON_MIN_WIDTH).min(width_ceiling);
```

中文「打开串口」在 16px 下 `content_width ≈ 130`，正好压在下限；德语长译文会撞上限
（侧边栏默认 280 时上限是 256），此时按比例缩字号，缩到 **14** 为止。实测：德语文案
`"Seriellen Anschluss dauerhaft schliessen"` → 按钮宽 256、字号 11.31 → 抬到 14。

**判据落在哪**（新增测试 2 条，317 → **319**）：

| 测试 | 断言 |
|---|---|
| `connect_button_label_is_larger_than_the_body_default` | 文案字号 **≥ 16.0（写死的字面量）**；报错消息里带上正文的实测值 |
| `connect_button_keeps_a_right_margin_when_the_label_is_long` | 长德语文案下：按钮宽 ≤ 画布 − 24；字号**确实被缩小了**（< 16）且 **≥ 14** |

宽度指纹那条（`connect_button_paints_its_own_background_inside_the_sidebar`，原
`..._full_width_...`）也从"宽 ≥ 240"改成"宽落在 **[130, 200]**"——**上界是新增的**：
只留下界的话，"哪天又变回全宽"不会被发现。判据常量改名 `BUTTON_WIDTH_FLOOR` /
`BUTTON_WIDTH_CEILING`，与实现常量分开、用字面量写死。

**⭐⭐ 本轮空转验证抓出的两个真问题（比测试本身更值得记）**：

1. **判据引用实现常量 = 自证**。两条新测试的第一版分别写成
   `font_size >= CONNECT_TEXT_MIN_SIZE - 0.01` 和 `font_size > body_size`。注入验证时：
   - 把 `CONNECT_TEXT_MIN_SIZE` 压到 6 → 断言**照样绿**（实测字号 11.31 > 6）；
   - 把 `CONNECT_TEXT_SIZE` 改回 14 → 断言**照样绿**（14 > 正文的 13 成立）。

   两个阈值全部改成**字面量**后才在注入下变红。这与 §3.18 的 G2/G3 是同一个病：
   **判据的松紧必须对着"用户会不会不满意"定，不能对着"是否大于某个默认值"。**
2. **"大于正文"这条判据本身就是错的**——它逼出了上一条里 `14 > 13` 的假绿，也顺带暴露了
   `Body` 实测 13.0 这个事实。第一版判据的措辞（"必须比正文更大"）听起来合理，实际放走的
   正是用户抱怨的那个字号。

**非空转验证 5 处注入全红**（逐一还原，`grep -rn "INJECT-I" src/` 确认零残留）：

| 注入 | 改什么 | 红了哪条 |
|---|---|---|
| I1 | 宽度改回 `ui.available_width()` | 宽度指纹（日志里按钮矩形 `[729,250]-[995,282]`，宽 266 = 全宽） |
| I2 | 去掉右边距（上限 = 可用宽） | 长文案右边缘（按钮宽 280 = 顶到画布边） |
| I3 | `CONNECT_TEXT_SIZE` 改回 14 | 字号下限（报"字号 14，小于下限 16（正文才 13）"） |
| I4 | `CONNECT_TEXT_MIN_SIZE` 压到 6 | 长文案字号下限（报"缩到了 11.31，低于下限 14"） |
| I5 | `CONNECT_BUTTON_MIN_WIDTH` 压到 60 | 宽度指纹（按钮矩形宽 120.8，低于下界 130） |

**门禁（本增量结束）**：`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **319 passed / 1 ignored / 0 failed** · clippy **8 条既有**（0 新增）·
`cargo build --release` **成功**，exe 时间戳 **19:36:00** 晚于 `src/sidebar.rs` 的 **19:34:58**
⇒ **约定 49 满足**。

**i18n 仍未动**：文案条目与 15 种语言字表一个字没改，条目数不变。

---

### 3.20 增量（2026-09-24：全界面按钮统一成「打开串口」那一套外观）

**用户原话**：「把 GUI 界面上的所有按钮都按照打开串口的样式重新写一遍，注意和其它组件的
边距，不允许覆盖到其它无关的组件。」

#### 定稿（三问三答，均由用户在 `AskUserQuestion` 里选定）

| 问题 | 决定 |
|---|---|
| 改动范围 | **只改界面命令按钮（16 处）**；菜单栏下拉项与接收区右键菜单**保持菜单样式** |
| 配色分档 | **两档：主操作绿 + 其余中性灰**（不做破坏性红） |
| 按钮尺寸 | **分两号：独行 28/15、窄行与下拉框同行 24/14** |

#### 改了什么

**新增 `src/button.rs`**（全界面唯一的按钮绘制入口）：

- `ButtonTone::{Primary, Neutral}` × `ButtonSize::{Normal, Compact}` 两个正交枚举；
- `button_fill(tone, dark, hovered, pressed, enabled)` / `button_text_color(tone, dark, enabled)`
  两组纯函数（有单测），底色**报"按下去是主操作还是普通操作"**；
- `button(ui, label, tone, size)` 一个绘制函数 + `standard` / `primary` / `compact`
  三个便捷包装。

**替换的 16 处调用点**（原先是 `ui.button(..)` 或 `Button::new(..)`，即 egui 默认的
"灰标签"外观）：

| 文件 | 处 | 档位 |
|---|---|---|
| `sidebar.rs` | `>>` 收起 · 清空日志 · 导出日志 · 选择目录 | 标准 / 中性灰 |
| `sidebar.rs` | 刷新（与端口下拉同行） | **紧凑** / 中性灰 |
| `sidebar.rs` | 开始/停止记录 | 标准；**未记录时 `Primary` 绿，记录中退回中性灰** |
| `app.rs` | 发送 | 标准 / `Primary` 绿 |
| `app.rs` | 清空发送框 · 预设按钮 ×N | 标准 / 中性灰 |
| `app.rs` | 保存常用数据（可禁用） | 标准 / `Primary` 绿 |
| `panels/chart.rs` | 清空图表 | 标准 / 中性灰 |
| `panels/settings.rs` | 清空历史 · 清空预设 · 清空图表 · 选择目录 · 恢复默认 | 标准 / 中性灰 |
| `panels/menu_bar.rs` | 发送历史（历史为空时的禁用态占位） | 标准 / 中性灰 |

**刻意没改的 6 处**（都在弹出菜单里，宽度由菜单决定、整行高亮是菜单自己的选中语言，
套上实心块会把菜单撑宽并与其整体风格打架）：菜单栏「文件 → 信息 / 退出」「设置 → 打开
设置窗口」、发送历史下拉的每条历史 + 清空历史、接收区右键的「复制全部日志」。

#### 两处关键实现（都是为了"不覆盖其它组件"）

1. **宽度先按内容算，上限只能是 `ui.available_width()`**。
   上限**不**取 `max(available, 下限)`——那样在窄容器里会算出超过可用宽度的值，正好违反
   用户这次的要求。宁可按钮被挤窄。收窄时按比例缩字号，下限 13（标准档）/ 12（紧凑档），
   **不截断**。
2. **文字用 `painter.with_clip_rect(rect)` 画**。万一容器窄到"缩到最小字号仍放不下"，
   也只让字被裁在按钮内，不会跑出去压住旁边的控件。正常宽度下裁剪不起作用。

#### 两档配色（与「打开串口」同一套语言）

| 档 | 深色主题（常态/悬停/按下） | 浅色主题 |
|---|---|---|
| 主操作 `Primary` | `#1A7F4B` / `#21905A` / `#14663C` | `#17794A` / `#1E8F58` / `#0F5C38` |
| 普通 `Neutral` | `#3A3A3A` / `#474747` / `#303030` | `#DCDCDC` / `#E8E8E8` / `#C8C8C8` |
| 禁用（两种档共用） | `#2E2E2E` + 字 `#6E6E6E` | `#EDEDED` + 字 `#A0A0A0` |

绿的三个值**直接取自连接按钮那一档**：两者说的是同一个意思（"按下去会发起"），
各写一个近似绿迟早漂移成两种绿。

**禁用态不是把常态乘一个透明度系数**：那样在浅色主题下会得到一块"更浅的灰"，反而比
周围更扎眼。给的是两个固定值，与各自主题的面板底色只差一点点——判据是"到面板底色的
距离更小"，不是"和常态不一样"（后者会被"改成任意另一个亮色"骗过）。

#### 布局后果（已实测，非推测）

行高从 egui 默认按钮的 **18** 变成 **28**（标准档）/ **24**（紧凑档）。实测对照
（来自接入测试 panic 时打出的形状列表）：

| | 矩形 | 高度 |
|---|---|---|
| egui 默认按钮（注入还原后） | `[[798.5,425] - [859.9,443]]` 底色 `#3C3C3C` | **18** |
| 统一外观按钮 | `[[920,10] - [992,38]]` 底色 `#3A3A3A` | **28**（宽 72） |

- 侧边栏「端口 / 波特率」那两行的行高不变——「刷新」走的是紧凑档 24，比旁边的下拉框
  （实测约 18）高 6 点，整行被顶高 6；这是把按钮调到可读字号的必然代价。
- 设置窗口更长，但它本来就有纵向滚动区，不新增溢出风险。
- 「发送」按钮所在行的其余控件（定时发送勾选、间隔、发送历史）不受影响——按钮宽度贴合
  文案（「发送」两个字 ⇒ 72），远小于同行剩余宽度。

#### 测试（9 条新增，总数 319 → 328）

`button.rs` 8 条：三态亮暗方向（2 档 × 2 主题）· 主操作偏绿 / 普通档中性 · 禁用态更贴近
面板底色 · 两档高度（**字面量 28 / 24**）· 两档字号（**字面量 15 / 14**，且大于正文）·
宽度贴合文案且不吃满整行 · **宽度不超可用宽度** · **同排两个按钮不重叠且留缝 ≥ 4 点**。

`sidebar.rs` 1 条：**接入测试**——断言侧边栏这一帧里真的出现了 ≥ 3 个「高 28 + 中性灰实心
底」的矩形（`>>` 收起 / 清空日志 / 导出日志）。这条与纯函数测试分开写，防的正是"函数写得
对、界面没调用"。

**非空转验证 7 处注入全红**（逐一还原，`grep -rn "INJECT-J" src/` 确认零残留）：

| 注入 | 改什么 | 红了哪条 |
|---|---|---|
| J1 | 标准档高度改 30 | 两档高度 · 并排不重叠 · 侧边栏接入测试（报"只有 0 个"） |
| J2 | 悬停与按下的亮暗对调 | 三态方向（报"悬停没有比常态更亮"） |
| J3 | `Primary` 的绿改成中性灰 | 主操作偏绿（报"不偏绿：#DC_DC_DC_FF"） |
| J4 | 标准档字号改 12 | 两档字号（报"标准档字号是 12，应当是 15"） |
| J5 | 去掉宽度上限（`width = wanted`） | 宽度不超可用（报"按钮宽 114.375 超出了可用宽度 90"） |
| J6 | 宽度改成 `ui.available_width()`（全宽） | 宽度贴合文案 · 宽度不超可用 |
| J7 | 侧边栏两处改回 `ui.button` | 接入测试（报"只有 1 个"，只剩 `>>`） |

#### 门禁（本增量结束）

`cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** ·
`cargo test` **328 passed / 1 ignored / 0 failed** · clippy **8 条既有**（全部在
`receive.rs` / `send.rs` / `app.rs`，`button.rs` 与本次改动的文件**零新增**）。

**i18n 仍未动**：文案条目与 15 种语言字表一个字没改，条目数不变。新按钮直接复用既有的
`s.*` 文案，没有新增任何条目。

---


### 3.21 增量（2026-09-24：实时图表的高度 —— 实测 + 修掉"高度写死导致越界"）

**用户原话**：「你测试一下实时图表的窗口高度这么小，能够正常显示数据吗？」

#### 先量后答（实测，不是估算）

用无头渲染把图表画一遍，从形状列表里读**真实矩形**（`panels/chart.rs::tests::paint()`）：

| 量的是什么 | 实测值 |
|---|---|
| 图表块整体（标题行 + 绘图区） | **251 点**（4 通道 × 60 点，宽 900） |
| 绘图区（扣掉坐标轴后真正画曲线的矩形） | **886 × 206 点** |
| 曲线包围盒 y | **40.4 – 227.6**（绘图区是 31 – 237，上下各留 ~10 点余量） |
| 每条曲线的点数 | **60**（这个数据量下不发生抽样） |
| 16 通道时 | 绘图区尺寸**一模一样**（886 × 206），曲线 16 条、960 点 |

**结论：220 点够用。** 留给数据的净面积是 206 点高，自动缩放会把曲线铺满其中约 187 点
（**90%**）。"这么小能不能显示数据"这一问，实测答案是可以。

#### 但实测同时暴露一个真缺陷：高度写死 ⇒ 不会缩

`Plot::height(220.0)` 在 egui_plot 里落成 `min_size.y = 220` **且** `height = Some(220)`，
而 `calculate_widget_complete_rect`（`egui_plot-0.37.0/src/plot.rs:814-853`）对 `Option`
取 `unwrap().at_least(min_size.y)` ⇒ **无论可用空间多少，矩形恒为 220 点**：

| 中央区可用高 | 图表块占用 | 越界 |
|---|---|---|
| 480 / 260 | 251 | 0（放得下） |
| 220 | 251 | **31** |
| 120 | 251 | **131** |

中央区**没有滚动条**，越界部分被面板裁掉 ⇒ 夹在图表**下面**的发送区整块落到可视区之外。
中央区实测预算（工具栏 18 → 接收区 ≤400 → 图表 251）：**要 693 点才够把图表的底画出来**，
再算上发送区约需 **900 点**。

**修法**：`plot_height(available) = available.clamp(90, 220)`，且在**画完标题行之后**取
`ui.available_height()`（比预先减一个常数准）。**空间充足时与从前逐点一致**（仍 220，本次
没改变正常窗口下的观感）。下限取 90：再矮就只剩一条看不见的曲线，此时宁可让下面的内容被挤出。

#### 新增 5 条测试 + 4 处注入

| 测试 | 判据（一律字面量，不引用实现常量） |
|---|---|
| `plot_height_keeps_its_documented_value_when_there_is_room` | 251 / 400 / 1000 → **220**；`INFINITY` → 220（不产出 NaN） |
| `plot_height_shrinks_to_fit_and_stops_at_the_floor` | 150→150、90→90、50 / 0 / −30 → **90** |
| `chart_never_claims_more_than_the_room_it_has` | 画布 180 → 占用 ≤ 180 **且**没有留空白（改前占 251） |
| `shrunk_chart_still_draws_the_curves_inside_the_plot_area` | 缩过之后 4 条曲线仍在、每点都落在绘图区内、曲线高度 ≥ 绘图区 **70%** |
| `roomy_container_gets_the_same_plot_area_as_before` | 画布 480 → 绘图区高 ≥ 200、宽 ≥ 800（改前 206 × 886） |

| # | 注入 | 结果 |
|---|---|---|
| J1 | 高度改回写死 220 | `chart_never_claims_more…` 红：**越界 71.0 点** |
| J3 | 下限 90 → 60 | `plot_height_shrinks…` 红 |
| J4 | 只画坐标轴、不画曲线 | `shrunk_chart_still_draws…` 红："四条通道应各有一条曲线" |
| J6 | `include_y(1e6)` 把 y 范围拉大 | `shrunk_chart_still_draws…` 红：曲线只占绘图区高度的 **0%** |

全部还原，`grep -rn "INJECT-J" src/` **零残留**。

**一处负面结果（记下来，别当成已覆盖）**：注入"关掉 `auto_bounds`"（`&& false`）**没有**让
任何测试变红——首帧本来就会自动定界。所以"曲线落在绘图区内"这条断言**不等于**"自动缩放开着"，
它拦的是"画到别的矩形里去 / 变换退化"；真正盯住自动缩放的是 J6 那条（曲线必须用掉绘图区
≥70% 的竖直高度）。

#### 没动的一处（已知，属布局结构变更，见 §4② 第 76 项与 §5）

接收区的滚动区是 `.max_height(400.0)` + `auto_shrink([false, false])`（`receive.rs`），
**窗口不够高时它会把可用高度吃光**（实测：中央区 420 → 接收区底正好也是 420），此时图表只能
拿到 ≈0 点，`plot_height` 的 90 点下限仍会越界。要把"短窗口下发送区也可见"彻底解决，得在
**让接收区让出空间**与**整个中央区可滚动**之间选一个——那是布局结构变更，等用户拍板。

> **用户的决定（2026-09-24）**：在"接收区让位 / 中央区可滚动 / 先不动"三选一中**选了「先不动」**。
> 因此本轮的落地范围就到这里：**图表自身不再越界**（空间不足时它会缩），短窗口下接收区依旧会
> 吃光可用高度，被挤出的发送区靠「视图 → 面板」自行取舍。两个方向留在 §5 备查，未实现。

### 3.22 增量（2026-09-24：实时图表移到中央区顶部 + 复用日志滚动区）

**用户要求（逐字）**：「在侧边栏上面做一个页眉标题叫做"实时图表"然后复用日志滚动区可行吗？
可行就执行」。落点经选项卡确认为 **图表移到中央区顶部**（接收区之上）。

**做了三件事：**

1. **抽出公共滚动块**（`receive.rs::capped_scroll_area`）：把原先内联在日志两处的 `ScrollArea`
   参数（`auto_shrink([false, false])` + `max_height` + `id_salt`）抽成一个构造函数，接收区
   （纵滚 / 双滚两处）与图表**共用同一份定义**，不会出现"日志改了、图表没跟上"。
2. **图表加页眉标题**：第一行由 `ui.label` 改成 `ui.heading`（与侧边栏顶部那条页眉同一套写法）
   \+ 其下一条分隔线；右侧的「自动缩放 / 清空图表 / 样本摘要」原样保留。
3. **图表内容改由滚动块承载**：`Plot::height()` 固定 `PLOT_HEIGHT_MAX`(220)，外层套
   `capped_scroll_area(..., 220)`。

**图表高度这一天演进过三次**（每次都因用户目视或提问）：

| 版本 | 做法 | 结果 |
|---|---|---|
| ① 写死 | `Plot::height(220.0)` | 矩形**恒为 220**（同时写 `min_size.y` 与 `height` 再取 `at_least`）⇒ 中央区剩 120 点时越界 131 点，发送区被挤出可视区 |
| ② 自适应 | `plot_height(available) = clamp(90, 220)` | 越界没了，但矮窗口下**绘图区被压矮 ⇒ 纵横比失真**，趋势看起来比实际平缓 |
| ③ **滚动块**（本轮） | 内容固定 220，容器由 `ScrollArea::max_height` 夹 | 块不越界 **且** 绘图区保持 220 等比例，放不下就滚动 |

**为什么 ② 要改掉**：`ScrollArea::max_height` 是**至多**（可用空间更小时它自己缩），而
`Plot::height()` 是**固定**——把"缩"与"滚"分开、让容器负责夹高度，比让控件自己猜可用高度更可靠。
`plot_height()` 与 `PLOT_HEIGHT_MIN` 随之删除（连带删掉 2 条测它们的纯函数测试）。

**滚轮归属（读 egui 0.36.2 `containers/scroll_area.rs:1219-1254`）**：滚动区**只在"内容确实超出
可视区、且该方向还能滚"时**才消费 `smooth_scroll_delta`（源码注释原文："Clear scroll delta so no
parent scroll will use it"）。内容装得下时两个条件都不成立，滚轮原样留给内部控件 ⇒ 图表平时照常
缩放/平移；只有矮到出现滚动条，滚动块才接管滚轮。**这是"复用日志滚动区"没有把图表交互弄坏的依据。**

**测试 332 → 334**（删 2 条测已删除的 `plot_height` 的，新增 3 条，净 +2）：

- `panels::chart::squeezed_chart_clips_its_block_but_keeps_the_plot_area_whole`：矮画布下
  **块不越界**（≤ ROOM）**且绘图区仍是 220**（≥ 200）。两条缺一不可——只断前者会放走"顺手把高度
  算小"，只断后者会放走越界（改动前的写死 220 正是"绘图区完整 + 越界 131 点"）。
- `receive::capped_scroll_area_caps_the_height_and_can_scroll`：① 容器 600 比上限 400 高时只占
  400；② 容器 120 比内容 300 矮时 `content_size > inner_rect`（egui 文档给出的"需要滚动"判据）。
  **② 是关键**——只断 ① 的话，把上限设成"永远装得下"也照样绿。
- `app::chart_sits_above_the_receive_area`：比两块**落笔文字的 y 坐标**（图表页眉 < 接收区内容），
  而不是"文字存在"——后者把图表挪回底部照样通过。

**非空转验证 3 处注入全红并还原**：

- **I1** 把图表挪回发送区之后 → `chart_sits_above...` 红：「图表页眉落在 y=665.5，接收区内容落在
  y=68.0——图表不在接收区之上」。
- **I2** `max_height` 改 `f32::INFINITY` → `capped_scroll_area_caps...` 红：「容器有 600 点高，
  滚动块却占了 600.0 点——上限 400 没生效」。
- **I3** 绘图区高度改 60 → `squeezed_chart_clips...` 红：「绘图区被压到只剩 46.0 点高（固定值是
  220）」。

**为可测而做的一处抽取**：中央区那一列抽成 `ComTool::render_central_column(&mut self, ui)`。
`eframe::App::ui` 需要一个无头环境构造不出的 `eframe::Frame`，写在里面等于一次也不会被
`cargo test` 画到。⚠️ 过程中踩到一个坑：**同一个 trait 不能在两个 `impl` 块里实现**（E0119），
所以 `fn save` 必须留在同一个 `impl eframe::App for ComTool` 块内——最终把
`render_central_column` 放进独立的 `impl ComTool` 块、`save` 归位到原处。

### 3.23 增量（2026-09-26：「滚轮往上就停住」—— 缺的不是解粘，是淘汰）

**用户报告（逐字）**：「串口日志在向上滚动的时候，用滚轮往上应该自动停止日志滚动，好像没有实现
这个功能？」

**先量后修，量出来的结论与直觉相反**：解粘**本来就是好的**。无头喂一个 `Event::MouseWheel`
（+50 点）给接收区那条滚动块，`offset` 逐帧 `[700, 700, 684, 673, 665, 665]`——不粘底了，也不会
被拉回去。所以"没实现"指的不是解粘。

**真正缺的那一环是 `trim_text_front`**：日志满 `max_log_chars`(默认 30 000) 后每帧从**最旧**端淘汰
新增的等量内容。滚动位置（像素）不变、内容却整体上移 ⇒ 用户读到的那一行被换成更晚的行。复现得很
干净（把上限缩到 1/10 跑无头）：

| 帧 | 滚轮 | offset | 已裁行数 | **视口顶行** |
|---|---|---|---|---|
| 0~5 | — | 1316.0（恒定） | 5→30 | 87→112（每帧 +5，跟随中，正常） |
| 6 | 50↑ | 1300.1 | 35 | 116.3 |
| 7~13 | — | 1289→1268 | 40→70 | **120→149（每帧 +4~5，仍在前进）** |

`offset` 在缺陷下**每帧都不变**，行号却每帧往前跑——这就是"滚轮往上之后日志还在滚"的真身。
**判据必须是"视口顶部那一行的绝对行号"，不能是 `offset`。**

**修法（两半）**：

1. **读历史时不淘汰**：每次绘制把"视口是否贴底"带出来（`ReceiveScrollOutcome` →
   `ComTool::receive_at_bottom` / `receive_trim_budget`，三个接收视图共用
   `note_receive_scroll` 一处落点）。不在底部时上限放宽 `RECEIVE_TRIM_DEFER_ALLOWANCE`
   （30 000 字符 ≈ 1 000 行）；接收区没画时照旧裁到上限（没人看，内存优先）。
2. **追平欠账必须分帧**：回到贴底后每帧最多裁 `receive_trim_budget` 个字符（上一帧实测的
   "半个可视区能装多少字符"，`trim_budget_per_frame`）。

**第 2 半是量出来的，不是想出来的**：第一版一次裁完（5 850 → 3 000 字符），那一帧画出的文字
shape 里**只剩半行残句**——egui 用上一帧的滚动位置布局、把 `offset` 夹回合法范围发生在内容画完
之后，一帧缩短超过一个可视区高度就把内容整个推到视口上方。改成 371 字符/帧后逐帧都有完整内容。
"半个可视区"是按**高度**换算的，字体档位（`ui_font_scale`）与窄窗口自动跟上。

**改动落点**：`src/receive.rs`（`is_at_bottom` / `trim_budget_per_frame` /
`ReceiveScrollOutcome` / `trim_receive_logs` 的三分支 / `clear_receive` 复位）、`src/app.rs`
（两个运行时字段 + `Default`）、`src/config.rs`（`restore_runtime_state` 复位）。**没有新增任何
UI 文案**（15 种语言表不用动），也没有新增设置项。

**测试 336 → 338**（新增 4 条）：

- `is_at_bottom_covers_the_three_interesting_positions`：三态真值表（装得下 / 贴底 / 翻上去差
  1 点以外）。
- `trim_budget_stays_well_under_one_viewport`：额度须明显小于一个可视区能装的字符数，且视口减半
  时额度同步减半；量不出数时返回 `usize::MAX`（宁可空一帧也不能让上限失效）。
- `front_trim_is_deferred_while_the_user_reads_history`：读历史一个字符都不裁 / 额度用满后照旧裁 /
  贴底恢复原上限 / 接收区隐藏时照旧裁到上限 / 分帧额度确实只裁那么多。
- `wheel_up_freezes_the_reading_position`：无头驱动**真实** `render_receive_area`，喂滚轮，从
  **画出来的文字 shape** 反推视口顶行行号 ⇒ ① 往上翻之后行号只许往回走；② 滚回底部后恢复前进且
  内容重新贴住底边；③ 顺带钉住"绘制结果确实写回了 `receive_at_bottom`"。空转防护：整条链路任何
  一帧画不出日志文本都会当场 panic（即"空一帧"会被抓住，不留情面）。

**非空转验证 3 处注入全红并还原**：

- **I1** "读历史放宽"改回一律裁到 `max` → 单元测试报「用户正在读历史，却把旧内容裁掉了」（1000 vs
  6000），**且**真实路径那条报出「滚轮往上之后阅读位置还在被往前推：[109, 113, 117, 122, …]」。
- **I2** 额度改成 `chars`（= 一次裁完）→ 单元测试报「额度 5850 字符 ≈ 215 点高，已经接近可视区高度
  400 点」，真实路径那条报「这一帧没有画出接收区日志文本」（就是空一帧）。
- **I3** `is_at_bottom` 改成恒 `true` → 真值表与真实路径两条同时红。

**人工审查**：2026-09-26 18:56 用户回复「人工审查本次修复通过」。

**取舍（留档）**：读历史时内存会临时超过 `max_log_chars`（默认 30 000 → 至多 60 000 字符/镜像，
四个镜像各一份）。长时间停在历史里且设备一直喷数据，到额度后恢复淘汰，**那一刻阅读位置会跳一次**
——"内存上限优先于阅读位置"的明确选择。另一条已确认的现状：**「自动滚动」勾选框不会因手动上滚而
自动取消**（它表达的是"回到最新就继续跟随"的意图）；用户尚未要求改，先记为现状。

### 3.24 增量（2026-09-26：技术债清理第 2 项 —— clippy 8 条既有警告清零）

用户按技术债报告的顺序（2 → 4 → 3 → 1）要求实施，第 2 项即"8 条既有 clippy 警告一次性清零"。
**逐条手改，没有用 `cargo clippy --fix` 一键套用**——其中一条需要改公开方法名（见下），
一键修会在不知情的情况下改语义。

| # | lint | 位置 | 修法 |
|---|---|---|---|
| 1 | `needless_option_as_deref` | `app.rs:597` | `cc.storage.as_deref()` → `cc.storage`。`eframe` 的 `CreationContext::storage` 本身已是 `Option<&dyn Storage>`，`as_deref()` 解出来是同一个类型 |
| 2 | `unnecessary_map_or` | `app.rs:678` | `.map_or(true, …)` → `.is_none_or(…)`。语义不变："没有上次尝试 → 立刻试" |
| 3 | `needless_borrow` | `app.rs:715` | `&self.receive_text.as_bytes()` → 去掉 `&`，`as_bytes()` 已是 `&[u8]` |
| 4~6 | `manual_range_contains` ×3 | `receive.rs:141/143/145` | UTF-8 lead 字节的三档判断改成 `(0xC0..=0xDF).contains(&lead)` 等，顺带让"四段连续区间"这件事在代码里看得出来 |
| 7 | `derivable_impls` | `receive.rs:165` | `AnsiStyle` 的手写 `Default` → `#[derive(Default)]`（两字段都是零值） |
| 8 | `wrong_self_convention` | `send.rs:366` | **唯一一条不能一键修的**：`SendHistory::from_back(&self, …)`。clippy 规定 `from_*` 不接受 `self`（那是"从别物构造新值"的命名），而这是"从已有集合取一条"。重命名为 **`nth_newest`**，与同文件 `HistoryNav` 的 `older` / `newer` 同族 |

`nth_newest` 的重命名是**纯内部 API**（单 binary crate，无下游），7 处调用点（3 处生产 +
4 处测试）全部同步改完，338 条测试一次通过。

**门禁口径收紧**：此前是"0 新增"（等于默许 8 条欠账永久存在），现改为 **`cargo clippy
--all-targets` 必须 0 warning**。落点：`AGENTS.md` §7（英文摘要段）、`DEVELOPMENT_GUIDE.md`
§0 门禁行、`HANDOVER.md` §9 快速上手。

**验证**：`cargo clippy --all-targets` **0 warning**（此前 bin + test 各 8 条）；
`fmt --check` 干净；`check --all-targets` 0 warning；`test` **338 passed + 1 ignored**。
未运行本项目、未触碰任何串口。

### 3.25 增量（2026-09-26：技术债报告第 3 项 —— 依赖补丁版升级）

四个依赖落后一个补丁版，一次性升级。照报告的建议**单独一次提交**，不与其他改动混杂——
本项目 338 条测试里有大量**形状级断言**（读 `Shape::Text` / `Shape::Circle` / `Shape::Rect`
的矩形与颜色），egui 小版本一旦改排版就会当场变红，正好当回归网用。

| crate | 升级 | 说明 |
|---|---|---|
| `egui` / `eframe` | 0.36.1 → **0.36.2** | 连带 `emath` / `epaint` / `egui_glow` / `epaint_default_fonts` 一起 |
| `serialport` | 4.10.0 → **4.10.1** | |
| `encoding_rs` | 0.8.41 → **0.8.42** | 顺带摘掉 `multiversion` + `multiversion-macros` 两个传递依赖 |
| `egui_plot` 0.37.0 / `chrono` 0.4.45 / `serde` 1.0.229 / `rfd` 0.17.2 | 已是最新 | 无需动 |

**升级前先量了改动范围（这是"低风险"的依据，不是推测）**：

- `egui` 0.36.1 → 0.36.2：61 个文件、**589 行**差异，但**几乎全是 `std::` → `core::`
  的机械替换**。`containers/scroll_area.rs` 只动了 4 行，全是 `std::ops` / `std::mem` →
  `core::ops` / `core::mem`；`style.rs` 同理。布局与排版逻辑零改动——这解释了为什么
  338 条形状级断言一条都没红。
- `serialport` 4.10.0 → 4.10.1：**只动了 4 行，全在 `posix/tty.rs`**（波特率常量解析
  `B500000 => 500_000` 改成 `Ok(500_000)`）。**Windows 侧一行未改** ⇒ §5 那条
  "Windows 打开端口时显式把两根控制线置为 Disable"的结论在 4.10.1 下依然成立。

**⭐ 顺手做了一件容易被漏掉的事**：文档里有五处**带行号的 egui 源码引用**，版本一升就
可能悄悄失效。升级后逐条回查，**全部仍然成立**（这才有把握把 `0.36.1` 全部改写成
`0.36.2`）：`containers/scroll_area.rs:1219-1254` 的滚轮归属那段（"Clear scroll delta
so no parent scroll will use it" 原注释仍在原位）、`eframe/src/epi.rs:248` 的
`fn clear_color(&self, _visuals: ...)`（连行号都没变，仍是写死的 `rgba(12,12,12,180)`）、
`containers/frame.rs` 的 `Frame::NONE.fill == TRANSPARENT`、`style.rs` 的
`button_padding == vec2(4.0, 1.0)`。

**版本策略（写入 §1 依赖表）**：manifest 只写"最低要求"，实际版本由 `Cargo.lock` 钉住；
升级补丁版**只动 lock、不动 manifest**（`egui = "0.36.1"` 的 caret 本就允许 0.36.2，
把它改成 0.36.2 等于凭空多声明一条我们并不需要的最低要求）。

**验证**：`fmt --check` 干净 · `check --all-targets` 0 warning ·
`clippy --all-targets` **0 warning** · `test` **338 passed + 1 ignored**（零失败）。
未运行本项目、未触碰任何串口。**仍需目视一眼**：自动化测的是断言里的形状，测不了
"整体观感"，而 GUI 补丁版有可能改了断言之外的观感细节。

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

73. **浅色主题下整条界面的底色（含状态栏）（2026-09-24 加，用户目视发现后修）**：
    - **怎么试**：菜单栏「设置 → 主题」依次点 **浅色 / 深色 / 跟随系统**，每种都停两秒。
    - **判据（通过 = 三条全中）**：① **底部状态栏的底色跟着主题一起翻**——浅色主题下
      是浅底、深色主题下是深底，**不再是一条固定的深色横带**；② 状态栏的底色与**中央区
      连成一片**（同一档灰，看不出分界；它比深色主题下"以前的样子"会**略微亮一点点**，
      因为以前那层是 eframe 的窗口底色 (12,12,12)、现在是面板底色 (27,27,27)——这是修正
      不是新问题）；③ 浅色主题下状态栏那行字（连接状态 + 收发字节数）**看得清**，
      不是深字压在深底上。
    - **顺带看一圈**：整幅窗口的四条边（顶/底/左/右）与面板交界的缝隙里**不该出现**
      任何一条深色细带——那说明还有区域没人画背景。这是同一个缺陷在别处的表现。
    - 自动化侧已覆盖 2 条：无头渲染读出"状态栏面板背景矩形"的填充色，断言它等于**该主题
      的面板底色**（浅 248 / 深 27，于是既不透明、也不与中央区异色）；以及窗口底色
      `clear_color` 跟着主题走（见 §3.17）。**非空转验证 4 处注入全红**
      （`Frame::NONE` / 写死深色 / 退回 eframe 默认 clear_color / 写死浅色）。

74. **「打开串口」按钮的新样子（2026-09-24 加；同日晚按用户实机目视反馈修订过一轮）**：
    - **怎么试**：打开侧边栏（中央区顶部「☰ 侧边栏」），看「连接类型」下面那个按钮。
    - **判据（通过 = 六条全中）**：
      ① 它看起来**是一个按钮**：**有实心底色**、圆角 6、比旁边的「端口」「波特率」下拉框
         明显**更高**（32 vs 约 22），**而且不再横着吃满整行**——宽度只比文案宽一点，右边缘与
         侧边栏右边界之间留着一段空白。（2026-09-24 修订：原先是全宽的，用户目视后说
         「按钮不需要那么长贴到右边框」。）老实现是块无描边、无固定尺寸的"灰标签"，高度只
         等于一行文字。
      ② 文案字号 **16**，比界面正文（egui 的 `Body`，实测 13）**明显大**——用户原话
         「打开窗口这四个字太小了」，这一档因此从 14 提到 16。
      ③ 未连接时是**绿色**「打开串口」，左侧是**空心**圆点，右侧有一个**实心三角箭头**。
      ④ 鼠标悬停时底色**变亮**；按住不放时**变暗**（与旁边的下拉框手感同向）。
      ⑤ 已连接时变成**红色**「关闭串口」+ **实心白点**。
      ⑥ 三种连接类型（串口 / TCP 客户端 / UDP）的文案分别是「打开串口」「连接」「绑定」↔
         「关闭串口」「断开」「断开」。
    - **「正在打开」这一档怎么试**：串口选一个**占用中或被拔掉**的端口去打开，那一小段里按钮
      应当是**琥珀底 + 深色字**、圆点是实心的，**文案仍是「关闭串口」**（不是"正在打开"——
      那时候按下去确实是关闭），按钮**下方**另有一行小字说「正在打开…」。
    - **顺带验"提示换行"**：把「端口」下拉清空（或选一个不存在的端口）→ 按钮**下方**应当出现
      一行小字「请先选择端口」；它**不该**挤在按钮右边。
    - **浅色主题再看一遍**：菜单栏「设置 → 主题 → 浅色」，绿色按钮与红/琥珀底色在浅底上都要
      **看得清字**（琥珀那一档是深色字，其余是白字）。
    - **顺带看多语种**：切德语或俄语（按钮文案比中文长很多）→ 按钮会**变宽**（这是特性，
      宽度贴合文案），但右边缘与面板边界**仍留 24 点空白**，文字**不许压到右侧箭头上**、
      也不许被截断（放不下时缩到 14 为止，**不低于正文**）。
    - **顺带看窄侧边栏**：把侧边栏拖到最窄（200）→ 按钮跟着变窄，但不会窄过 130。
    - 自动化侧已覆盖 9 条（见 §3.18 / §3.19）：三态映射、「正在打开」文案仍在关闭侧、底色按
      动作语义 + **通道方向断言**、琥珀与状态栏同源、宽度落在判据区间且有底色、提示在按钮
      下方、状态点空心/实心、**文案字号不小于 16**、**长文案下右边距与字号下限**。
      **非空转验证 7 + 5 处注入全红**。

75. **全界面按钮统一成「打开串口」那一套外观（2026-09-24 加）**：
    - **怎么试**：把界面走一遍——侧边栏、发送区、图表面板、设置窗口（菜单栏「设置 →
      打开设置窗口」）。
    - **判据（通过 = 六条全中）**：
      ① 这些按钮**都有一个实心底色**，不再是原先那块"灰标签"。老实现是 egui 默认外观：
         实测**高 18**、圆角 2、**无实心底色**；新的是**高 28**、圆角 6、有实心底。
      ② **主操作的绿**：发送区「发送」、发送区「保存」、侧边栏「开始记录」——这三颗是
         **绿色实心**。注意「开始记录」在**开始记录之后**会变回中性灰：那时文案是
         「停止记录」，按下去不再是"发起"（底色报动作，与连接按钮同一条规则）。
      ③ **其余全是中性灰实心**：`>>` 收起、刷新、清空日志、导出日志、选择目录、清空发送框、
         常用数据按钮、清空图表、清空历史、清空预设、恢复默认。
      ④ **悬停变亮、按住变暗**（与旁边的下拉框手感同向），鼠标移上去是手型光标。
      ⑤ **宽度只比文案宽一点**，不吃满整行；同一行里两个按钮之间**有缝**，不贴在一起。
      ⑥ **位置没有挪动、也没压到别的东西**：标签、下拉框、勾选框、计数文字的左右关系与
         改动前一致；设置窗口的纵向滚动条该出现还出现。
    - **两处行高会变（预期之内，不是缺陷）**：原先那批按钮高约 18，现在标准档 28、
      紧凑档 24 ⇒ 它们所在的行会变高一点。侧边栏「端口 / 波特率」两行因为「刷新」走紧凑档，
      整行比改动前高约 6 点。
    - **禁用态看两处**：① 发送区的「保存」——输入框为空时它是**暗灰底 + 暗灰字**，
      点下去**没有任何反应**，悬停时给出置灰原因；② 发送区表头右侧的「发送历史」——
      历史为空时同样是暗灰的，且点不动。
    - **浅色主题再看一遍**（菜单栏「设置 → 主题 → 浅色」）：中性灰按钮在浅底上要**看得出
      边界**（底色 `#DCDCDC` 压在面板 `#F8F8F8` 上），绿色主操作要**看得清白字**。
    - **顺带看多语种**：切德语 / 俄语（按钮文案长得多）→ 按钮**变宽**（宽度贴合文案），
      但不许越过所在行的右边界、不许压住同行其它控件；真放不下时会**缩字号**
      （标准档缩到 13、紧凑档缩到 12 为止），**不截断**。
    - **菜单栏里的项不该变**：文件 / 设置 / 视图 / 文本四个下拉里的项、以及接收区右键菜单里
      的「复制全部日志」，**仍是原来的菜单样式**（整行高亮），不是实心块——这是定稿时刻意
      划在范围之外的。
    - 自动化侧已覆盖（见 §3.20）：9 条新增测试——三态方向、主操作偏绿、禁用态更贴近面板底色、
      两档高度 **28 / 24**（字面量）、两档字号 **15 / 14**（字面量）、宽度贴合文案、
      **宽度不超可用宽度**、**同排两按钮不重叠且留缝**、**侧边栏真的调用了新外观**。
      **非空转验证 7 处注入全红**。

76. **实时图表：位置 + 顶部页眉 + 高度由滚动块夹住（2026-09-24 加，同日二次修订；**已由用户目视确认通过**，2026-09-24 22:41 原话「目测通过」）** ——
    这一项**要把窗口拖矮**才看得出来：
    - **怎么试**：菜单栏「视图 → 面板」先勾上图表面板；确认图表现在位于**中央区最上面**
      （顶部工具栏那一行之下、接收区之上）；然后**把窗口高度往下拖**，盯着图表与它下面的接收区。
    - **判据（通过 = 四条全中）**：
      ① **位置**：图表在最上面，接收区在它下面。它原先夹在接收区与发送区中间。
      ② **页眉**：「实时图表」比同一行的其余文字明显大（`ui.heading`），下面有一条分隔线；
         右侧仍是「自动缩放 / 清空图表 / 通道数·样本数」。
      ③ **窗口够高时**：绘图区约 886 × 206、曲线铺满约九成高、横轴刻度与图例都在——与改动前一致。
      ④ **窗口变矮时**：绘图区**不再被压扁**（仍约 206 高、纵横比不变），图表块被滚动块夹住、
         出现滚动条，滚一下就能看全；**不出现"绘图区连同发送区一起被裁掉"**。
    - **滚轮**：正常窗口下（没有滚动条时）滚轮 / 拖拽照旧作用于图表；矮到出现滚动条后滚轮先滚
      容器。依据见 §3.22（egui 只在"内容确实超出且还能滚"时才消费滚轮）。
    - **仍然存在的一处（未修，见 §3.21 末与 §5）**：窗口再矮下去，接收区的滚动区会把可用高度
      吃光（它自己的上限是 400 点），发送区依旧可能被挤出。**用户 2026-09-24 在三选一里选了
      「先不动」**，因此布局结构不动——那是**已知且用户已确认的现状**，不是缺陷复发；需要时用
      「视图 → 面板」自己关掉一块。两个可选方向（接收区让位 / 中央区可滚动）留在 §5。
    - 自动化侧已覆盖（见 §3.22）：`squeezed_chart_clips_its_block_but_keeps_the_plot_area_whole`
      （**块不越界 + 绘图区仍 220**，两条缺一不可）、
      `capped_scroll_area_caps_the_height_and_can_scroll`（上限生效 + 确实需要滚动）、
      `app::chart_sits_above_the_receive_area`（比两块落笔文字的 y 坐标）。

77. **滚轮往上之后日志必须"停住"（2026-09-26 加，用户报告后修；**已由用户人工审查通过**，
    2026-09-26 18:56 原话「人工审查本次修复通过」）** ——
    这一项**用眼睛看"内容有没有从眼皮底下滑走"**，看滚动条位置是看不出来的：
    - **怎么试**：开串口，让设备持续输出到日志明显长于接收区（或者用 §4② 第 71 项那种"喂数据"
      的做法）。等日志满起来之后，**把滚轮往上滚几格**，然后把鼠标挪开、盯着接收区里你正在读的
      那一行。
    - **判据（通过 = 三条全中）**：
      ① **停住**：你正在读的那一行**不许自己往上走**。改动前它会跟着新数据的节奏一行行被顶走
         （看起来就像"自动滚动没停"）——这正是本次要修的东西。
      ② **不空屏**：读历史期间接收区**不能出现整块空白**，也不该有"内容突然跳一大段"的闪烁。
      ③ **能回到最新**：把滚轮往下一直滚到底 → 日志**重新跟上最新数据**（顶行开始逐帧前进、最新
         一行贴在可视区底边），此后仍保持 ① 期间读到的那部分历史内容可继续翻上去看。
    - **顺带看**：读历史读很久（十几秒）之后再滚到底 —— 内容可能会**跳一次**（那是"内存上限优先
      于阅读位置"的取舍，见 §3.23 末），跳完必须立刻回到"贴住最新"的正常跟随状态。
    - **顺带看**：把接收面板用「视图 → 面板」关掉再打开 —— 打开后日志应照常显示（关掉期间不
      保留无限历史）。
    - **背景**：解粘是老早就好的（egui 原生）；缺的是"读历史时别淘汰旧内容"。依据、量法、注入
      结果见 §3.23；约定见 `DEVELOPMENT_GUIDE.md` §6 第 58 条。
    - 自动化侧已覆盖 4 条（见 §3.23）：三态真值表、额度上界、裁剪放宽规则、**真实渲染路径下
      "视口顶行不许前进"**（从画出来的文字 shape 反推行号）。**非空转验证 3 处注入全红**。
      **非空转验证 3 处注入全红**（挪回底部 / 上限改无穷 / 绘图区改 60）。§3.21 那轮的 4 处注入
      与一条**负面结果**（关掉自动缩放不会让任何测试变红，不当成已覆盖）同样保留在案。

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
| ~~**高**~~ **已修** | ~~**状态栏在浅色主题下不变色（整条仍是深色横带）**~~ | `status_bar.rs` 的 `.frame(...)` + `app.rs` 的 `clear_color` | **已于 2026-09-24 修复**（用户目视发现）：面板用的是 `Frame::NONE`，其 `fill` 是 `TRANSPARENT` ⇒ **状态栏自己不画背景**，透出 `eframe::App::clear_color` 的默认值——一个**写死的**深灰 `rgba(12,12,12,180)`，参数名就叫 `_visuals`、**拿得到主题却不用**（`eframe-0.36.2/src/epi.rs:248`）。深色主题下 (12,12,12) 与面板底色 gray(27) 几乎一样所以一直没暴露；浅色主题下面板底色是 gray(248)、文字色是 gray(80) ⇒ **深字压在近黑底上**，整条状态栏"没变色、也像没内容"。修法：① 面板改用 `Frame::side_top_panel(ui.style())`（只覆盖内边距），背景 = 该主题的 `panel_fill`；② 兜底重写 `App::clear_color()` 返回 `visuals.panel_fill`，让任何未被面板覆盖的区域也跟主题走。⚠️ **它躲过全部自动化测试的原因是"没画背景"测不出来**——原测试问的是圆点数/文字/位置/留白，没有一条问"这块面板自己的背景是什么颜色"。见 §3.17 |
| 中 | 写路径瞬态错误静默重试 | `channel.rs::worker_main` | `WouldBlock` / `Interrupted` 直接重试是合理的，但设备长期不接受数据时用户得不到提示。背压修好后已有"队列已满"的 `Notice` 兜底，但队列**未满而长期写不动**时仍无提示。建议侧边栏显示「待发字节数」 |
| ~~中~~ **已修** | ~~**TCP 写侧没有写超时 ⇒ 背压不生效，还可能卡住界面**~~ | `net_impl/tcp.rs::open` / `worker_main` | **已于 2026-09-24 修复，走的是「非阻塞写」那条路**：`open()` 把 `set_read_timeout` 换成 `stream.set_nonblocking(true)`（另加 `set_nodelay(true)`），worker 循环加了 `idle` 标记与显式的 `IDLE_SLEEP = 1 ms`（非阻塞 socket 上 `set_read_timeout` 无效——阻塞性是**每 fd** 的），`WouldBlock` 时让出时间片后下一轮再试。<br>**修掉的两条后果**：① 对端彻底停读时 worker 不再卡在 `write`，1 MiB 上限真正生效（`TcpChannel` 现在跑的是与串口同一套 `PendingLedger` 账本，队列满**在调用线程当场**返回 `Err`）；② `close()` 里那个 `join()` 不再无限等——实测对端完全不读时 `close()` 也能在毫秒级返回（`close_returns_promptly_even_when_the_peer_never_reads` 钉住，3 秒上限）。代价：读侧不能再靠 `set_read_timeout`，改成 1 ms 轮询（这就是 `IDLE_SLEEP` 的另一个用途，同时决定 `close()` 的最大响应延迟）。<br>⚠️ **为什么没走"加一行写超时"**（已实测排除）：Windows 上带 `SO_SNDTIMEO` 的阻塞 `send` 超时后，数据**可能已经进入内核缓冲并发出**，而调用方只拿到 `WSAETIMEDOUT`、拿不到已写字节数；worker 于是重发整片。实测（64 KB 一片、片内含片号、超时 10 ms、对端停读 2 秒）：同一片在接收端出现 **126** 遍（片 42 的词数 1032192 = 126×8192）。非阻塞写没有这个歧义——`Ok(n)` 就是**准确**的已写字节数。串口也没有（驱动不接受就一个字节都不排队），所以那里 `.timeout(10ms)` 同时落成 `WriteTotalTimeoutConstant` 是安全的。<br>**UDP 刻意不动**（同日的对照实测）：向一个从不 `recv` 的对端发 4000 × 60 KB（`SO_SNDBUF=4096`）→ **0 次失败**、单次 `send` 最坏 **0.1 ms**（TCP 对照组挂了 **2 分 05 秒**）⇒ UDP 没有"对端停读 ⇒ worker 卡住"这条路径。见 §3.15 ④ 与 `tools/backpressure_check.py` 的 `--self-test` |
| 低 | UDP 的 `send` 仍可能被本机网卡出口堵住 | `net_impl/udp.rs::write`（刻意保持阻塞） | 与 TCP 那条**不是同一个问题**：本机到对端的**网卡出口**被打满时（例如 100 Mbps 网卡上持续满速发大包），`send` 仍会阻塞。实测在回环上向从不 `recv` 的对端发 4000 × 60 KB 是 **0 次失败 / 最坏 0.1 ms**，所以没有"对端停读 ⇒ worker 卡住"这条常被踩到的路径，这是 UDP 不改非阻塞的依据。但要构造出出口饱和需要真实网卡与真实流量，**未实测**，因此保留为已知残余风险；真出现时应按 TCP 同样的方式改非阻塞（`WouldBlock` 对 UDP 是"本机队列满、报文未发出"，语义清晰、无重发歧义） |
| ~~低~~ **已清理** | ~~2 个未使用依赖~~ | `Cargo.toml` | 已于 P6 第 1 批移除：`tokio`（features = `["full"]`，零调用，明显拖慢编译）与 `egui-chinese-font`（零调用，中文字体已由 `main.rs::setup_fonts` 读系统字体实现）。`chrono` / `egui_plot` / `rfd` / `serde` / `serialport` 仍是活跃依赖 |
| ~~低~~ **已修** | ~~侧边栏连接按钮的**文案与动作是反的**~~ | `sidebar.rs` | **已于 2026-09-21（F4）修复**：原来「已连接」时按钮显示「打开串口」、点下去却执行关闭，从上游继承而来（`e2039ff:src/sidebar.rs:157` 即在）。F4 重写这段时一并改成同向，并按传输类型分叉文案。见 §3.8 |
| ~~低~~ **已清零** | ~~8 条 clippy 风格警告~~ | `app.rs` / `receive.rs` | ✅ **已于 2026-09-26 清零**（8 → 0，见 §3.24）。逐条手改而非 `cargo clippy --fix` 一键套用（其中 `wrong_self_convention` 一条需把 `SendHistory::from_back` 重命名为 `nth_newest`，一键修会在不知情的情况下改公开 API）。门禁同步从"0 新增"收紧为 **0 warning**。以下为清零前的现场记录（2026-09-24 按 `cargo clippy --all-targets` **逐类现场重数**，此前这一行写的是 `derivable_impls`×2，与实测不符，已订正）：`manual_range_contains`×3（`manual RangeInclusive::contains implementation`）、`derivable_impls`、`unnecessary_map_or`、`redundant_borrow`、`wrong_self_convention`（`methods called from_* usually take no self`）、deref 冗余（`derefed type is same as origin`）各 1 条，共 **8 条**。全部位于**本任务未触及**的既有代码（P4 顺带修掉了 `send_data` 里那条 `collapsible_if`，因为该函数被重写）。`cargo clippy --fix` 可一键清掉，但那会改动范围之外的代码，因此留作一次专门的"清警告"提交（P6 收尾不再处理） |
| 低 | 记录中途改配置不切换文件 | `log_file.rs` | 记录过程中改「文件名模板」或「输出目录」不会切到新文件，下次开始记录才生效；界面上没有特别提示 |
| 低 | **图表不支持无换行的连续逗号流** | `chart.rs::push_bytes` | 通道语义（第 k 个字段 = 第 k 条曲线）要求「一行一组」；无换行时通道数只能靠猜，猜错就画错位。因此这类数据在超过 4096 字节后被**整行丢弃**并在界面计数。属于明确的设计取舍，不是待修 bug |
| 低 | 绘制抽样（4000 点/曲线）未在界面暴露 | `panels/chart.rs` | 样本上限（可配）与绘制抽样（固定 4000）是两层不同的限制，界面只显示了前者。用户设到 10 万时会看到"曲线比样本数少"，目前只能靠文档解释 |
| 低 | 图表 x 轴是组序号而非时间 | `chart.rs` | 串口数据没有可靠时间基准，因此看不出真实时间跨度或采样率。若将来要显示"每秒多少个样本"，需要在解析端额外维护时间戳 |
| 低 | 图表解析在面板隐藏时也照常进行 | `receive.rs::receive_data` | 刻意如此：打开面板时立刻能看到已有曲线，而不是"打开后才从零开始"。代价是即使用户从不看图，每次接收仍会做一次扫描与少量解析 |
| ~~低~~ **已修** | ~~「HEX 接收」与「字节查看」同时勾上时**字节查看视图是空的**~~ | `app.rs` 接收设置行 / `panels/menu_bar.rs` / `receive.rs` | **已于 2026-09-21 修复**：两者改为互斥，规则收敛在 `ComTool::resolve_display_mode()`（两个界面入口 + 启动修复都走它），`receive_data()` 里的落点判据保留作纵深防御。见 §3.6 |
| ~~低~~ **已修** | ~~「颜色」与「字节查看」同时勾选时，显示的是彩色视图~~ | `receive.rs::display_mode` / `app.rs` 的渲染分发 | **已于 2026-09-24 修复**（用户实机目视审查发现，见 §3.16）。<br>⚠️ **上一版把这一行判错了**：原写"不丢内容，只是被另一视图盖住"——**不对**。真实情况是数据只写进 `ascii_spans`，而彩色视图渲染的是**从来没人写过**的 `display_spans` ⇒ 接收区**一片空白**，用户看到的就是"日志内容丢了"。错因是**只读渲染分支、没核对镜像落点**。<br>根因：三处判定不同源——片段构造与镜像落点只看 `hex_view` / `ascii_view`，渲染分支却把 `color_view` 排在最前。修法：① 新增 `ComTool::display_mode()` 作为**唯一判定**，「片段构造 / 镜像落点 / 渲染分发」三处都读它（渲染分发另抽成 `render_receive_area()`，否则写在 `ui()` 里测不到）；② 把「颜色」纳入 `resolve_display_mode` 的互斥（与「字节查看」互斥，**最后被勾上的胜出**），启动修复一并覆盖 `ascii + color`（字节视图胜出）。「HEX + 颜色」是**合法**组合，行为保持不变 |
| 低 | 时间戳前缀在 HEX / 字节查看视图里是"注解"而不是收到的字节 | `receive.rs` | 刻意如此（三种视图一致 + 日志文件一致），靠方括号与灰色区分。要一份纯字节记录请关掉前缀（默认就是关闭）。实时日志文件同样会带上前缀 |
| ~~中~~ **已修**（同日两次修订） | ~~**图表高度写死 220 ⇒ 可用空间不足时越界，把下面的发送区挤出可视区**~~ | `panels/chart.rs` | **已于 2026-09-24 修复**（用户问"实时图表的窗口高度这么小，能够正常显示数据吗"时实测暴露）：`Plot::height(220.0)` 在 egui_plot 里**同时**落成 `min_size.y = 220` 与 `height = Some(220)`，矩形**恒为 220 点**、不随可用空间缩。实测中央区只剩 120 点时图表块仍占用 251 点、**越界 131 点**；中央区没有滚动条，越界部分被面板裁掉 ⇒ 夹在图表**下面**的发送区整块落到可视区之外。**第二次改法（当前）**：绘图区固定 220，外层套 `receive::capped_scroll_area`（与日志共用同一个构造函数）——`ScrollArea::max_height` 是**至多**，由它夹住块高并给出滚动条，绘图区**不再被压扁**（纵横比不失真）；中间那版「按可用高度在 `[90, 220]` 之间夹」已被取代并删除。见 §3.21 与 §3.22 |
| 低 | **短窗口下接收区会吃光可用高度**（本轮只修了图表那一半，**用户已确认"先不动"**） | `receive.rs::show_plain_receive`（`.max_height(400.0)` + `auto_shrink([false, false])`） | 仍然存在：中央区高度低于约 700 点时，接收区按 `min(400, 可用)` 占位（实测中央区 420 → 接收区底正好也是 420），**发送区依旧被挤出可视区**；中央区没有滚动条，够不着。<br>2026-09-24 起图表块已改为**由滚动块夹住**（不再是 90 点下限），所以这一项现在的表现是"图表块先被夹小、可滚动"，被挤出的只剩发送区。<br>**用户决定（2026-09-24）**：在"让接收区让位 / 中央区整体可滚动 / 先不动"里选了 **先不动** —— 本轮只保留"图表不再越界"，布局结构不动，短窗口下用「视图 → 面板」自行取舍。两个方向备查：① 让接收区按"可用高度 − 图表与发送区的下限"主动让位；② 把中央区整体套一层纵向滚动区（代价是与接收区自己的滚动条形成嵌套滚动）。 |

---

## 6. 后续开发计划

> **注意：现在有两份计划文档，基线不同、数字不可混用。**
>
> | 文档 | 基线 | 覆盖率口径 | 状态 |
> |---|---|---|---|
> | `FEATURE_GAP_AND_PLAN.md` | `SerialGUI-rs` v1.4.0（Rust + egui，GPLv3） | 约 **90%** | P0~P6 **已收口**（唯一明确不做的是版本检查） |
> | `FUTURE_ROADMAP.md` | `Neutree/COMTool`（Python + PyQt5） | 约 **74%**（36/50 完成 + 2 部分；F1 之前是 26/53 ≈ 49%） | **新开的计划线**：**F1~F5 全部已完成**（F5 后口径），插件架构明确不做 |
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
④ ~~§5 里「颜色 + 字节查看同时勾选」的界面行为决策~~ —— **已于 2026-09-24 修复并订正判断**：
   原先这一行写的「后者被盖住；已不丢内容」是**错的**（只读了渲染分支、没核对镜像落点）——
   实际是彩色视图渲染了一个从未被写入的镜像 ⇒ **接收区一片空白**。修法见 §3.6 与 §5 对应行。

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

0. **⛔ 未经项目所有者逐次批准，禁止运行本项目（`cargo run` / 直接跑 exe）。串口按端口分两类：**
   **COM3 受长测门闩保护**（门闩在 ⇒ 一律不碰；门闩不在 ⇒ 可烧录/测试/复位）；
   **COM4 是 AI 功能测试放行端口**（不受门闩影响），**但必须置 DTR 才有日志输出**
   ——GUI 里勾 DTR 控制线，脚本里 `ser.dtr = True` / `DtrEnable = true`；没置 DTR 收不到日志
   不是程序的问题，先查这一项。开发阶段只用 `cargo fmt` / `cargo check --all-targets` /
   `cargo test` 验证（338 条测试均不需要串口硬件）。需要实机运行时停下来请求批准。
   完整条文在 `AGENTS.md` §2 与 `DEVELOPMENT_GUIDE.md` §0（**两者 2026-09-26 同步更新过：
   旧版写的是"COM3 永远拒绝"，与已实现的门闩语义不符**）。
   **该规则已配置机器级硬拦截**：PreToolUse 钩子 `~/.workbuddy/hooks/comtool-run-guard.sh`
   （注册于 `~/.workbuddy/settings.json` 与 `~/.codebuddy/settings.json`），命中即拒绝工具调用，
   fail-closed；自测 `python3 .workbuddy/hooks/test_guard.py`（52 条断言）；限时批准由所有者
   本人执行 `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（批准只放行"运行"，与串口判定
   互不影响）。

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
  `test_out.txt`（**2026-09-26 起**：该文件此前被误跟踪，是某次手工 `cargo test` 的
  输出重定向，已删除并加进忽略名单防复发）、
  `.workbuddy/`（**唯一例外：`hooks/test_guard.py`**）、`.opencode/agents/*.md`、`.opencode/prompts/*.md`。
  - `AGENTS.md` **原先也在此列，现已被移出**（2026-09-21）：它随仓库传播，不再只是本机文档。
    取消排除的原因很简单——它记的是"怎么改这个项目"的硬约定，而不是本机环境，放在仓库外
    等于换台机器就没人看得见。**同一批条文在 `DEVELOPMENT_GUIDE.md` §0/§6 与本文 §7 各有一份，
    三处仍需同步。**
  - `.workbuddy/hooks/test_guard.py` 也在同一天开了例外（守卫拦截规则的行为自测，
    **2026-09-26 起 52 条断言**：门闩开/关两档分别断言，门闩档用临时配置目录造门闩）。
    写法是四行而不是一条否定规则：`.workbuddy/*` + `!.workbuddy/hooks/` + `.workbuddy/hooks/*`
    + `!.workbuddy/hooks/test_guard.py`——**父目录被排除时无法单独重新包含其中的文件**，
    必须先否定父目录让 git 愿意进去。第三行把 `hooks/` 重新收窄成"只放行名单"，
    将来往该目录多丢文件时不会顺手一起进仓库。其余 `.workbuddy/` 内容（含 `memory/`）仍全部忽略。
  - ⚠️ 但**被测试的那个脚本本体仍不进仓库**：`comtool-run-guard.sh` 与批准脚本在
    `~/.workbuddy/hooks/`（用户级，不在本仓库内）。所以新克隆的仓库能读到规则、也能读到
    自测脚本，却**没有可测的对象**——要复现拦截必须另行取得这两个脚本。
    **文档与自测随仓库走，并不等于保护随仓库走。**
- **`echo_raw.txt`**：根目录下的串口捕获文件（**未被跟踪**，`.gitignore` 已排除），内容无关紧要。
  `test_out.txt` 同类，但**此前被误跟踪**，2026-09-26 删除并加进忽略名单（技术债报告第 4 项）。
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

# 3. 建立门禁基线（应为 0 warning / 338 passed / 1 ignored / fmt 干净 /
#    clippy --all-targets **0 warning**）
cargo fmt && cargo check --all-targets && cargo test && cargo clippy --all-targets

# 4. 看清两个版本的差异
git diff master ai-refactor --stat

# 5. 读这三份文档再动手
#    AGENTS.md / DEVELOPMENT_GUIDE.md / REFACTOR_REPORT.md

# 6. 计划内的功能项已全部落地（P0~P6 与 F1~F5），已知缺陷也已在 2026-09-24 全部收口
#    （§5 现在只剩"低"档设计取舍，见 §3.15）。**下一件活是 §4② 的实机验证清单**
#    （77 项 + 34a/34b/57b，共 80 条），都需所有者亲手跑。
#    **有"通过"记录的只有三条**：第 64 项（文档原已标注"颜色/大小/垂直对齐三项已由用户目视
#    确认通过"）、第 76 项（2026-09-24）与第 77 项（2026-09-26 用户人工审查通过）。第 72/73/74
#    项属"用户目视**发现**问题 → 已修"，**修复后没有再次目视确认的记录**。第 44~47 / 56 / 58 项
#    依赖硬件，第 1~5 项（磁盘持久化）从未做过一次。
#    ✅ **clippy 已于 2026-09-26 清零**（8 → 0），门禁口径同步从"0 新增"提到"0 warning"，
#    见 §3.24；GPLv3 合规已降为最低优先级（§4①）。
```

如果你要对照原版行为：`git -C E:\Downloads\Zed\comtool-rs log -1`（原版停在 `04dc007`），
或在本仓库 `git checkout master`。
