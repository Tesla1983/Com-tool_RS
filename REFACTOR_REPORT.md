# comtool-rs 重构执行报告

**执行日期**：2026-09-21
**对应计划**：`FEATURE_GAP_AND_PLAN.md`
**本次范围**：计划中的 §6（合规）、§5（架构拆分）、§7 阶段 P1（持久化）

> ⚠️ **本文是 P0/P1 阶段的历史快照**，其中的测试数（19）、clippy 警告数（9）、
> 模块结构与当前 HEAD 已不一致。要看当前状态请读 `HANDOVER.md`（并以其为准）。

---

## 0. 结果

| 任务 | 状态 | 验证方式 |
|---|---|---|
| 基线测试 | ✅ 已完成并记录 | `cargo check` / `cargo test` / `cargo fmt` |
| ① 通信层独立重写（脱离 GPLv3 衍生关系） | ✅ 代码完成，**合规结论待法律确认** | 结构/命名/并发机制全部更换；新增 6 条契约测试 |
| ② `app.rs` 纯移动式拆分 | ✅ 完成 | 归一化逐行比对脚本，29/29 代码单元等价 |
| ③ serde 状态持久化 | ✅ 完成 | 6 条测试覆盖正常/缺字段/损坏存档/运行时状态排除 |

**质量门禁**：`cargo check --all-targets` 0 error / 0 warning；`cargo test` **19 passed / 0 failed**；`cargo fmt --check` 干净。

---

## 1. 基线快照（改动前）

| 检查项 | 结果 |
|---|---|
| 工具链 | `rustc 1.97.1` / `cargo 1.97.1`，无 `rust-toolchain` 固定文件，edition 2024 |
| `cargo check --all-targets` | ✅ 0 error / **12 warning**（11 处 unused import + 1 处 dead_code） |
| `cargo test` | ✅ **7 passed** / 0 failed |
| `cargo fmt --check` | ❌ 2 处差异（`app.rs:253`、`app.rs:322` 行尾残留空白） |
| clippy | 仓库未配置，基线未测量 |

> 注意：仓库在本次改动前已存在**已暂存未提交**的改动（`src/app.rs`、`src/main.rs`，内容为删除若干中文注释）。本次所有修改都建立在该暂存状态之上，未覆盖它。

---

## 2. 任务 ① — 通信层独立重写

### 2.1 变更的文件

| 操作 | 文件 | 说明 |
|---|---|---|
| 重写 | `src/communicationtrait.rs` | 68 行。`CommunicationManager` → `ByteChannel`，`CommunicationEvent` → `ChannelEvent` |
| 新增 | `src/serial_impl/channel.rs` | 368 行。`SerialCommunication` → `SerialChannel`，含 6 条契约测试 |
| 新增 | `src/serial_impl/config.rs` | 46 行。`PortSettings` → `SerialConfig`；波特率表改为 `[u32; 20]` |
| 删除 | `src/serial_impl/serial.rs` | 223 行（可从 git 恢复） |
| 删除 | `src/serial_impl/portsettings.rs` | 124 行（可从 git 恢复） |
| 重写 | `src/serial_impl/mod.rs` | 13 行，作为模块唯一公开面 |

### 2.2 与参考实现的结构性差异

| 维度 | 参考实现 | 本实现 |
|---|---|---|
| 状态表示 | `Arc<Mutex<EPortState>>`（三态枚举，每次循环加锁读取） | `Arc<AtomicU8>`（两态，仅用于汇报，不参与控制流） |
| 停机机制 | 置共享标志位 → 后台线程轮询发现 → `join` | 向命令通道投递 `Halt` → 后台线程必然在本轮读到 |
| 发送指令 | 独立的 unbounded 通道，`write_all` 一次写完 | 同一个命令通道 + `VecDeque` 待发队列，增量推进 |
| 状态归位 | 在若干 `return` 路径手工置位（有遗漏） | `Drop` 守卫统一收口，不可能遗漏 |
| 死循环防护 | 每轮固定 `sleep(1ms)` 节流 | 仅在驱动返回 `Ok(0)` 时让出 1 ms |
| 事件枚举 | `DataReceived` / `ConnectionClosed` / `Error` | `Bytes` / `Closed` / `Failed` |
| 端口枚举 | 返回系统原始顺序 | 排序后返回（修 `COM10` 排在 `COM2` 前） |
| 端口诊断输出 | 向 stdout 打印 VID/PID/厂商等 | 移除；这些信息不该混进调试工具的日志 |

