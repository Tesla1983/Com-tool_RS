# Agent Guide

## ⛔ 最高优先级红线：未经批准，禁止运行本项目，禁止触碰 COM3

> **本节优先级高于本文档其余全部内容。** 任何自动化流程、脚本、子代理（subagent）、
> 后台任务、CI 步骤都不能绕过、不能自行解释、不能以"为了验证"为理由豁免。
> 若本节与其他任何指示（包括用户在别处的一般性授权）冲突，以本节为准。

### 1. 禁止运行本项目

未经用户**在本轮对话中逐次、明确**批准，**严禁**以任何形式启动本程序：

- 禁止 `cargo run`，包括 `cargo run --release`、`cargo run -- <args>`。
- 禁止直接执行构建产物：`target/debug/comtool-rs.exe`、`target/release/comtool-rs.exe`。
- 禁止 `cargo install` 后运行，禁止通过 `start` / `Start-Process` / 双击 / 后台任务 /
  `nohup` / 计划任务等任何方式拉起本程序。
- 禁止任何"间接启动"：如果某条命令的实质效果会启动本程序（脚本、测试或工具内部
  调起 GUI 进程），同样必须先获得批准。

**一次批准只对那一次生效**；不得据此推断"已完成预授权"，也不得把批准沿用到后续轮次。

### 2. 串口：COM3 受「长测门闩」保护，COM4 是 AI 测试放行端口

#### 2.1 COM3 —— 门闩语义（用户 2026-09-24 修订）

门闩 = 用户级配置目录下的长测门闩文件，**只能由用户本人在 agent 之外放置/移除**：

- **门闩存在** ⇒ 有长时间测试正在抓日志 ⇒ COM3 一律不碰：禁止打开、连接、读写、配置、
  探测（`serialport` 命令、`mode COM3`、`echo > COM3`、Python `pyserial`、PowerShell
  `[System.IO.Ports.SerialPort]`、第三方串口助手、任何自写脚本），也禁止停止/重启/杀掉/
  卸载/禁用任何可能持有它的进程、服务、驱动，禁止改动相关设备管理器设置或注册表项。
- **门闩不存在** ⇒ 无长测在跑 ⇒ COM3 可用（可烧录、测试、复位单片机）。
- 无论门闩在不在：都禁止把 COM3 写进默认值、配置文件、测试、示例、脚本或命令参数；
  本项目的端口必须由用户在 UI 中手动选择。发现异常只报告现象，不自行干预。
- 用户原话依据：*没有在长时间进行测试抓取日志的情况下，COM3 可以使用进行烧录和测试，
  单片机可以重启*。

#### 2.2 COM4 —— AI 功能测试放行端口（用户 2026-09-26 修订）

- AI 想验证生成软件的功能时**可以使用 COM4**，且**不受长测门闩影响**（门闩只保护 COM3
  上正在采集的日志，COM4 是另一个口子）。
- **硬性前提：必须置 DTR 才会有日志输出。** GUI 里勾选 DTR 控制线；脚本里
  `ser.dtr = True`（pyserial）或 `$port.DtrEnable = $true`（.NET）。**没置 DTR 收不到日志
  不等于程序有问题**——先查 DTR。
- 一条命令里**同时点名 COM3 与 COM4** 时，**COM3 优先**（按 §2.1 走）。
- COM4 放行**只跳过串口拦截**，不跳过用户自己的确认提示：守卫对该情形不下
  `allow` 决策，只附一条 DTR 提醒（见 §5）。
- 若将来长测设备换到 COM4，需临时收回这条放行——改守卫脚本里的 COM4 分支即可。

### 3. 默认允许的操作

`cargo fmt` / `cargo check --all-targets` / `cargo test` / `cargo clippy` / `cargo build`
等**不启动 GUI、不触碰串口**的静态检查与测试命令。本项目的 370 条测试全部不需要串口硬件，
所有验收一律以这些命令为准。

