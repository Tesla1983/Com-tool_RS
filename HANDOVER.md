# 交接文档 — AI_comtool-rs

> 面向下一个会话的接手者。读完这一份应该就能直接开工，不需要回头翻聊天记录。
> 生成时间：2026-09-21

---

## 1. 你在哪 / 两份代码的关系

工程被拆成两个**互相独立**的目录，各自是完整可编译的项目：

| 目录 | 角色 | 当前分支 | 当前提交 | `.git` 体积 |
|---|---|---|---|---|
| `E:\Downloads\Zed\AI_comtool-rs` | **AI 重构版**（本目录，主要工作对象） | `ai-refactor` | `a21d75e`（P2/P3）→ 其后是 P4 提交（见 §3.2） | 243 KB |
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
| 其它 | `rfd`（文件对话框）、`serde`（持久化）、`chrono`（日志文件名时间戳，P4 起在用）、`egui_plot` 0.37（实时图表，P5 起在用）。`tokio` 与 `egui-chinese-font` 已作为死依赖移除（见 P6 第 1 批） |
| 类型 | 单 binary crate（无 workspace、无 CI、无 lint 配置） |

### 模块结构（`ai-refactor` 分支）

```
src/
├── main.rs                 入口；字体回退链；eframe::run_native；APP_TITLE 常量；挂 `app_icon` 到窗口图标
├── app_icon.rs   (289)     256×256 圆角方波图标，代码绘制、零资源文件、11 测试（1 条 ASCII 预览默认跳过）
├── app.rs        (595)     ComTool 状态 + Default + 串口生命周期 + eframe::App 编排
├── receive.rs    (886)     接收链路：四份镜像状态 + AnsiDecoder + 字节可视化
│                           + 三种渲染 + FIFO 淘汰（17 测试）
├── chart.rs      (485)     实时图表数据层：多通道数值流解析 + 跨包分片
│                           + 样本 FIFO 淘汰 + 通道上限（20 测试）
├── log_file.rs   (801)     实时文件日志：后台写盘线程 + 文件名模板渲染
│                           + 同名防截断 + 待写队列上限（14 测试）
├── sidebar.rs    (286)     右侧边栏；返回 SidebarAction 而非在绘制中改状态
├── config.rs     (434)     持久化策略 + 启动不变量修复（12 测试）
├── panels/
│   ├── mod.rs     (151)    面板模块出口（5 条无头 UI 冒烟测试）
│   ├── menu_bar.rs(106)    顶部菜单栏；返回 MenuAction
│   ├── settings.rs(180)    设置窗口（含实时图表与文件日志区块）
│   ├── chart.rs   (223)    实时图表面板：egui_plot 绘制 + 绘制点抽样（5 测试）
│   └── info.rs    (53)     信息窗口
├── communicationtrait.rs (68)  ByteChannel trait + ChannelEvent
├── serial_impl/
│   ├── channel.rs (368)    串口后台线程实现（6 测试）
│   ├── config.rs  (46)     SerialConfig + 波特率档位
│   └── mod.rs     (13)     出口
```

测试函数合计 **90**（`cargo test` 报 90 passed，1 ignored；两者一致）。

**权威约定文档**（改代码前先读）：
- `AGENTS.md` — 给 AI/协作者的模块地图与硬性约束（简短，必读）
- `DEVELOPMENT_GUIDE.md` — 中文开发参考：四层架构、数据流、新增功能的完整流程、注意事项 20 条

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

| 命令 | 前 | P0/P1 后 | P4 后 | 本轮（P5）后 | 本轮（P6 第 1 批）后 |
|---|---|---|---|---|---|
| `cargo check --all-targets` | 0 error / **12 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** | 0 error / **0 warning** |
| `cargo test` | 7 passed | **19 passed** | **50 passed** | **79 passed** | **90 passed**（1 ignored，为图标 ASCII 预览） |
| `cargo fmt --check` | 2 处差异 | **干净** | **干净** | **干净** | **干净** |
| `cargo clippy --all-targets` | 未测量 | 9 warning | **8 warning** | **8 warning**（与上轮逐条相同，全在我未改动的既有代码里：`derivable_impls`×2、`manual_range_contains`×3、`explicit_deref`、`unnecessary_map_or`、`redundant_borrow`） | **8 warning**（与上轮逐条相同，未改既有代码；`app_icon.rs` 与 `Cargo.toml` 新代码 0 新增） |