接口划分、并发模型、内部数据结构均为重新设计；未沿用原实现的函数体骨架。

### 2.3 有意的行为变更（功能性改进，非等价重构）

| # | 变更 | 理由 |
|---|---|---|
| 1 | 写超时不再静默丢弃剩余字节 | `timeout()` 同时作用于读写。设备侧流控拉低时 `write_all` 会中途返回 `TimedOut`，而它不报告已写字节数，原实现直接丢弃剩余部分——对串口工具而言是数据丢失。现改为按剩余长度增量推进 |
| 2 | 后台线程退出后状态必然归零 | 原实现只在部分路径置位，可能导致 UI 永久显示「已连接」 |
| 3 | `close()` 幂等；`open()` 已活动时返回 `AlreadyExists` | 契约明确化，避免重复 open 产生两个线程 |
| 4 | `close()` 遇后台线程 panic 返回 `Err` 而非 `unwrap()` panic | 如实上报，不假装关闭成功 |
| 5 | 读缓冲 1024 → 4096 字节 | 减少系统调用次数；3 Mbaud 下留十倍余量 |
| 6 | 打开失败的错误信息带上端口名 | 便于定位是哪个设备出问题 |
| 7 | `scan_ports()` 结果排序 | 修正 Windows 上的端口序号排序 |

### 2.4 合规结论（**必须由你决策**）

**已做的**：消除了与 `SerialGUI-rs`（GPLv3）之间逐字相同的表达——包括 `CommunicationManager` trait 的完整声明、`SerialCommunication` 的结构与函数体骨架、`BAUD_RATES` 的 `BaudRate { string_repr, numeric_repr }` 双字段表表达、以及 `get_available_connections()` 中逐行的 VID/PID 打印。`src/` 目录下已不存在与上游逐字相同的代码段。

**未做也无法由我做的**：法律意义上的 clean-room 要求「重写者从未接触过原实现」，由规格出发独立编写。我作为读过原实现的同一方，只能做到**独立结构设计 + 独立命名 + 不使用上游的非平凡代码结构**，这显著降低了衍生风险，但不构成法律上的排除证明。

**建议的收口动作（二选一）**：
1. 让一位未读过 `SerialGUI-rs` 的作者仅依据 `src/communicationtrait.rs` 的文档注释与需求重新实现一遍 `channel.rs`；
2. 若决定接受 GPLv3，则补齐 `LICENSE`（GPLv3 全文）+ `NOTICE`，并在 `Cargo.toml` 加 `license = "GPL-3.0-only"`。

**当前仓库仍是无许可证状态**：根目录无 `LICENSE`，`Cargo.toml` 无 `license` 字段。

---

## 3. 任务 ② — `app.rs` 纯移动式拆分

### 3.1 拆分结果

```
拆分前：src/app.rs  1172 行（单文件）

拆分后：
  src/app.rs        434 行   状态定义 + Default + 串口操作 + eframe::App 编排
  src/receive.rs    634 行   三镜像接收状态 + AnsiDecoder + 字节可视化 + 三种渲染 + 7 条测试
  src/sidebar.rs    240 行   右侧边栏 + SidebarAction
  src/config.rs     227 行   持久化策略 + 6 条测试（原为空文件）
```

### 3.2 等价性验证方法

写了独立脚本 `verify_pure_move.py`（位于本工作区，非项目内），以 **git index 版 `app.rs`** 为基线：