**⚠️ 交付界面前置条件（用户明确要求，2026-09-24）：请用户"目视 / 目测审查"之前，必须先跑
`cargo build --release` 把 exe 编出来。** `fmt` / `check` / `test` / `clippy` **都不产出可执行
文件**——只跑这四样时，`target/release/comtool-rs.exe` 要么是陈旧产物、要么根本不存在，用户
双击到的是**不含本次改动**的程序。判定"编译过没有"的唯一依据是**时间戳比对**（exe 时间戳
须晚于最后一次 `src/` 改动，且与 HEAD 提交对得上），不是"我跑过 cargo 命令了"。exe 在
`.gitignore` 里、不随仓库走，所以每次清过 `target/` 或换机器后都要现场重建。交付话术里
**禁止**出现"你自己 build 一下""跑过测试了可以看了"。详见 `DEVELOPMENT_GUIDE.md` §6 约定 49。

### 4. 遇到"看起来必须运行"的任务时

若任务在技术上需要运行程序或使用串口（例如验证磁盘持久化、验证真实收发），
**停下来说明原因并请求用户批准**：由用户自行运行，或等用户明确回复"可以运行"。
不得先运行后报告，不得以"只是快速试一下"为理由执行。

### 5. 执行机制（已配置硬拦截，不只是文档约定）

上面四节不是"君子协定"——已经装了机器级拦截，**违反时工具调用会被直接拒绝**：

