# Com-tool_RS

一个用 Rust 重写的串口 / 网络调试上位机（GUI 助手），基于 `egui` / `eframe`。

> **许可声明**：本项目是 [Opentronika/SerialGUI-rs](https://github.com/Opentronika/SerialGUI-rs) 的**修改版（derivative work）**，
> 按 GPL-3.0-only 分发。逐字重合度实测、第三方依赖许可与分发检查清单见 [`NOTICE.md`](NOTICE.md)。

## 功能

**连接**

- 三种传输：`串口` / `TCP 客户端` / `UDP`
- 完整串口参数：波特率、数据位、停止位、校验、流控（无 / 软件 / 硬件）
- **任意波特率手输**（1 ~ 12,000,000），不限于预设档位
- DTR / RTS **手动控制**；打开端口时**不会**自动拉线（不会意外复位开发板）
- 连接过程有明确的中间态（打开中），不是"点了没反应"

**接收与显示**

- 四种视图：文本 / HEX / ANSI 颜色 / 字节
- **接收区筛选**（显示层过滤，导出与日志始终是完整原文）
- **强制换行**：超过 200 个字符的行自动折行（可在「视图」菜单关闭）
- **实时图表**（`egui_plot`）
- 字符编码可选 UTF-8 / GBK / GB18030 等，`encoding_rs` **增量解码**，跨包不丢半个汉字

**发送**

- 文本 / HEX 发送，**周期发送**（10 ms ~ 600 s）
- **发送文件**：分块读取 + 分块入队，遇背压保留当前块、不静默丢数据；可实时取消
- 状态栏显示待发队列字节数，长时间停滞时变色警示

**其它**

- 文件记录：文件名模板、同名自动防截断；记录过程中改动配置会提示"下次生效"
- 15 种界面语言：简体中文、English、繁體中文、日本語、Español、Italiano、Português、
  Bahasa Indonesia、Français、Deutsch、한국어、Tiếng Việt、Türkçe、Русский、Polski
- 窗口大小记忆、主题切换、界面字号缩放

## 构建

需要 Rust **1.95 或更高**（`Cargo.toml` 的 `rust-version`；下限由 `egui` / `eframe` / `egui_plot`
自己声明的 1.95 决定，不是本项目的代码需要）与 **2024 edition**。

```bash
cargo build --release
```

产物在 `target/release/`。

Release 档刻意使用 `opt-level = 2`（见 `Cargo.toml` 注释）：这是每帧都要跑布局与文本排版的 GUI 程序，
热点在串口 IO 与布局而非算术，低一档优化对体感几乎无影响，但能明显缩短构建时间。

跑测试与静态检查：

```bash
cargo test                      # 401 passed + 1 ignored
cargo clippy --all-targets      # 0 warning
cargo fmt --check
```

这四项在 GitHub Actions 上自动跑（`.github/workflows/ci.yml`，`windows-latest`；另有
一个钉在 1.95 的 MSRV job）。401 条测试全部不需要真实串口硬件。

## 项目状态

自动化测试覆盖较完整（**401 条测试**，全部无需真实串口硬件，替身 + 无头渲染驱动 UI），
但**实机交互尚待人工验证**——待验项集中在 [`HANDOVER.md`](HANDOVER.md) §4② 的目视清单里，
绝大多数条目仍未在真实设备上跑过。坦白标注在此，请以实际使用结果为准。

## 文档地图

| 文档 | 内容 |
|---|---|
| [`NOTICE.md`](NOTICE.md) | 派生声明、重合度实测、第三方依赖许可、分发检查清单 |
| [`AGENTS.md`](AGENTS.md) | 面向 AI 协作的项目红线与模块地图（含运行约束、串口端口约束） |
| [`HANDOVER.md`](HANDOVER.md) | 交接总纲：待人工验证清单、已知缺陷、仓库状态 |
| [`DEVELOPMENT_GUIDE.md`](DEVELOPMENT_GUIDE.md) | 逐条开发约定（66 条）+ egui 绘制硬事实附录 |
| [`HISTORY_2026.md`](HISTORY_2026.md) | 各阶段设计取舍与偏离记录 |
| [`FEATURE_GAP_AND_PLAN.md`](FEATURE_GAP_AND_PLAN.md) | 与上位的逐项差距 |
| [`GAP_ANALYSIS_VS_COMTOOL.md`](GAP_ANALYSIS_VS_COMTOOL.md) | 与 `Neutree/COMTool`（Python 版）的对照 |
| [`FUTURE_ROADMAP.md`](FUTURE_ROADMAP.md) | 路线图与未立项项 |
| [`TEST_PORT_COM4.md`](TEST_PORT_COM4.md) | 用 COM4 做实机验证的可复现步骤与已知陷阱 |
| `I18N_*.md` | 多语种扩展的风险分析与目标语言分档 |

## 工具脚本

`tools/` 下有三个辅助脚本：

- `license_overlap_check.py` — 与上游的逐字重合度复核
- `backpressure_check.py` — 背压行为验证
- `backpressure_dummy_sender.py` — 背压测试的假数据发送端