1. 对每个被搬走的代码单元按花括号配平截出完整块；
2. 归一化：剔除注释行、空行、缩进、属性行；应用显式列出的预期改名映射；
3. 断言基线代码块归一化后，作为**连续子序列**出现在目标文件中。

### 3.3 验证结果

```
receive.rs  20/20 单元 OK（utf8_complete_prefix_len、AnsiDecoder、ascii_spans_from_bytes、
                           layout_job_from、show_receive_scroll、show_plain_receive、
                           receive_data、build_layout、render_* 、mod tests …）
sidebar.rs   1/1  单元 OK（show_sidebar 主体 161 行逐行一致）
app.rs       8/8  单元 OK（refresh_ports、auto_reconnect、export_receive_text、
                           open_port、close_port、send_data、struct ComTool、Default）
RESULT: 全部代码单元逐行等价 ✅
```

**唯一的非等价点**：侧边栏的 `&mut sidebar_open` 改为方法入参 `open`——这是把闭包提取为方法所必需的结构调整，语义完全一致（调用方仍以局部变量传入并回写）。

> 该脚本已调用于本次会话的每一轮验证。它已沉淀为可复用技能
> `~/.workbuddy/skills/refactor-equivalence-proof/`（含通用版脚本与使用说明），
> 日后任何「拆分 / 搬动 / 提取方法」类重构都可直接复用；通用版脚本已用本项目的
> 配置实测通过，复现同样的 29/29 结果。

---

## 4. 任务 ③ — serde 状态持久化

### 4.1 实现

| 位置 | 内容 |
|---|---|
| `Cargo.toml` | `serialport` 启用 `serde` feature |
| `src/config.rs` | `STORAGE_KEY` / `load` / `save` / `restore_runtime_state` + 测试 |
| `src/app.rs` | `ComTool` 与 `Language` 派生 `Serialize`/`Deserialize`；容器级 `#[serde(default)]`；`ComTool::new(cc)` |
| `src/main.rs` | 入口由 `ComTool::default()` 改为 `ComTool::new(cc)` |

`serialport 4.10.0`（`"4.7.2"` 是 caret 要求，实际解析到 4.10.0）通过可选依赖提供 `serde` feature，`DataBits`/`StopBits`/`Parity`/`FlowControl` 均已有 `cfg_attr(feature = "serde", derive(...))`，因此**无需手写枚举与字符串之间的转换层**。

### 4.2 持久化策略

**落盘**：`selected_port`、`baud_rate`、`data_bits`、`stop_bits`、`parity`、`flow_control`、`send_text`、`hex_view`、`color_view`、`ascii_view`、`sidebar_open`、`language`、`last_opened_port`

**不落盘**（`#[serde(skip)]`，启动时重建）：

| 字段 | 理由 |
|---|---|
| `serial`、`event_rx`、`connected`、`last_reconnect_attempt` | 句柄活不过进程边界，连接必须显式重建 |
| `ports` | 沿用旧快照会显示已拔掉的设备，必须按当前系统重新枚举 |
| `receive_text`、`display_spans`、`ascii_text`、`ascii_spans`、`decoder` | 目前无长度上限，而 eframe 会周期性自动保存——落盘等于把无界字符串反复写盘。**先做长度上限（计划 P3），再考虑恢复日志** |
| `show_settings` | 模态窗口不该在启动瞬间自动弹出 |

**兼容性**：容器级 `#[serde(default)]` 保证以后新增字段时旧存档仍可读；`get_value` 在解码失败时返回 `None`，退化为默认值而不是 panic。

### 4.3 测试覆盖

用内存版 `eframe::Storage` 走**与生产完全相同**的路径（`save` → `eframe::set_value` → RON 编码 → `eframe::get_value` → RON 解码 → `load`），未新增测试依赖：

