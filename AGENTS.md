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

### 2. 禁止以任何方式占用或打断 COM3

- 禁止打开、连接、读写、配置、探测 COM3——包括但不限于 `serialport` 相关命令、
  `mode COM3`、`echo > COM3`、Python `pyserial`、PowerShell `[System.IO.Ports.SerialPort]`、
  第三方串口助手、任何自写脚本。
- 禁止在默认值、配置文件、测试、示例、脚本、命令参数里把 **COM3** 作为目标端口。
  本项目的端口必须由用户在 UI 中手动选择。
- 禁止停止、重启、杀掉、卸载或禁用任何可能持有 COM3 的进程、服务、驱动；
  禁止改动 COM3 相关的设备管理器设置或注册表项。
- 对 COM3 的原则是：**只能"看"，不能"碰"**。确有必要时，最多允许不打开端口的只读端口名
  枚举，且必须先说明理由；绝不允许把它当作"验证收发"的手段。
- 发现 COM3 被占用或行为异常时，**只报告现象，不要自行干预**。

### 3. 默认允许的操作

`cargo fmt` / `cargo check --all-targets` / `cargo test` / `cargo clippy` / `cargo build`
等**不启动 GUI、不触碰串口**的静态检查与测试命令。本项目的 230 条测试全部不需要串口硬件，
所有验收一律以这些命令为准。

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
| 测试 | `python3 .workbuddy/hooks/test_guard.py`（39 条断言：命中/放行/批准三态，不启动程序、不碰串口）。**该脚本已在版本控制中**——它是仓库里唯一被 `.gitignore` 例外放行的 `.workbuddy/` 文件 |
| 批准 | `bash ~/.workbuddy/hooks/comtool-approve.sh 30`（**只能由用户本人在 agent 之外执行**，签 30 分钟有效期；`0` 立即撤销） |
| 批准文件 | `<配置目录>/combot-run-approval`（第一行 = 过期 Unix 秒；不存在/过期/损坏 → 全部拒绝） |

要点：

- **批准只放行"运行本项目"**；COM3 / 串口操作**永远拒绝**，不受批准影响。
- **批准文件与批准脚本，AI 无权触碰**——`combot-run-approval` / `comtool-approve` 一旦出现在命令或文件路径里就会被拦。
- 守卫是 **fail-closed**：payload 解析失败、stdin 为空，一律拒绝，绝不静默放行。
- 已知的"误拦"（都是设计如此，改用别的工具即可）：命令文本里出现 `cargo run`、`COM3`、`comtool-rs.exe` 等字面量就会被拦——例如 `grep "cargo run" AGENTS.md` 会被拒，请改用 Grep 工具；把这句话写进 commit message 也会被拒，可先用 Write 生成消息文件再 `git commit -F`。

### 6. 人工/机器可读摘要（供英文工具链解析）

**Do NOT run this project (`cargo run`, the built binary, or any equivalent) and do NOT
open, occupy, reconfigure, or interrupt the COM3 serial port without the user's explicit,
per-instance approval in the current conversation. Static checks only
(`cargo fmt` / `check` / `test` / `clippy` / `build`) are allowed by default.
This rule overrides everything else in this document.**

Enforcement: a `PreToolUse` hook (`~/.workbuddy/hooks/comtool-run-guard.sh`, registered in
`~/.workbuddy/settings.json` and `~/.codebuddy/settings.json`) *denies the tool call* when a
command or file path matches those patterns. It is fail-closed. The only escape hatch is a
time-boxed grant file that the human must create outside the agent:
`bash ~/.workbuddy/hooks/comtool-approve.sh 30`. Serial-port (COM3) operations are always
denied regardless of any grant. Run the guard's own tests with
`python3 .workbuddy/hooks/test_guard.py` (no program start, no port access). That test script
**is** version-controlled (the single `.workbuddy/` file allowed through by `.gitignore`), so a
clone can read both the rule and how to test it — but the guard and grant scripts themselves
live outside the repository in the user-level hooks directory and must be recreated on a new
machine, or the rules stay on paper while the enforcement silently disappears.

---

