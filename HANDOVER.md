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
├── app.rs        (613)     ComTool 状态 + Default + 串口生命周期 + eframe::App 编排
├── receive.rs   (1465)     接收链路：四份镜像状态 + AnsiDecoder + 字节可视化
│                           + 可选时间戳前缀（TimestampPrefix）+ 三种渲染
│                           + 显示模式互斥（resolve_display_mode）+ FIFO 淘汰（39 测试）
├── chart.rs      (485)     实时图表数据层：多通道数值流解析 + 跨包分片
│                           + 样本 FIFO 淘汰 + 通道上限（20 测试）
├── log_file.rs   (801)     实时文件日志：后台写盘线程 + 文件名模板渲染
│                           + 同名防截断 + 待写队列上限（14 测试）
├── sidebar.rs    (288)     右侧边栏；返回 SidebarAction 而非在绘制中改状态
├── config.rs     (497)     持久化策略 + 启动不变量修复（15 测试）
├── panels/
│   ├── mod.rs     (151)    面板模块出口（5 条无头 UI 冒烟测试）
│   ├── menu_bar.rs(132)    顶部菜单栏；返回 MenuAction；主题/时间戳/显示模式单选共用入口
│   ├── settings.rs(182)    设置窗口（含实时图表与文件日志区块）
│   ├── chart.rs   (223)    实时图表面板：egui_plot 绘制 + 绘制点抽样（5 测试）
│   └── info.rs    (53)     信息窗口
├── communicationtrait.rs (68)  ByteChannel trait + ChannelEvent
├── serial_impl/
│   ├── channel.rs (368)    串口后台线程实现（6 测试）
│   ├── config.rs  (46)     SerialConfig + 波特率档位
│   └── mod.rs     (13)     出口
```

测试函数合计 **116**（`cargo test` 报 115 passed、1 ignored；1 条 ignored 是图标 ASCII 预览，
需要目视时用 `cargo test app_icon::tests::ascii_preview -- --ignored --nocapture`）。

**权威约定文档**（改代码前先读）：
- `AGENTS.md` — 给 AI/协作者的模块地图与硬性约束（简短，必读）
- `DEVELOPMENT_GUIDE.md` — 中文开发参考：四层架构、数据流、新增功能的完整流程、注意事项 28 条

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

| 命令 | 前 | P0/P1 后 | P4 后 | P5 后 | P6 第 1 批后 | P6 第 2 批 / P3 后 | 本轮（显示模式互斥修复）后 |
|---|---|---|---|---|---|---|---|
| `cargo check --all-targets` | 0 error / **12 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** |
| `cargo test` | 7 passed | **19 passed** | **50 passed** | **79 passed** | **90 passed** | **109 passed**（1 ignored，为图标 ASCII 预览） | **115 passed**（1 ignored） |
| `cargo fmt --check` | 2 处差异 | **干净** | **干净** | **干净** | **干净** | **干净** | **干净** |
| `cargo clippy --all-targets` | 未测量 | 9 warning | **8 warning** | **8 warning**（与上轮逐条相同，全在我未改动的既有代码里：`derivable_impls`×2、`manual_range_contains`×3、`explicit_deref`、`unnecessary_map_or`、`redundant_borrow`） | **8 warning**（逐条同上） | **8 warning**（逐条同上） | **8 warning**（逐条同上；本次改动 0 新增） |

跑法：

```bash
cd E:/Downloads/Zed/AI_comtool-rs
cargo fmt && cargo check --all-targets && cargo test
```

115 条测试**都不需要串口硬件**，包括一条走完异步打开失败路径的集成测试、
一条把字节经事件通道注入后核对日志文件内容的集成测试、五条用
`egui::__run_test_ui` 跑的无头 UI 冒烟测试（菜单栏、设置/信息窗口、三种接收视图、
侧边栏、图表面板），11 条对应用图标做纯断言的单元测试（尺寸、透明度、渐变、
前景笔画、越界检查、确定性），以及 17 条 P3 时间戳前缀测试（格式、三种模式下
的插入位置、跨批次不重复、程序提示不加前缀、清空后重新标注、不进图表）。
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
  此时「字节查看」勾着却不显示对应视图。这是同一类"勾了没反应"的老问题，但性质比空视图轻
  （不丢内容、只是被另一视图盖住），本次未动——见 §5 末行，需要一次界面行为决策。

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

### ③ 仓库状态

`AI_comtool-rs` 的 `ai-refactor` 分支工作区是干净的。
`comtool-rs` 也是干净的（等于 HEAD）。

---

## 5. 已知缺陷与残余风险

| 严重度 | 问题 | 位置 | 说明 |
|---|---|---|---|
| ~~**高**~~ **已修** | ~~**接收缓冲无上限**~~ | `receive.rs` 的 `trim_receive_logs` | 已于 2026-09-21 修复：新增 `max_log_chars`（默认 30000，0 = 不限）FIFO 淘汰，四份镜像各自按同一字符预算裁剪，支持从句中切开 `ColoredSpan`。接收日志仍不参与持久化（退出时保存大日志只会变慢，无明确需求） |
| **高** | **`connected` 假阳性** | `app.rs::open_port` | `open()` 只要线程成功 spawn 就返回 `Ok`，随即 `connected = true`；真正打开失败要等 `Failed` 事件回来才置 false。UI 会闪一下「已连接」 |
| 中 | 自动重连无退避 | `app.rs::auto_reconnect` | 每 500 ms 失败一次就新建线程，串口被占用时会持续重试 |
| 中 | 待发队列无上限 | `channel.rs::worker_main` 的 `outgoing` | 设备长期不可写 + 用户持续点发送时队列增长。原实现用 unbounded mpsc 同样无上限，未引入新问题，但应加背压 |
| 中 | 写路径瞬态错误静默重试 | `channel.rs::worker_main` | `WouldBlock` / `Interrupted` 直接重试是合理的，但设备长期不接受数据时用户得不到提示。建议侧边栏显示「待发字节数」 |
| ~~低~~ **已清理** | ~~2 个未使用依赖~~ | `Cargo.toml` | 已于 P6 第 1 批移除：`tokio`（features = `["full"]`，零调用，明显拖慢编译）与 `egui-chinese-font`（零调用，中文字体已由 `main.rs::setup_fonts` 读系统字体实现）。`chrono` / `egui_plot` / `rfd` / `serde` / `serialport` 仍是活跃依赖 |
| 低 | 8 条 clippy 风格警告 | `app.rs` / `receive.rs` | `derivable_impls`×2、`manual_range_contains`×3、`explicit_deref`、`unnecessary_map_or`、`redundant_borrow`。全部位于**本任务未触及**的既有代码（P4 顺带修掉了 `send_data` 里那条 `collapsible_if`，因为该函数被重写）。`cargo clippy --fix` 可一键清掉，但那会改动范围之外的代码，因此留作一次专门的"清警告"提交（P6 收尾不再处理） |
| 低 | 记录中途改配置不切换文件 | `log_file.rs` | 记录过程中改「文件名模板」或「输出目录」不会切到新文件，下次开始记录才生效；界面上没有特别提示 |
| 低 | **图表不支持无换行的连续逗号流** | `chart.rs::push_bytes` | 通道语义（第 k 个字段 = 第 k 条曲线）要求「一行一组」；无换行时通道数只能靠猜，猜错就画错位。因此这类数据在超过 4096 字节后被**整行丢弃**并在界面计数。属于明确的设计取舍，不是待修 bug |
| 低 | 绘制抽样（4000 点/曲线）未在界面暴露 | `panels/chart.rs` | 样本上限（可配）与绘制抽样（固定 4000）是两层不同的限制，界面只显示了前者。用户设到 10 万时会看到"曲线比样本数少"，目前只能靠文档解释 |
| 低 | 图表 x 轴是组序号而非时间 | `chart.rs` | 串口数据没有可靠时间基准，因此看不出真实时间跨度或采样率。若将来要显示"每秒多少个样本"，需要在解析端额外维护时间戳 |
| 低 | 图表解析在面板隐藏时也照常进行 | `receive.rs::receive_data` | 刻意如此：打开面板时立刻能看到已有曲线，而不是"打开后才从零开始"。代价是即使用户从不看图，每次接收仍会做一次扫描与少量解析 |
| ~~低~~ **已修** | ~~「HEX 接收」与「字节查看」同时勾上时**字节查看视图是空的**~~ | `app.rs` 接收设置行 / `panels/menu_bar.rs` / `receive.rs` | **已于 2026-09-21 修复**：两者改为互斥，规则收敛在 `ComTool::resolve_display_mode()`（两个界面入口 + 启动修复都走它），`receive_data()` 里的落点判据保留作纵深防御。见 §3.6 |
| 低 | 「颜色」与「字节查看」同时勾选时，显示的是彩色视图 | `app.rs` 的渲染分支顺序 | `color_view` 在渲染顺序里优先，于是「字节查看」勾着却不显示对应视图——同类"勾了没反应"，但**不丢内容**，只是被另一视图盖住（上面的空视图缺陷已修）。最小修法是二选一：让字节视图在渲染里优先，或勾上字节视图时把「颜色」置灰。需要一次界面行为决策 |
| 低 | 时间戳前缀在 HEX / 字节查看视图里是"注解"而不是收到的字节 | `receive.rs` | 刻意如此（三种视图一致 + 日志文件一致），靠方括号与灰色区分。要一份纯字节记录请关掉前缀（默认就是关闭）。实时日志文件同样会带上前缀 |

---

## 6. 后续开发计划

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
④ §5 里「颜色 + 字节查看同时勾选」的界面行为决策（后者被前者盖住；已不丢内容，属同类小问题）。
其余可做的新功能（例如图表按通道显示/隐藏、x 轴改成真实时间基准）都不在原计划内。

---

## 7. 硬性约定（改代码前务必遵守）

来自 `AGENTS.md` 与 `DEVELOPMENT_GUIDE.md`，这里只列最容易踩的：

0. **⛔ 未经项目所有者逐次批准，禁止运行本项目（`cargo run` / 直接跑 exe），禁止以任何方式
   占用或打断 COM3。** 开发阶段只用 `cargo fmt` / `cargo check --all-targets` / `cargo test`
   验证（115 条测试均不需要串口硬件）。需要实机运行时停下来请求批准，由所有者自行启动。
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
    打开后交给线程——这是刻意与 `open_port` 的「乐观置位」不同的写法，别改回去。
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

---

## 8. 环境与工作方式备注

- **`.opencode/`（61 MB）** 是 OpenCode AI 工具的配置与 node_modules，已随目录搬迁。
  `opencode.json` 配了三套角色：`executor`（改代码，默认）/ `planner`（只规划）/
  `expert`（攻难题），默认走 DeepSeek-V4-Flash。
- **`.gitignore` 忽略**：`SerialGUI-rs/`、`opencode.json`、`target/`、`echo_raw.txt`、
  `AGENTS.md`、`.opencode/agents/*.md`、`.opencode/prompts/*.md`。
  ⚠️ 注意 `AGENTS.md` 本身被忽略——**它不会随 git 传播**，所以它是"本机文档"，改动只在本地生效。
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

# 3. 建立门禁基线（应为 0 warning / 115 passed / 1 ignored / fmt 干净）
cargo fmt && cargo check --all-targets && cargo test

# 4. 看清两个版本的差异
git diff master ai-refactor --stat

# 5. 读这三份文档再动手
#    AGENTS.md / DEVELOPMENT_GUIDE.md / REFACTOR_REPORT.md

# 6. 计划内的功能项已全部落地（P0~P6）。下一件活从 §4② 的磁盘持久化实机验证（需所有者亲手跑）
#    或 §5 里那 8 条既有 clippy 警告的清理开始；GPLv3 合规已降为最低优先级（§4①）
```

如果你要对照原版行为：`git -C E:\Downloads\Zed\comtool-rs log -1`（原版停在 `04dc007`），
或在本仓库 `git checkout master`。
