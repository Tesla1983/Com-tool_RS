# comtool-rs 开发参考文档

> 本项目是一个基于 `eframe`/`egui` 的串口调试工具。本文档描述当前项目的代码组织方式、功能开发的完整流程，以及各层职责划分，作为新增功能的参考。

---

## 1. 项目架构概览

本项目采用 **egui 即时模式（immediate mode）UI**，代码组织在一个核心结构体 `ComTool` 上，共分四层：

| 层 | 位置 | 职责 | 典型例子 |
|---|---|---|---|
| **状态层** | `struct ComTool` | 定义功能所需的跨帧/跨调用状态字段 | `connected`、`display_spans`、`color_view` |
| **初始化层** | `impl Default for ComTool` | 给所有状态字段设默认值（漏了会编译报错） | `color_view: false`、`baud_rate: 115200` |
| **业务逻辑层** | `impl ComTool` | 串口读写、数据解析、状态修改、布局构建 | `open_port()`、`receive_data()`、`build_layout()` |
| **UI 层** | `impl eframe::App for ComTool` | 绘制控件并把 UI 事件接到业务方法上 | `ui.button("发送").clicked()` |

关键文件：`src/app.rs`（核心逻辑 + UI）、`src/serial.rs`（串口扫描）、`src/main.rs`（入口）。

---

## 2. 数据流总览

```
串口接收线程 --(mpsc channel)--> ComTool.rx
        │
        ▼
receive_data()      ← 业务层：拉取原始字节
   ├─ HEX 模式   → 逐字节转文本 → push 进 display_spans
   └─ 文本模式   → decoder.feed() 解析 ANSI 颜色 → push 进 display_spans
        │
        ▼
display_spans: Vec<ColoredSpan>   ← 状态层：彩色片段缓存（内存）
        │
        ▼
build_layout(display_spans)       ← 业务层：转成 egui 的 LayoutJob
        │
        ▼
egui::Label::new(job).wrap()      ← UI 层：渲染多色彩文本
```

> 同一份接收数据维护两份镜像：`display_spans`（彩色片段）喂彩色模式，`receive_text`（纯字符串）喂普通 `TextEdit` 模式。因此「清空」时两者都要 `clear()`。

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
│ ② 在 struct ComTool 里加状态字段                           │
│      如：pub rx_byte_count: u64,                          │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ③ 在 impl Default for ComTool 里给新字段设默认值           │
│      如：rx_byte_count: 0,                                 │
│      ⚠ 漏了这步 → 编译报错（E0061 字段未初始化）             │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ④ 在 impl ComTool 里写业务逻辑方法                         │
│      （数据处理 / 串口操作 / 状态修改 / 布局构建）            │
└────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────┐
│ ⑤ 在 impl eframe::App for ComTool 的 ui() 里添加控件       │
│      （按钮 / 滚动区 / TextEdit / ComboBox …）              │
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
│ ⑦ cargo check 验证编译，确认无 error                      │
└────────────────────────────────────────────────────────────┘
```

---

## 4. 实例演示：添加「接收字节计数」

走一遍上面的流程，假设要实现"统计并显示已接收字节数"：

```rust
// ① 需要跨帧显示计数 → 需要状态字段

// ② struct ComTool 中加字段
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

// ④ impl ComTool 中写业务逻辑：在接收数据时累加
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

// ⑦ cargo check
```

---

## 5. 现有功能与代码位置的对应关系

| 功能 | 状态字段 | 业务方法 | UI 入口 |
|---|---|---|---|
| 打开/关闭串口 | `connected`、`rx`、`tx`、`stop_tx` 等 | `open_port()` / `close_port()` | 「打开串口」/「关闭串口」按钮 |
| 接收数据显示 | `display_spans`、`receive_text`、`decoder` | `receive_data()`、`build_layout()` | 接收区滚动区 + 颜色勾选框 |
| 彩色 / HEX 切换 | `color_view`、`hex_view` | `render_color_receive()` | 「颜色」「HEX/文本」控件 |
| 清空接收 | （复用现有字段） | 无独立方法，逻辑写在按钮内 | 「清空」按钮 |
| 导出接收数据 | `receive_text` | `export_receive_text()` | 「导出」按钮 |
| 发送数据 | `send_text`、`tx` | `send_data()` | 「发送」按钮 |

---

## 6. 开发注意事项

1. **不是所有功能都要加字段**：只有需要"跨帧/跨调用共享"的数据才进结构体。纯计算/纯展示（如格式化字符串）直接写方法或局部变量即可，避免污染状态。

2. **新增字段务必同步 `impl Default`**：egui 结构体大多通过 `Default::default()` 创建，漏初始化会直接编译失败。

3. **UI 事件与业务逻辑分离**：尽量把数据处理写在 `impl ComTool` 里，`ui()` 只负责画控件和调用方法，保持界面层清爽。

4. **渲染用即时数据更新**：`ui()` 开头调用 `self.receive_data()` 拉取最新数据（或由 `render_color_receive` 内部调用），配合 `request_repaint_after` 实现持续刷新。

5. **一次性提示不要每帧追加**：类似"串口已打开/已关闭"这类提示，应在事件发生（如 `open_port`/`close_port`）时追加到 `display_spans`，而不是在渲染函数里按 `self.connected` 每帧写入，避免日志里不断重复。

6. **两个滚动分支共用 `SCROLL_AREA_ID`**：彩色模式与文本模式的 `ScrollArea` 使用同一个 `id_salt`，切换模式时滚动位置保持延续；「清空」按钮也用它重置滚动状态。