跑法：

```bash
cd E:/Downloads/Zed/AI_comtool-rs
cargo fmt && cargo check --all-targets && cargo test
```

90 条测试**都不需要串口硬件**，包括一条走完异步打开失败路径的集成测试、
一条把字节经事件通道注入后核对日志文件内容的集成测试、五条用
`egui::__run_test_ui` 跑的无头 UI 冒烟测试（菜单栏、设置/信息窗口、三种接收视图、
侧边栏、图表面板），以及 11 条对应用图标做纯断言的单元测试（尺寸、透明度、渐变、
前景笔画、越界检查、确定性）。

### 3.1 增量（2026-09-21：P2 菜单栏 + 设置面板，P3 部分提前）

| 项 | 值 |
|---|---|
| 新增模块 | `src/panels/{mod,menu_bar,settings,info}.rs` |
| 新增持久化字段 | `theme` `auto_scroll` `max_log_chars` `repaint_interval_ms` `clear_send_after_send` `default_log_file_name` `check_update_on_start` `show_receive_panel` `show_send_panel` |
| 新增运行时字段 | `show_info`（与 `show_settings` 一样 `#[serde(skip)]`） |
| 门禁 | `cargo fmt --check` 干净 · `cargo check --all-targets` **0 warning** · `cargo test` **34 passed** · clippy 仍是 9 条既有警告（新代码 0 新增） |

- **上游 F 组（顶栏与菜单）4 项全部可用**：File(Info/Quit) · Settings 菜单 · 主题切换
  （浅色/深色/跟随系统）· 面板显隐勾选（至少保留一个可见）。
- **上游 G 组（设置项）6 项中 4 项已真正接线**：日志长度上限 · 发送后清空 · 自动滚动 ·
  重绘间隔（16~1000 ms）。剩下 2 项**只有设置项本身**：`default_log_file_name` 等 P4 的
  实时文件日志消费、`check_update_on_start` 等 P6 的版本检查消费——两者在设置窗口里都
  带了明确标注，不会让人误以为已经生效。
- **P3 提前完成的部分**：`max_log_chars` FIFO 淘汰（四份镜像各自按同一字符预算裁剪、
  能从句中切开 `ColoredSpan` 并保留其样式、按字符边界切分不破坏 UTF-8）+ `auto_scroll`
  开关。P3 剩下的"可选时间戳前缀"仍未做。
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
- **版本检查仍未实现**：设置项 `check_update_on_start` 已存在但没有任何消费逻辑。见 §6 后续计划与下文"下一步"。

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
| 低 | 8 条 clippy 风格警告 | `app.rs` / `receive.rs` | `derivable_impls`×2、`manual_range_contains`×3、`explicit_deref`、`unnecessary_map_or`、`redundant_borrow`。全部位于截至 P5 未改动的既有代码（P4 顺带修掉了 `send_data` 里那条 `collapsible_if`，因为该函数被重写；其余 8 条留待 P6 一并处理） |
| 低 | 记录中途改配置不切换文件 | `log_file.rs` | 记录过程中改「文件名模板」或「输出目录」不会切到新文件，下次开始记录才生效；界面上没有特别提示 |
| 低 | **图表不支持无换行的连续逗号流** | `chart.rs::push_bytes` | 通道语义（第 k 个字段 = 第 k 条曲线）要求「一行一组」；无换行时通道数只能靠猜，猜错就画错位。因此这类数据在超过 4096 字节后被**整行丢弃**并在界面计数。属于明确的设计取舍，不是待修 bug |
| 低 | 绘制抽样（4000 点/曲线）未在界面暴露 | `panels/chart.rs` | 样本上限（可配）与绘制抽样（固定 4000）是两层不同的限制，界面只显示了前者。用户设到 10 万时会看到"曲线比样本数少"，目前只能靠文档解释 |
| 低 | 图表 x 轴是组序号而非时间 | `chart.rs` | 串口数据没有可靠时间基准，因此看不出真实时间跨度或采样率。若将来要显示"每秒多少个样本"，需要在解析端额外维护时间戳 |
| 低 | 图表解析在面板隐藏时也照常进行 | `receive.rs::receive_data` | 刻意如此：打开面板时立刻能看到已有曲线，而不是"打开后才从零开始"。代价是即使用户从不看图，每次接收仍会做一次扫描与少量解析 |

---

## 6. 后续开发计划

