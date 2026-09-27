# NOTICE — comtool-rs

本文件是**分发时必须随附**的声明（GPLv3 第 5 节要求保留版权与许可声明、并对被修改的版本
给出显著提示）。仓库根目录的 `LICENSE` 是 GNU GPLv3 全文（FSF 官方文本，未改动；
LF 行尾、35 148 字节、sha256 `8b1ba204bb69a0ade2bfcf65ef294a920f6bb361b317dba43c7ef29d96332b9b`）。

> 那个 sha256 是**可自证"许可没被改动"的手段**，所以 `.gitattributes` 里钉了
> `LICENSE text eol=lf`——否则在 Windows 上（`core.autocrlf=true`）检出会变成 CRLF、
> 字节数与校验和当场对不上。核对命令：
> `sha256sum LICENSE`（GNU/Linux）或 `certutil -hashfile LICENSE SHA256`（Windows）。

## 1. 许可

本作品以 **GNU General Public License v3.0（GPL-3.0-only）** 授权。
`Cargo.toml` 的 `license = "GPL-3.0-only"` 与之一致。

用一句话说清这意味着什么：**GPLv3 允许你把本程序（包括二进制）对外分发、允许收费、
允许商用**；它要求的是——分发时随附完整对应源码、保留本文件与 `LICENSE`、并把你做的
修改同样置于 GPLv3 之下。它不允许的是"把衍生作品闭源分发"。

> ⚠️ **为什么是 GPLv3 而不是 MIT/Apache**：本作品是 **SerialGUI-rs** 的衍生作品（见第 2 节），
> 而 SerialGUI-rs 是 **GPLv3 单一许可**——它自己的 `NOTICE.md` 明写
> "licensed under the GNU General Public License v3.0 (GPLv3)"，其 git 历史里也只跟踪过
> `LICENSE-GPLv3`（它的 `Cargo.toml` 里那句 `include = ["LICENSE-APACHE", …]` 是**残留**：
> 仓库里从来没有那个文件，历史里也没有）。衍生作品的许可由原作品的许可决定，**换个
> `LICENSE` 文件并不能把这一层关系切断**。

## 2. 本版本是被修改过的版本（GPLv3 §5(a)）

- **上游**：`SerialGUI-rs` v1.4.0 — <https://github.com/Opentronika/SerialGUI-rs>
  （Rust + egui 串口助手，作者 Lutgaru，GPLv3）。
- **本作品**：`comtool-rs`（仓库名 `AI_comtool-rs`），是上述项目的**修改版**，
  自 **2026-09-21** 起由本仓库所有者大幅重构与扩充。
- **主要修改**（逐轮细节见 `HISTORY_2026.md`、`HANDOVER.md` §3 目录）：
  模块拆分（`communicationtrait` + `serial_impl` / `net_impl`）；新增 TCP / UDP 客户端；
  实时文件日志（独立写线程 + BufWriter）；实时图表（`egui_plot`）；发送侧补齐（HEX / 转义 /
  行尾 / 定时发送 / 发送历史 / 常用数据）；任意波特率与控制线 RTS/DTR；多语言 15 种；
  接收区筛选；主题 / 字体档位；面板布局重构；370 条自动化测试。
- 上游的版权声明与许可**原样保留**（本文件第 1 节与 `LICENSE`）。

## 3. 与上游的代码重合度：实测结果

**这一节是给分发者的技术依据，不是法律意见。** 测量脚本在
`tools/license_overlap_check.py`（可复现）：

```
python tools/license_overlap_check.py            # 需要本机存在参考副本 SerialGUI-rs/
```

口径：把上游 `src/**.rs` 与本地仓库每个 `rev` 的 `.rs` 逐行比较——去掉首尾空白、
丢弃长度 < 12 的行、丢弃注释行，取交集。

| 对照 | 结果 |
|---|---|
| 上游 `src/`（19 个 `.rs`） | 750 条去重后的实质代码行 |
| 当前 `HEAD`（25 个 `.rs`） | 与上游逐字相同 **106 条** |
| 最初提交 `ecfb6e8` | 与上游逐字相同 **11 条** |

**这 106 条是什么**（全量列举见脚本输出）：全部是**框架与库强制唯一的写法**，没有一条是
逻辑、算法或数据结构。例如：