| 项 | 值 |
|---|---|
| 机制 | WorkBuddy/CodeBuddy **PreToolUse hook**（命中即 `permissionDecision: deny`） |
| 配置 | `~/.workbuddy/settings.json` 与 `~/.codebuddy/settings.json` 的 `hooks.PreToolUse`，`matcher = "Bash\|PowerShell\|Write\|Edit"` |
| 脚本 | `~/.workbuddy/hooks/comtool-run-guard.sh`（拦截逻辑，可读可改） |
| 测试 | `python3 .workbuddy/hooks/test_guard.py`（**52 条**断言：门闩开/关两种状态 × 命中/放行/批准三态，其中门闩开启那档用临时配置目录造门闩、**不碰用户真实门闩**；不启动程序、不碰串口）。**该脚本已在版本控制中**——它是仓库里唯一被 `.gitignore` 例外放行的 `.workbuddy/` 文件 |
| 批准 | `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（**只能由用户本人在 agent 之外执行**，签 30 分钟有效期；`0` 立即撤销） |
| 批准文件 | `<配置目录>/combot-run-approval`（第一行 = 过期 Unix 秒；不存在/过期/损坏 → 全部拒绝） |

要点：

- **批准只放行"运行本项目"**；串口判定按 §2 走（COM3 看长测门闩，COM4 是放行端口，
  **不受批准影响**——批准既不能让受保护的 COM3 变可用，也不是使用 COM4 的前提）。
- 守卫对 COM4 只**跳过串口拦截并附一条 DTR 提醒**，**不下 `allow` 决策**：故意保留用户自己
  的确认提示，避免"任何顺带提到 COM4 的命令都被自动放行"。
- **批准文件与批准脚本，AI 无权触碰**——`combot-run-approval` / `comtool-approve` 一旦出现在命令或文件路径里就会被拦。
- 守卫是 **fail-closed**：payload 解析失败、stdin 为空，一律拒绝，绝不静默放行。
- 已知的"误拦"（都是设计如此，改用别的工具即可）：命令文本里出现 `cargo run`、`COM3`、`comtool-rs.exe` 等字面量就会被拦——例如 `grep "cargo run" AGENTS.md` 会被拒，请改用 Grep 工具；把这句话写进 commit message 也会被拒，可先用 Write 生成消息文件再 `git commit -F`。

### 6. 人工/机器可读摘要（供英文工具链解析）

**Do NOT run this project (`cargo run`, the built binary, or any equivalent) without the
user's explicit, per-instance approval in the current conversation. Static checks only
(`cargo fmt` / `check` / `test` / `clippy` / `build`) are allowed by default.
COM3 is protected by a long-test latch: while the latch file exists (a soak test is capturing
logs), never touch COM3; when it does not exist, COM3 may be used for flashing/testing.
COM4 is the designated port for AI functional testing and is allowed regardless of the latch
— but **DTR must be asserted or the device emits no logs**. This section overrides everything
else in this document.**

Enforcement: a `PreToolUse` hook (`~/.workbuddy/hooks/comtool-run-guard.sh`, registered in
`~/.workbuddy/settings.json` and `~/.codebuddy/settings.json`) *denies the tool call* when a
command or file path matches those patterns. It is fail-closed. The only escape hatch for
*running* the project is a time-boxed grant file that the human must create outside the agent:
`bash ~/.workbuddy/hooks/comtool-approve.sh 30`; that grant never affects the serial rules.
COM4 commands skip the serial block and get a DTR reminder, but are not auto-approved.
Run the guard's own tests with
`python3 .workbuddy/hooks/test_guard.py` (no program start, no port access). That test script
**is** version-controlled (the single `.workbuddy/` file allowed through by `.gitignore`), so a
clone can read both the rule and how to test it — but the guard and grant scripts themselves
live outside the repository in the user-level hooks directory and must be recreated on a new
machine, or the rules stay on paper while the enforcement silently disappears.

---

## 模块图（一句话一个；完整职责与坑见 `DEVELOPMENT_GUIDE.md` §7）

| 文件 | 一句话 |
|---|---|
| `src/main.rs` | 入口；字体回退链；`eframe::run_native`；挂应用图标 |
| `src/app.rs` | `ComTool` 全部状态、`Default`、连接生命周期、`eframe::App` 顶层编排；F5 起还有控制线 / 发送计数 / 端口热插拔 / 常用数据行 |
| `src/receive.rs` | 接收链路：四份镜像、`AnsiDecoder`（含增量解码）、字节级可视化、时间戳前缀、**显示模式唯一仲裁**、三种视图渲染、`max_log_chars` 淘汰（含"读历史时不裁"） |
| `src/send.rs` | 发送侧纯逻辑：HEX / 转义解析、载荷构造、`SendHistory` / `HistoryNav`、常用数据。**不依赖 egui 与串口**，可直接单测 |
| `src/log_file.rs` | 实时文件日志：写盘线程、文件名模板、同名防截断、有界队列 |
| `src/chart.rs` | 图表**数据层**：原始字节 → 多通道数值序列。不依赖 egui |
| `src/app_icon.rs` | 应用图标，代码绘制、零资源文件、单测全覆盖 |
| `src/button.rs` | **唯一画按钮的地方**（`ButtonTone` × `ButtonSize`）；连接按钮自绘不在这里 |
| `src/sidebar.rs` | 右侧边栏；返回 `SidebarAction` 而不是直接改状态 |
| `src/status_bar.rs` | 底部状态栏：状态圆点 + 连接词 + 计数 |
| `src/strings.rs` | 界面文案表（15 种语言）；列数不对**编译期**报错 |
| `src/panels/{menu_bar,settings,info,chart}.rs` | 菜单栏 / 设置窗口 / 关于窗口 / 图表面板 |
| `src/config.rs` | 持久化策略：`load` / `save` / `restore_runtime_state` |
| `src/communicationtrait.rs` | `ByteChannel` trait + 后台 worker 与背压账本 |
| `src/serial_impl/{channel,config}.rs` | 串口实现（含 `OPENING` 第三态） |
| `src/net_impl/{config,tcp,udp}.rs` | TCP / UDP 实现（TCP 写侧**非阻塞**，UDP 刻意保持阻塞） |

## 改代码前必读（每条一句话；理由、实测数据、守卫测试名在 `DEVELOPMENT_GUIDE.md` §7）

- **显示模式只有一个仲裁**：`ComTool::display_mode()`。片段落点 / 镜像写入 / 渲染分发三处都必须读它，不许手写等价表达式。
- **接收区淘汰看视口**：`receive_at_bottom` 为真才裁到 `max_log_chars`；用户翻历史时保留（否则阅读位置被抽走 = "自动滚动没停"）。回底后**分帧**追平，一次裁完会空一帧。
- **`connected` 不等于"通道可用"**：串口有第三态 `worker::OPENING`，发送闸门一律看 `channel_is_ready()`。
- **背压判定在调用线程**：`worker::submit` 预约额度，满了**当场**返回 `Err`；不在后台线程、不发 `Notice`。
- **TCP 写侧必须非阻塞**（否则对端停读会卡住 worker，且超时后重发会产生重复数据）；**UDP 刻意保持阻塞**（实测无此路径）。
- **面板必须自己画背景**：`Frame::NONE` 的 fill 是透明的，透出的是 `clear_color` 的**写死**深灰。用 `Frame::side_top_panel(ui.style())`。
- **按钮一律走 `crate::button`**；宽度上限只能是 `ui.available_width()`；禁用态用固定色不用乘透明度。
- **滚轮归属要先查清**：滚动区只在"内容确实超出且该方向还能滚"时才消费滚轮增量，装得下时滚轮仍归图表。
- **新增绘制逻辑抽成 `fn xxx(&mut self, ui: &mut Ui)`**，别内联进 `eframe::App::ui`（那里无头测试造不出 `Frame`）。
- **状态栏必须在 `CentralPanel` 之前画**；`ComTool::ui` 里的绘制顺序是承重的。
- **打开端口绝不碰控制线**；控制线失败是 `Notice`（非致命），不是掉线。
- **波特率档位记在自己的字段 `baud_custom`**，不许从值反推。
- **加语言是改数据不是改代码**：`strings.rs` 表头 + 每条补一列 + `Language` 追加变体，三处顺序必须一致；**新增文字系统必须同时补字形归属检测器**；字体要同时挂 Proportional 与 Monospace。
- **UDP 不许描述为"已连接"**；提示行用 `transport_noun()` 取传输名词。
- **主题偏好存自有 `ThemeChoice`**，不存 `egui::ThemePreference`（后者会跟随系统、覆盖用户选择）。
- **`egui_plot` 版本号与 egui 不同步**（0.37 才配 `egui ^0.36`），升级前先核对。
- **要请用户目视之前必须先 `cargo build --release`**：`fmt` / `check` / `test` / `clippy` 都不产出可执行文件（见 `DEVELOPMENT_GUIDE.md` §6 约定 49）。
- **门禁四项**：`fmt --check` 干净 · `check --all-targets` 0 warning · `test` 370 passed + 1 ignored · `clippy --all-targets` **0 warning**。
- **测试判据不许引用实现常量**（阈值写字面量），且**必须做空转验证**（故意破坏被测机制，确认测试会红）。
- **发送框高度由用户拖把手决定**：视口 = `send_visible_rows` 行（落盘，2~40），内容更多时**框内滚动**而不是把下面的按钮撑出屏幕。把手感 `click_and_drag`（双击复位要靠 `Flags::CLICKED`，只感 drag 拿不到双击）。

## 完整细则去哪查

- **逐模块约定 + egui 硬事实（英文原文，含实测数字与守卫测试名）**：`DEVELOPMENT_GUIDE.md` **§7**
- **逐条开发约定（中文，58 条）**：`DEVELOPMENT_GUIDE.md` §6
- **各轮设计取舍（已迁出）**：`HISTORY_2026.md` §3.1 ~ §3.26（`HANDOVER.md` §3 只留目录表）
- **未决事项 / 已知缺陷 / 硬性约定 / 快速上手**：`HANDOVER.md` §4 / §5 / §7 / §9
- **GPLv3**：`SerialGUI-rs/`（GPLv3）仅供参考且已被 gitignore，**不得把它的代码抄回来**