- This is a single Rust 2024 binary crate (`comtool-rs`); the executable entrypoint is `src/main.rs`.
- Module map:
  - `src/app.rs` — `ComTool` state, `Default`, connection lifecycle (`open_connection` / `close_connection` / `teardown_channel`, branching on `ConnectionKind`), the `ThemeChoice` / `Language` / `ConnectionKind` preference enums, view-level helpers (`visible_panel_count`, `apply_theme`), the `eframe::App` top-level orchestration, and (F5) the control-line entry point (`apply_line_state`), the send-byte counter allocation in `transmit`, port hot-plug (`rescan_ports` / `note_focus`), and the preset row renderer (`render_send_presets`).
  - `src/receive.rs` — the receive pipeline: mirrored state, `AnsiDecoder`, byte-level ASCII visualization, the optional timestamp prefix (`TimestampPrefix` / `format_timestamp` / `timestamped_spans`), display-mode exclusivity (`resolve_display_mode`), layout building, the three view renderers, and the `max_log_chars` FIFO trimming (`trim_text_front` / `trim_spans_front`). `ChannelEvent::Notice` is handled here too: it writes a note **without** touching `connected` or stopping repeat-send.
  - `src/send.rs` — send-side pure logic: `parse_hex` / `parse_escapes` (returning `EscItem::{Literal, Bytes}`), `build_payload`, `SendFormat` (text / HEX / escape), `SendHistory` / `HistoryNav`, and (F5) `SendPresets` + `preset_label`. No `egui` and no serial dependency, so it is unit-testable as-is.
  - `src/log_file.rs` — real-time file logging: the `LogSink` writer thread, file-name template rendering, same-name collision handling, the bounded write queue, and the `impl ComTool` entry points (`start_logging` / `stop_logging` / `log_append` / `poll_log_events`).
  - `src/chart.rs` — the real-time chart's data layer: parsing the raw byte stream into multi-channel numeric series (`ChartData`), plus `impl ComTool { clear_chart }`. Deliberately free of any `egui` dependency so it is testable on its own.
  - `src/app_icon.rs` — the application icon, generated at compile time / run time from code (rounded square + digital pulse waveform, 256×256, supersampled). It has no external asset file, so it is fully covered by unit tests and needs no GUI to verify.
  - `src/sidebar.rs` — the right-hand sidebar; returns a `SidebarAction` instead of mutating state while drawing. Holds the connection-kind radio plus the serial-vs-network parameter fork, and (F5) the send-byte counter line, the free-form baud rate field, and the RTS/DTR control-line row.
  - `src/strings.rs` — the UI string table (`Strings` + `ZH` / `EN`, both columns in one `strings!` macro arm, so a missing translation is unwriteable), `Language::strings()`, and `fill()` for `{}` templates.
  - `src/panels/{menu_bar,settings,info,chart}.rs` — the top menu bar (`MenuAction`), the settings window, the about window, and the chart panel (`egui_plot` rendering + `points_for_drawing` downsampling). All inherent `impl ComTool` methods, so nothing is re-exported from `src/panels/mod.rs`.
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
- "HEX receive" and "byte view" are **mutually exclusive**, and the rule has exactly one implementation: `ComTool::resolve_display_mode` (`receive.rs`). Both UI entry points (the receive-settings row in `app.rs`, `panels/menu_bar.rs::display_mode_checkboxes`) and the startup repair in `config::restore_runtime_state` call it — do not inline `if hex { ascii = false }` at a call site, or the menu and the main window will eventually disagree. Why they must not coexist: the HEX branch only writes `display_spans`, while the byte view renders `ascii_spans`, so enabling both leaves the receive area **blank**. The arguments mean "which one the user just ticked on", so the function applies that click and clears the other; do **not** change it to "HEX always wins" — that would silently undo a click on "byte view" (looks like a dead checkbox). With `(false, false)` it repairs stale archives, letting HEX win in line with the branch order below.
- Receive mirror routing must match the mode branch order exactly: HEX wins over byte view (`if self.hex_view { .. } else if self.ascii_view { .. }`), so the mirror target is `!self.hex_view && self.ascii_view`, not `self.ascii_view`. Now that the UI forbids the combination, this is defence in depth (hand-edited archives, future callers bypassing the UI); locked by `receive::tests::hex_takes_precedence_over_ascii_like_before`.
- Every append to a receive mirror must also reach the real-time file log: use `push_receive_note` (mirror + log) or `push_str` followed by `log_append` (log only). Never call `self.receive_text.push_str` directly in `receive_data` without logging — that silently drops data from the log file. Note that `&self.event_rx` keeps `self` borrowed, so collect notices into a local `Vec<String>` first and apply them after the drain loop.
- Real-time file logging writes on its own thread (`log_file.rs::worker_main`); the UI thread must never `write` to the log file. Stopping sends `Halt`, and `Drop` must use `try_send` — `send` blocks when the queue is full, so a stuck disk could hang the UI thread forever. The file is opened by the UI thread and then handed to the thread, so "start logging" can report failure immediately; do not switch to the optimistic style used by the serial channel (its `open()` only spawns a worker, so a failure surfaces later as a `Failed` event). Existing files are never truncated: `create_log_file` adds a `_1`/`_2` suffix instead. Log file names are rendered from a user-supplied `chrono` template and must go through `render_log_file_name`, which falls back to the default template on invalid specifiers (`DateTime::format` panics on them) and replaces path separators with `_`.
- Logging state has a single source of truth: `log.is_some()` (query via `is_logging()` / `log_path()` / `log_written_bytes()`). The log file is *not* bounded by `max_log_chars`, and "clear receive" does not truncate it — both only affect in-memory mirrors.
- The real-time chart is **not** a fourth receive mirror. `receive_data` feeds it the **raw bytes**, before the per-mode branches run, so switching to HEX / byte view does not freeze the curves; conversely `ChartData` has its own parsing rules, its own limits, and its own clear entry point (`clear_chart`). Do not move the feed call inside one of the mode branches.
- Chart input is one group per line, fields comma separated (`\n` or `\r\n` ends a group). Field *k* always maps to channel *k*, and all fields of one group share the same x — the **group counter**, not a timestamp (serial data has no reliable time base). A line with no numeric field produces no sample **and does not advance x**, so text noise cannot punch holes in the curves.
- Never make the chart invent data: missing channels get no sample for that group (the curve just breaks — do not pad with zeros), and an unterminated comma stream is not re-grouped into fixed-width chunks (the channel count would be a guess, and a wrong guess draws the data misaligned). A half line exceeding `MAX_PENDING_BYTES` is discarded **as a whole line**: the rest of it keeps being dropped until a newline restores alignment, otherwise the truncated tail would become a fake sample.
- `ChartData` holds at most `MAX_CHANNELS` (16) channels and `chart_max_samples` points per channel, evicting the oldest — same FIFO idea as `max_log_chars`, but counted in points. Channel count only ever grows, derived from the widest line seen. The entry point clamps the limit itself so a hand-edited archive with `0` cannot make the chart look broken.
- Chart drawing is capped: `panels/chart.rs::points_for_drawing` downsamples to `MAX_DRAWN_POINTS` (4 000) per channel and **must keep the last point**, which is the newest data — losing the tail looks like "the device stopped sending". Curve colours come from a fixed palette (`CHANNEL_COLORS`, cycling per channel index): they encode *which channel*, so they are intentionally theme-independent.
- There is exactly one way to establish a connection: `ComTool::open_connection()`, used by both the UI button and `auto_reconnect`. It branches on `connection_kind` to build a serial / TCP / UDP channel, but `configure` → `open` → adopt the event channel is written **once** — do not clone that sequence per transport (a missing event channel means "connected but no data"; a missing `configure` means "the settings never take effect"). Preconditions are decided solely by `connection_ready()`, the wording comes from `channel_descriptor()`, and when they are not met the function must do **nothing** — never set `connected` optimistically.
- `channel` and `channel_kind` live and die together: the reuse rule is "**reuse in place when the transport kind is unchanged, tear down and rebuild when it changed**" (the guard reads `channel.is_none() || channel_kind != Some(connection_kind)`), and `teardown_channel()` clears both. That rule serves two things: serial reconnect is "open the same handle again", and the send-path tests depend on the injected `FakeChannel` *not* being torn down. Reverting it to "always build fresh" compiles fine but silently turns those tests into ones that talk to a real serial port.
- Serial-only controls must fork on the connection kind: port / baud rate / data bits / stop bits / parity / flow control are **not drawn at all** in TCP / UDP mode (grey-out is not enough — it implies they still apply), the available-ports list is hidden too, and the read-only info block becomes "target + local port". The connection-kind radio is **locked while connected**: switching it would leave the shown parameters describing a channel other than the open one, while the button reads "disconnect" and would tear down a different transport. Button labels and actions must always agree and follow the kind ("Open port / Close port", "Connect / Disconnect", "Bind / Disconnect").
- UDP must never be described as "connected": it has no connection, and saying "connected" implies the peer is online and that a drop would be noticed. Use "bound / not bound" (`status_bound` / `status_unbound`). Related: UDP's `Ok(0)` is a zero-length datagram (valid traffic), whereas TCP's `Ok(0)` means the peer closed and must emit `ChannelEvent::Closed`. The bind address is `0.0.0.0:local_port` (not loopback), and because `local_port` may be 0 the actual port is exposed through `ByteChannel::local_addr()` (a trait method with a default of `None`, overridden only by UDP) and shown in the sidebar — the user has to know which port the device should reply to.
- Notices written into the receive log name the transport via `ComTool::transport_noun()` ("串口" / "TCP" / "UDP"). The notice text itself is deliberately **not** translated (see the string-table rule in `DEVELOPMENT_GUIDE.md` §6), but a TCP drop must not claim the serial port closed.
- Transport-specific ignorable errors stay in that transport, not in the shared helper: `worker::is_transient` lists only the three kinds every transport shares (`TimedOut` / `WouldBlock` / `Interrupted`). Windows reports ICMP port-unreachable on a connected UDP socket as `ConnectionReset`, so `udp.rs` adds that case itself — putting it in the shared function would make serial ignore a real fault.
- Control lines (RTS / DTR, F5): opening a port must **never** touch them. `open_connection()` only syncs the `rts` / `dtr` checkboxes to `false` and sends no `SetLineState` command — `serialport` on Windows already leaves both lines disabled on open (`windows/dcb.rs`), and pulling DTR high at open time resets a large class of dev boards while the user only wanted to look at the port. The checkbox means "**the level we requested**", not a measurement: `serialport` only offers writes, never reads, so the UI cannot know where the hardware actually drove the lines. Say that in the tooltip, the comments and the wording alike.
- Control-line failures are **not** disconnects: `ChannelEvent::Notice` is the non-fatal channel event. `receive_data` writes a note for it and deliberately leaves `connected` alone and keeps repeat-send running, because data keeps flowing while a line toggle may have failed. Use `Notice` for new "failed but not fatal" channel reports instead of relaxing `Failed`; `Failed` / `Closed` still mean the channel is gone and stop repeat-send.
- `ByteChannel::set_line_state(rts: Option<bool>, dtr: Option<bool>)` uses `Option` (not two bools) because the driver can only write: `None` means "leave that line alone". It has a default impl returning `Unsupported` (same precedent as `configure`); only `SerialChannel` overrides it, and TCP / UDP just ignore `WorkerCommand::SetLineState` in their command loops.
- There is exactly one outbound path: `payload_for()` (encoding + line ending) → `transmit()` (write + error reporting + `tx_bytes` accounting). Every sender — the send button, the Enter key, repeat-send, and preset buttons — goes through it. Never write your own `channel.write` in a callback: you will silently lose either the failure notice or the byte count. `tx_bytes` is accumulated only **after** `write` returns `Ok` (same principle as "clear the input only if it was really sent"), and it resets on teardown.
- Presets (`SendPresets`, F5) deliberately do **not** evict when full — the opposite of `SendHistory`. History is collected automatically, so dropping the oldest is expected; presets were picked by hand, and silently removing the first one looks like data loss. `can_add()` is the single decision point (both the greyed-out button and `add()` call it). Labels are derived from the content by `preset_label()` (first line, truncated by **characters**) rather than stored.
- New drawing logic should be a `fn xxx(&mut self, ui: &mut Ui)` method (e.g. `render_send_presets`), not inline in `eframe::App::ui` — `ui()` needs an `eframe::Frame` that headless tests cannot build, so anything written there is never drawn by `cargo test`. Keep pure drawing side-effect free and assert it: `panels::tests::connected_sidebar_renders_without_panic` checks that drawing the sidebar issues **no** `set_line_state` call at all.
- The receive area's default text colour **must come from the theme** (`ui.visuals().text_color()`), never a hard-coded black. All three render paths (`show_plain_receive`, `render_color_receive`, `render_ascii_receive`) take it from there; hard-coding `Color32::BLACK` makes the whole log **invisible on a dark theme**. What keeps this from breaking ANSI support is `layout_job_from`'s `span.style.fg.unwrap_or(default_color)`: only bytes the device actually coloured override it. Those explicit colours (`ASCII_CTRL_COLOR`, `ASCII_NONPRINT_COLOR`, `TIMESTAMP_COLOR`) are data encoding, not theme decoration — do not "fix" them to follow the theme. Smoke tests know nothing about colour (a hard-coded black passes them happily), so colour is pinned by asserting `LayoutJob.sections[].format.color` directly.
- Panel visibility has an invariant: at least one of `sidebar_open` / `show_receive_panel` / `show_chart_panel` / `show_send_panel` must stay visible (`visible_panel_count() >= 1`). The view menu disables un-checking the last visible panel, and `restore_runtime_state` repairs hand-edited archives. A new panel must be added to `visible_panel_count`, the view menu, and that repair.
- Drawing order in `ComTool::ui` is load-bearing: top menu bar → right sidebar → `CentralPanel` (must be last), and inside the central panel the order is receive → chart → send. Panels claim space in the order they are added.
- Theme preference is persisted as the crate's own `ThemeChoice`, never as `egui::ThemePreference` (a foreign enum whose variants can change). `apply_theme` pushes it to the context only when it differs from the active value.
- `SerialConfig` defaults to 115200 baud, 8 data bits, no flow control/parity, one stop bit. It is passed to the channel via `ByteChannel::configure` before `open`.
- Persistence: `ComTool` derives `Serialize`/`Deserialize` with a container-level `#[serde(default)]`. Only user preferences are stored; serial handles, the port list, connection state, the receive log, `ChartData`, and the running `LogSink` are `#[serde(skip)]` and rebuilt by `config::restore_runtime_state` on load. Add new runtime-only fields to both the `skip` list and that function. Removing a persisted field is safe (unknown keys are ignored — keep `config::tests::archive_with_a_removed_field_still_loads` green, it is what stops anyone from adding `deny_unknown_fields`), and a new preference field must be added to `Default`, to `customized()` in the config tests, and to that round-trip assertion.
- Do not ship a preference that nothing consumes (a checkbox that changes nothing is worse than a missing feature). P6 deliberately drops the version check, and `check_update_on_start` was removed together with its settings section rather than left in the UI as a dead switch.
- Tests are inline: parser/renderer/trimming/timestamp tests at the bottom of `src/receive.rs`, chart parsing/buffer/limit tests at the bottom of `src/chart.rs`, logging/format/naming/disk tests at the bottom of `src/log_file.rs` (they use a private temp-dir helper, no extra crate), channel contract tests at the bottom of `src/serial_impl/channel.rs`, persistence and load-invariant tests at the bottom of `src/config.rs`, headless UI smoke tests (menu bar, settings/about windows, all three receive views, the sidebar, the *connected* sidebar, the preset row, the chart panel) at the bottom of `src/panels/mod.rs`, icon geometry/color/alpha tests at the bottom of `src/app_icon.rs`. Run them with `cargo test` (or e.g. `cargo test config::tests::` for one module) — none of them require serial hardware, and the UI smoke tests use `egui::__run_test_ui` so they need no window either. `receive::tests::feed` injects bytes through the event channel, which is the same path a real port takes (no hardware needed). The network channels are covered at the bottom of `src/net_impl/{config,tcp,udp}.rs` — two of them talk over loopback (`TcpListener` / `UdpSocket`), so they need no hardware and no GUI either. `app.rs`'s send-path tests inject a `FakeChannel` through `connected_app_as(kind)`. A test may call `scan_ports()` (plain enumeration, no port is opened) — `config.rs` and `serial_impl/channel.rs` already do.
- Verification commands: `cargo check`, then `cargo test`; use `cargo fmt --check` to check formatting. `cargo clippy --all-targets` is available but not gated; it currently reports 8 pre-existing style warnings, all in code carried over unchanged.
- The chart draws with `egui_plot`. Its version number is **not** in lockstep with egui: `egui_plot 0.37` needs `egui ^0.36` whereas `egui_plot 0.36` needs `egui ^0.35`. Before bumping either crate, check the actual requirement (`cargo tree -i egui`, or the local `registry/src/**/egui_plot-*/Cargo.toml`) instead of assuming the numbers match.
- `DEVELOPMENT_GUIDE.md` records the verified state/default/UI wiring conventions; consult it for changes that add persistent `ComTool` state, and initialize every new field in `Default`.
- Repository-local OpenCode agents are configured in `opencode.json` and `.opencode/agents/`; the executor workflow expects code changes followed by `cargo check`/`cargo test`.
- Licensing: `SerialGUI-rs/` (GPLv3) is a reference implementation only and is gitignored. The serial modules were independently re-implemented to avoid derivation; do not copy code back from that directory.