```rust
fn save(&mut self, storage: &mut dyn eframe::Storage) {   // eframe::App 的签名，只能这么写
pub fn new(cc: &eframe::CreationContext<'_>) -> Self {     // 同上
use eframe::egui;                                          // 用这个 crate 就必须这么写
ui.separator();   ui.horizontal(|ui| {   ui.selectable_value(
Parity::None    StopBits::One    FlowControl::Hardware     // serialport 的枚举变体名
let (tx, rx) = mpsc::channel();   while let Ok(event) = rx.try_recv() {
pub parity: Parity,   pub stop_bits: StopBits,             // 与 serialport 参数一一对应的字段
```

也就是说：**当前代码在"表达"层面没有残留上游的实现**（这是本项目在
`HANDOVER.md` §4① 里早就记过的结论，本次用脚本把它量化了）。

⚠️ **但这不等于法律意义上的 clean-room**。clean-room 的定义是"重写者**从未接触过**原实现"，
它只能靠**过程**证明（谁写的、写之前看过什么），不能靠行比对证明。本项目的历史里，
最初的 `ecfb6e8`（"initialize ComTool project…"）与后续重构都由**读过上游的人/会话**完成，
所以按严格口径它仍然是衍生作品——这也是为什么本作品按 GPLv3 分发，而不是选一个更宽松的许可。

## 4. 第三方依赖与许可

直接依赖（版本取自 `Cargo.lock`，许可取自各 crate 的 `Cargo.toml`）：

| crate | 版本 | 许可 |
|---|---|---|
| `eframe` | 0.36.2 | MIT OR Apache-2.0 |
| `egui` | 0.36.2 | MIT OR Apache-2.0 |
| `egui_plot` | 0.37.0 | MIT OR Apache-2.0 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serialport` | 4.10.1 | MPL-2.0 |
| `chrono` | 0.4.45 | MIT OR Apache-2.0 |
| `encoding_rs` | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| `rfd` | 0.17.2 | MIT |

**全部与 GPLv3 兼容**（MIT / Apache-2.0 / BSD-3-Clause 是宽松许可；MPL-2.0 依其 §3.3
允许二次许可为 GPLv3——上游 `NOTICE.md` 对 `serialport-rs` 也是这么认定的）。
间接依赖见 `Cargo.lock`，可用 `cargo license` 之类的工具展开。

## 5. 分发检查清单

1. **不要**把参考副本 `SerialGUI-rs/` 打进分发包。它在 `.gitignore` 里，`git archive` /
   `git clone` 不会带上；但**手工打 zip 会**——打包脚本里请显式排除该目录。
2. **随附源码**：分发可执行文件时，必须同时提供**完整对应源码**（GPLv3 §6），
   并让接收者能拿到 `LICENSE` 与本文件。
3. **保留声明**：不要删改 `LICENSE`、本文件，以及源码里已有的版权/来源说明。
4. **改动要标注**：若再次修改后分发，按 §5(a) 补一句"本版本经过修改 + 日期"，
   改动清单可继续写在 `HISTORY_2026.md`。
5. 二进制里已内嵌的版本/构建信息（`app_icon`、信息窗口）无需额外处理。

## 6. 如果你想改用宽松许可（可选路线）

只有一条路：**真 clean-room 重写**。要点：

- 需要一个**从未读过 SerialGUI-rs** 的人（或一个全新的、上下文里没有上游代码的会话）
  依据需求与**本项目自己的**抽象（`src/communicationtrait.rs` 的文档注释）重写被认定
  有风险的模块。项目文档 `HANDOVER.md` §4① 早先估计范围为 `serial_impl/channel.rs`，
  约 1~2 天。
- 重写期间上游参考副本必须**不可见**（移出仓库树 / 换机器），并保留过程记录（谁、何时、
  依据什么），作为将来主张 clean-room 的证据。
- 重写完成且确认无上游表达残留后，才谈得上换 `LICENSE`（例如 MIT / Apache-2.0）。
- ⚠️ **本仓库的 AI 会话不能充当这个"重写者"**：它在核对许可时已经读过上游的
  `Cargo.toml` / `NOTICE.md` / `main.rs`，按 clean-room 的口径已经被污染。

在走上这条路之前，**GPLv3 已经允许你对外分发**（含收费与商用），并不构成阻碍。