完整计划见 **`FEATURE_GAP_AND_PLAN.md`**（含逐项功能对照表、上游 33 项能力清单、合规分析）。
该文档里 §7 的阶段划分与当前进度的对应关系：

| 阶段 | 内容 | 状态 |
|---|---|---|
| **P0** 架构拆分 | `app.rs` 拆为模块 | ✅ **已完成** |
| **P1** 配置与持久化 | `AppSettings` + `ComTool` 派生 serde + `save()` | ✅ **已完成**（未做磁盘实测，见 §4②） |
| **P2** 菜单栏 + 设置面板 | 顶部面板：File(Info/Quit) / Settings / 主题切换 / Byte mode / 面板显隐互斥。把 `config.rs` 的持久化字段接上真正的设置 UI | ✅ **已完成**（见 §3.1；G 组中「默认文件名」「启动检查更新」两项只有设置项，等 P4/P6 消费） |
| **P3** 接收链路补齐 | `max_log_string_length` FIFO 淘汰（**三镜像同步淘汰，`ColoredSpan` 需支持部分切分**）；`auto_scroll` 开关；可选时间戳前缀 | 🟡 **部分完成**：FIFO 淘汰与 `auto_scroll` 已提前做完；时间戳前缀未做 |
| **P4** 实时文件日志 | Start/Stop Log 开关 + 默认文件名 `LogFile_%Y-%m-%d_%H-%M-%S.txt`（chrono 现成）；**写入必须走独立线程 + `BufWriter`，绝不在 UI 线程 `write_all`** | ✅ **已完成**（见 §3.2；落在 `src/log_file.rs` 而非计划里的 `src/panels/file_log.rs`，另加了输出目录选择与同名防截断） |
| **P5** 实时图表 | 建议引 `egui_plot` 而非移植上游手绘代码；支持逗号分隔多通道；`stream_buffer` 处理跨包分片 | ✅ **已完成**（见 §3.3；落在 `src/chart.rs` + `src/panels/chart.rs`，多通道上限 16、绘制点封顶 4000、数据不足时断线而不补 0） |
| **P6** 收尾 | Info 弹窗；版本检查（**默认关闭或 3s 超时静默失败**，内网要能跳过）；应用图标；`[profile.release] opt-level = 2`；清理未使用依赖 | 🟡 **Info 弹窗 ✅**；**依赖清理 ✅**（移除 `tokio`、`egui-chinese-font`）；**release profile ✅**（`opt-level = 2`）；**应用图标 ✅**（代码绘制、11 测试）。**版本检查仍未实现** —— 下一步（待用户决策是否引入网络依赖 / 具体 URL） |

**覆盖率参考**：改动前相对 `SerialGUI-rs` 约 40%（核心链路约 90%）；P0/P1 完成后全局约 45%；
P2 完成约 75%（可日常使用）；P4 完成约 80%；**P5 完成约 88%（功能对齐上游）**。

**P3 的收尾项**：计划里的「可选时间戳前缀」仍未做（FIFO 淘汰与 `auto_scroll` 已在 P2 期间提前完成）。
它与 P6 都可以作为下一步；P6 里的「版本检查」需要联外网，内网环境建议做成默认关闭。

---

## 7. 硬性约定（改代码前务必遵守）

来自 `AGENTS.md` 与 `DEVELOPMENT_GUIDE.md`，这里只列最容易踩的：

0. **⛔ 未经项目所有者逐次批准，禁止运行本项目（`cargo run` / 直接跑 exe），禁止以任何方式
   占用或打断 COM3。** 开发阶段只用 `cargo fmt` / `cargo check --all-targets` / `cargo test`
   验证（90 条测试均不需要串口硬件）。需要实机运行时停下来请求批准，由所有者自行启动。
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

# 3. 建立门禁基线（应为 0 warning / 79 passed / fmt 干净）
cargo fmt && cargo check --all-targets && cargo test

# 4. 看清两个版本的差异
git diff master ai-refactor --stat

# 5. 读这三份文档再动手
#    AGENTS.md / DEVELOPMENT_GUIDE.md / REFACTOR_REPORT.md

# 6. 建议的第一件活：P6 收尾（见 §6）、或 P3 剩下的时间戳前缀；GPLv3 合规已降为最低优先级（§4①）
```

如果你要对照原版行为：`git -C E:\Downloads\Zed\comtool-rs log -1`（原版停在 `04dc007`），
或在本仓库 `git checkout master`。