- `preferences_survive_round_trip` — 13 个偏好字段全部往返一致
- `runtime_state_is_never_restored` — 连接状态/句柄/日志/端口快照均不被带回来
- `archive_missing_new_fields_still_loads` — 手写「旧版本」存档（只含 2 字段）可正常读取
- `corrupt_archive_falls_back_to_default` — 损坏存档退化为默认值，不 panic
- `missing_archive_falls_back_to_default` — 首次启动无存档可正常工作
- `default_state_is_usable`

### 4.4 存档位置

`eframe` 会按 `run_native` 的标题自动创建存储（无需设置 `persistence_path`）：

```
Windows: %APPDATA%\ComTool-RS\data\app.ron
```

### 4.5 ⚠️ 未执行的验证

**磁盘级往返冒烟测试没有运行**。它是 GUI 程序，`save()` 只在退出或周期性自动保存时触发，无法在不打开窗口的情况下验证。

请手动执行：

1. `cargo run --release`
2. 改波特率为 `921600`、勾选「字节查看」、把侧边栏收起
3. 正常关闭窗口（不要用任务管理器强杀——强杀可能跳过 `save()`）
4. 确认 `%APPDATA%\ComTool-RS\data\app.ron` 已生成
5. 再次 `cargo run --release`，确认上述三项被恢复

---

## 5. 未完成项与残余风险

### 5.1 本次**有意未做**

| 项 | 原因 |
|---|---|
| `connected` 假阳性（`open_port()` 乐观置位，真正打开失败要等 Error 事件） | 属行为变更，需单独决策：是让 `open()` 同步确认打开结果（UI 线程会短暂阻塞），还是引入 `Opening` 中间态供 UI 呈现 |
| 接收缓冲长度上限（计划 P3） | 不在本次三项范围内；且它是「能否持久化接收日志」的前置条件 |
| 自动重连指数退避 | 不在范围内 |
| 3 个未使用依赖 `chrono` / `tokio` / `egui-chinese-font` | `tokio` 启用 `full` 会明显增加编译时间。保留是因为它们对应计划中的「日志文件名时间戳」与「版本检查」，是否移除应由你决定 |
| 9 条 clippy 警告 | 全部位于**逐字搬动的既有代码**中（`derivable_impls`×2、`manual_range_contains`×3、`collapsible_if`、`redundant_borrow`、`unnecessary_map_or`、`explicit_deref`）。本次新增的 `communicationtrait.rs` / `channel.rs` / `config.rs` / `sidebar.rs` / `receive.rs` **零 clippy 警告**。修它们会破坏「纯移动」的等价性，留待计划 P2 一并处理 |
| GPLv3 决策 | 见 §2.4，需你拍板 |

### 5.2 已知残余风险

1. **合规结论未闭环**——见 §2.4。
2. **`outgoing` 待发队列无上限**：设备侧长时间不可写、用户又持续点发送时队列会增长。原实现用 unbounded mpsc 通道，同样无上限，因此未引入新问题；但后续应加背压。
3. **写入路径的瞬态错误仍被静默忽略**：`WouldBlock`/`Interrupted` 直接重试是合理的；但如果设备长期不接受数据，用户不会得到任何提示。建议后续在侧边栏加「待发字节数」显示。

### 5.3 仓库状态提醒

```
 M Cargo.lock                        （新增 serde feature 引起）
 M Cargo.toml
MM src/app.rs                        （你已暂存的注释删除 + 本次改动）
 M src/communicationtrait.rs
 M src/config.rs
 M src/main.rs                       （你已暂存的注释删除 + 本次改动）
 M src/serial_impl/mod.rs
 D src/serial_impl/portsettings.rs   （可从 git 恢复）
 D src/serial_impl/serial.rs         （可从 git 恢复）
?? src/receive.rs
?? src/sidebar.rs
?? src/serial_impl/channel.rs
?? src/serial_impl/config.rs
?? FEATURE_GAP_AND_PLAN.md
```

⚠️ 本次改动**全部处于未暂存状态**。`git checkout .` / `git stash` / `git restore` 会丢弃它们。建议先 `git add` 或提交到一个分支。
