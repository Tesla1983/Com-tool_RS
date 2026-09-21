//! 接收链路：原始字节 → 可显示的彩色片段 → egui 布局。
//!
//! 一份接收数据在这里维护三份互为镜像的状态：
//!
//! - `receive_text`：纯文本，供普通视图渲染与导出；
//! - `display_spans`：带 ANSI 颜色信息的片段，供彩色视图渲染；
//! - `ascii_text` / `ascii_spans`：逐字节转义后的可见化文本，供字节查看视图渲染。
//!
//! 三者必须同步清空、同步追加——任何只动其中一份的改动都会让切换视图时
//! 看到不一致的内容。三者也都受 [`crate::app::ComTool::max_log_chars`] 约束：
//! 每轮接收结束后一起裁剪，从最旧的内容开始淘汰（见 [`ComTool::trim_receive_logs`]）。
//!
//! 追加时同一段文本会一并投递给实时文件日志（[`crate::log_file`]），所以磁盘上的
//! 日志文件与接收区看到的内容一致；但**裁剪与清空只作用于内存镜像**——「清空日志」
//! 不会去截断一个正在写入的文件，那是用户没有要求的破坏性操作。
//!
//! 图表（[`crate::chart`]）**不是第四份镜像**：它有自己的解析规则、自己的上限、
//! 自己的清空入口，只是恰好也从 `receive_data` 这里拿到同一批原始字节。
//!
//! 时间戳前缀（[`TimestampPrefix`]）是一层**显示注解**，不是收到的数据：它随文本
//! 一起进入三份镜像与实时文件日志（所以导出的文件也带时间戳），但**不会**进入喂给
//! 图表的那份原始字节——图表解析的必须是设备真正发过来的内容。默认关闭，且关闭时
//! 接收链路与没有这个功能时逐字节相同。
//!
//! 两种解码路径：
//!
//! - [`AnsiDecoder`]：解析 CSI/SGR 转义序列并跨数据块拼接 UTF-8，用于文本视图。
//!   遇到非法字节不生成 U+FFFD（字体缺字形时会渲染成不可读的 □），而是输出
//!   灰色 `[FF]` 标记。
//! - [`ascii_spans_from_bytes`]：完全不做字符集解释，逐字节转义，用于字节查看视图。

use crate::app::ComTool;
use crate::communicationtrait::ChannelEvent;
use chrono::Local;
use eframe::egui;
use egui::Ui;

/// 接收区滚动视图的持久化 ID。
///
/// 普通文本、彩色、字节查看三种模式共用同一个 ID：切换模式时滚动位置得以延续，
/// 「清空日志」也用它把滚动状态一并复位。
pub(crate) const SCROLL_AREA_ID: &str = "receive_scroll";

/// ASCII 视图：`\r` `\n` `\t` 等常见控制字符的转义显示颜色（蓝）。
const ASCII_CTRL_COLOR: egui::Color32 = egui::Color32::from_rgb(36, 114, 200);
/// ASCII 视图：其余不可见字节（其它控制字符 / 高位字节）的显示颜色（灰）。
const ASCII_NONPRINT_COLOR: egui::Color32 = egui::Color32::from_rgb(128, 128, 128);

/// 求 `bytes` 中「可以安全解码」的前缀长度，即把可能被截断的尾部多字节序列
/// 整体排除在外的分界点。
///
/// 串口是字节流，一次读到的数据块可能在多字节 UTF-8 序列中间断开。若直接
/// 解码，尾部半截序列会变成乱码或非法字节标记。这里从尾部回溯连续的
/// continuation 字节（`0b10xxxxxx`）找到 lead 字节，按其声明的宽度判断该序列
/// 是否完整；不完整就把从 lead 起的部分整体留给下一个数据块。
fn utf8_complete_prefix_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    if len == 0 {
        return 0;
    }

    // 找到尾部连续 continuation 字节（0b10xxxxxx）的起点 start：
    // 循环退出时 bytes[start-1] 是 lead 字节（若 start > 0）
    let mut start = len;
    while start > 0 && (bytes[start - 1] & 0xC0) == 0x80 {
        start -= 1;
    }
    if start == len {
        // 尾部不是未完成的序列
        return len;
    }
    if start == 0 {
        // 整段都是 continuation 字节（异常流）：无法回溯 lead，交给上层按非法字节标记
        return len;
    }
    let lead = bytes[start - 1];
    let width = if lead < 0x80 {
        1
    } else if lead >= 0xC0 && lead <= 0xDF {
        2
    } else if lead >= 0xE0 && lead <= 0xEF {
        3
    } else if lead >= 0xF0 && lead <= 0xF7 {
        4
    } else {
        1 // 非法起始字节，当作单字节处理（会被标记为 [XX]）
    };
    let seq_start = start - 1; // 未完成序列的 lead 字节位置
    if seq_start + width <= len {
        len
    } else {
        seq_start // 从 lead 字节起整个保留，等下一个数据块补齐
    }
}

/// 一段文本的显示样式。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AnsiStyle {
    pub fg: Option<egui::Color32>,
    pub bold: bool,
}

impl Default for AnsiStyle {
    fn default() -> Self {
        Self {
            fg: None,
            bold: false,
        }
    }
}

/// 带样式的文本片段。接收区的最小渲染单元。
pub struct ColoredSpan {
    pub text: String,
    pub style: AnsiStyle,
}

/// ANSI 标准 8 色（SGR 30–37）。
fn basic_color(n: u8) -> egui::Color32 {
    match n {
        30 => egui::Color32::from_rgb(0, 0, 0),
        31 => egui::Color32::from_rgb(205, 49, 49),
        32 => egui::Color32::from_rgb(13, 188, 121),
        33 => egui::Color32::from_rgb(229, 229, 16),
        34 => egui::Color32::from_rgb(36, 114, 200),
        35 => egui::Color32::from_rgb(188, 63, 188),
        36 => egui::Color32::from_rgb(17, 168, 205),
        37 => egui::Color32::from_rgb(229, 229, 229),
        _ => egui::Color32::WHITE,
    }
}

/// ANSI 亮色（SGR 90–97）。
fn bright_color(n: u8) -> egui::Color32 {
    match n {
        90 => egui::Color32::from_rgb(128, 128, 128),
        91 => egui::Color32::from_rgb(255, 0, 0),
        92 => egui::Color32::from_rgb(0, 255, 0),
        93 => egui::Color32::from_rgb(255, 255, 0),
        94 => egui::Color32::from_rgb(92, 92, 255),
        95 => egui::Color32::from_rgb(255, 0, 255),
        96 => egui::Color32::from_rgb(0, 255, 255),
        97 => egui::Color32::from_rgb(255, 255, 255),
        _ => egui::Color32::WHITE,
    }
}

/// ANSI 256 色表：0–15 复用基本色/亮色，16–231 是 6×6×6 色立方，
/// 232–255 是 24 级灰阶。
fn color256(n: u8) -> egui::Color32 {
    if n < 16 {
        if n < 8 {
            basic_color(30 + n)
        } else {
            bright_color(90 + (n - 8))
        }
    } else if n < 232 {
        let i = n as u32 - 16;
        let conv = |x: u32| -> u8 { if x == 0 { 0 } else { (55 + x * 40) as u8 } };
        egui::Color32::from_rgb(conv(i / 36), conv((i % 36) / 6), conv(i % 6))
    } else {
        let g = (8 + (n as u32 - 232) * 10) as u8;
        egui::Color32::from_rgb(g, g, g)
    }
}

/// 接收数据的文本视图解码器。
///
/// 持有两份跨数据块的残留：`pending_utf8` 保存被截断的多字节序列，
/// `pending_ansi` 保存尚未收齐的 CSI 转义序列。样式状态 `style` 同样跨块延续，
/// 因为一条 SGR 序列的影响范围是其后所有字节，而不是当前数据块。
pub struct AnsiDecoder {
    pending_ansi: Vec<u8>,
    pending_utf8: Vec<u8>,
    pub style: AnsiStyle,
}

impl Default for AnsiDecoder {
    fn default() -> Self {
        Self {
            pending_ansi: Vec::new(),
            pending_utf8: Vec::new(),
            style: AnsiStyle::default(),
        }
    }
}

impl AnsiDecoder {
    /// 丢弃全部残留状态与样式，回到初始态。「清空日志」时调用。
    pub fn reset(&mut self) {
        self.pending_ansi.clear();
        self.pending_utf8.clear();
        self.style = AnsiStyle::default();
    }

    /// 送入一个数据块，返回本次可确定输出的片段。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ColoredSpan> {
        let mut spans = Vec::new();
        for &b in bytes {
            if self.pending_ansi.is_empty() {
                if b == 0x1B {
                    // 遇到 ESC：先输出已积累的普通文本，再开始收集转义序列
                    self.flush_utf8(&mut spans);
                    self.pending_ansi.push(b);
                } else {
                    self.pending_utf8.push(b);
                }
            } else {
                // 正在收集 ESC 序列
                self.pending_ansi.push(b);
                if self.pending_ansi.len() == 2 {
                    // ESC 后的第一个字节：必须是 '[' 才构成 CSI 序列，否则忽略整段
                    if b != b'[' {
                        self.pending_ansi.clear();
                    }
                } else if (0x40..=0x7E).contains(&b) {
                    // CSI 参数之后的终结字节（'m' 为 SGR，其余忽略）
                    let seq = std::mem::take(&mut self.pending_ansi);
                    self.parse_ansi(&seq);
                }
            }
        }
        self.flush_utf8(&mut spans);
        spans
    }

    fn flush_utf8(&mut self, spans: &mut Vec<ColoredSpan>) {
        if self.pending_utf8.is_empty() {
            return;
        }
        let valid_len = utf8_complete_prefix_len(&self.pending_utf8);
        let (complete, tail) = self.pending_utf8.split_at(valid_len);
        if !complete.is_empty() {
            // 严格解码：合法片段原样输出；非法字节以灰色 [XX] 十六进制标记显示，
            // 不再静默生成 U+FFFD（字体缺字形时会渲染成不可读的 □）
            let bad_style = AnsiStyle {
                fg: Some(ASCII_NONPRINT_COLOR),
                bold: false,
            };
            let mut rest = complete;
            let mut normal = String::new();
            while !rest.is_empty() {
                match std::str::from_utf8(rest) {
                    Ok(s) => {
                        normal.push_str(s);
                        rest = &[];
                    }
                    Err(e) => {
                        let valid = e.valid_up_to();
                        if valid > 0 {
                            normal
                                .push_str(std::str::from_utf8(&rest[..valid]).unwrap_or_default());
                        }
                        // error_len()=None 表示末尾被截断的不完整序列，按剩余全部处理
                        let bad = e.error_len().unwrap_or(rest.len() - valid).max(1);
                        if !normal.is_empty() {
                            spans.push(ColoredSpan {
                                text: std::mem::take(&mut normal),
                                style: self.style,
                            });
                        }
                        let hex: String = rest[valid..valid + bad]
                            .iter()
                            .map(|b| format!("[{b:02X}]"))
                            .collect();
                        spans.push(ColoredSpan {
                            text: hex,
                            style: bad_style,
                        });
                        rest = &rest[valid + bad..];
                    }
                }
            }
            if !normal.is_empty() {
                spans.push(ColoredSpan {
                    text: normal,
                    style: self.style,
                });
            }
        }
        self.pending_utf8 = tail.to_vec();
    }

    fn parse_ansi(&mut self, seq: &[u8]) {
        if seq.len() < 3 || seq[0] != 0x1B || seq[1] != b'[' {
            return;
        }
        let final_byte = *seq.last().unwrap();
        if final_byte != b'm' {
            return;
        }
        let params_str = String::from_utf8_lossy(&seq[2..seq.len() - 1]);
        let params: Vec<u32> = params_str
            .split(';')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect();
        self.apply_sgr(&params);
    }

    fn apply_sgr(&mut self, params: &[u32]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.style = AnsiStyle::default(),
                1 => self.style.bold = true,
                2 | 22 => self.style.bold = false,
                39 => self.style.fg = None,
                30..=37 => self.style.fg = Some(basic_color(params[i] as u8)),
                90..=97 => self.style.fg = Some(bright_color(params[i] as u8)),
                38 => {
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        self.style.fg = Some(color256(params[i + 2] as u8));
                        i += 3;
                        continue;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        self.style.fg = Some(egui::Color32::from_rgb(
                            params[i + 2] as u8,
                            params[i + 3] as u8,
                            params[i + 4] as u8,
                        ));
                        i += 5;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// 把一段原始字节逐字节解析为 ASCII 可视化 spans：
/// - 0x20..=0x7E：可打印字符，原样显示（默认色）
/// - `\r` `\n` `\t`：显示为 `"\\r"` `"\\n"` `"\\t"`（蓝色；其中 `\n` 额外追加真实换行以保持行结构）
/// - 其它控制字符（0x00..=0x1F、0x7F）：显示为 `"[0D]"` 形式（灰色）
/// - 高位字节（0x80..=0xFF）：显示为 `"\xFF"` 形式（灰色）
fn ascii_spans_from_bytes(bytes: &[u8]) -> Vec<ColoredSpan> {
    fn flush(spans: &mut Vec<ColoredSpan>, normal: &mut String) {
        if !normal.is_empty() {
            spans.push(ColoredSpan {
                text: std::mem::take(normal),
                style: AnsiStyle::default(),
            });
        }
    }

    let mut spans = Vec::new();
    let mut normal = String::new();
    for &b in bytes {
        match b {
            0x20..=0x7E => normal.push(b as char),
            b'\n' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\n\n".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            b'\r' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\r".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            b'\t' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\t".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            0x00..=0x1F | 0x7F => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: format!("[{b:02X}]"),
                    style: AnsiStyle {
                        fg: Some(ASCII_NONPRINT_COLOR),
                        bold: false,
                    },
                });
            }
            _ => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: format!("\\x{b:02X}"),
                    style: AnsiStyle {
                        fg: Some(ASCII_NONPRINT_COLOR),
                        bold: false,
                    },
                });
            }
        }
    }
    flush(&mut spans, &mut normal);
    spans
}

/// 由一组 `ColoredSpan` 构造等宽 14px 的 `LayoutJob`（彩色/字节查看两种视图共用）。
fn layout_job_from(spans: &[ColoredSpan], default_color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in spans {
        job.append(
            span.text.as_str(),
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::monospace(14.0),
                color: span.style.fg.unwrap_or(default_color),
                ..Default::default()
            },
        );
    }
    job
}

/// 从头部删除多余字符，只保留最后 `max` 个字符。
///
/// 按**字符边界**切分（不是字节下标），因此不会把多字节 UTF-8 序列切成半个。
/// 快速路径用字节长度判断：字节数 ≤ 上限时字符数必然也 ≤ 上限，省掉一次全串扫描。
fn trim_text_front(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let total = text.chars().count();
    if total <= max {
        return;
    }
    let drop = total - max;
    let cut = text
        .char_indices()
        .nth(drop)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    text.drain(..cut);
}

/// 从头部淘汰片段，只保留最后 `max` 个字符对应的内容。
///
/// 保留下来的第一个片段可能需要**从中间切开**（它只有一部分落在窗口内），
/// 这是旧实现里 `ColoredSpan` 最容易被漏掉的地方：整片段删/留都不对。
fn trim_spans_front(spans: &mut Vec<ColoredSpan>, max: usize) {
    let total: usize = spans.iter().map(|span| span.text.chars().count()).sum();
    if total <= max {
        return;
    }

    // 先整片段地数过去，找到第一个需要保留的片段及其内部偏移
    let mut drop = total - max;
    let mut keep_from = 0;
    while keep_from < spans.len() {
        let len = spans[keep_from].text.chars().count();
        if len <= drop {
            drop -= len;
            keep_from += 1;
        } else {
            break;
        }
    }

    // total > max 保证了循环一定在中途 break，因此 keep_from < spans.len()
    let cut = spans[keep_from]
        .text
        .char_indices()
        .nth(drop)
        .map(|(idx, _)| idx)
        .unwrap_or(spans[keep_from].text.len());

    if keep_from > 0 {
        spans.drain(..keep_from);
    }
    if cut > 0 {
        spans[0].text.drain(..cut);
    }
}

/// 绘制接收区滚动视图（可选中、右键复制全部日志），返回是否点击了「复制全部日志」。
///
/// `stick_to_bottom` 由 [`crate::app::ComTool::auto_scroll`] 决定：关掉自动滚动后，
/// 用户向上翻阅历史时不会被强行拉回底部。
fn show_receive_scroll(ui: &mut Ui, job: egui::text::LayoutJob, stick_to_bottom: bool) -> bool {
    let mut copy_all = false;
    egui::ScrollArea::vertical()
        .stick_to_bottom(stick_to_bottom)
        .auto_shrink([false, false])
        .max_height(400.0)
        .id_salt(SCROLL_AREA_ID)
        .show(ui, |ui| {
            let response = ui.add(egui::Label::new(job).wrap().selectable(true));
            response.context_menu(|ui| {
                if ui.button("复制全部日志").clicked() {
                    copy_all = true;
                    ui.close();
                }
            });
        });
    copy_all
}

/// 普通文本模式接收区：不折行（Extend）+ 纵/横双向滚动，
/// 等宽 galley 绘制保证盲文 ASCII 图案整行对齐；可选中、右键复制全部日志。
pub(crate) fn show_plain_receive(ui: &mut Ui, text: &str, stick_to_bottom: bool) {
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::monospace(14.0),
        egui::Color32::BLACK,
        f32::INFINITY,
    );
    egui::ScrollArea::both()
        .stick_to_bottom(stick_to_bottom)
        .auto_shrink([false, false])
        .max_height(400.0)
        .id_salt(SCROLL_AREA_ID)
        .show(ui, |ui| {
            let response = ui.add(
                egui::Label::new(job)
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true),
            );
            response.context_menu(|ui| {
                if ui.button("复制全部日志").clicked() {
                    ui.ctx().copy_text(text.to_owned());
                    ui.close();
                }
            });
        });
}

// —— 时间戳前缀 ——

/// 时间戳前缀的格式：`[时:分:秒.毫秒]`，本地时间。
///
/// `%.3f` 在 chrono 里的含义是「带小数点的三位小数秒」，也就是毫秒。这不是靠印象
/// 写的：`tests::timestamp_format_has_brackets_and_millis` 用固定时刻把它钉住了。
const TIMESTAMP_FORMAT: &str = "[%H:%M:%S%.3f]";

/// 时间戳前缀的显示颜色（灰）。
///
/// 前缀与设备发来的内容不同源，用颜色把它标成"注解"，避免用户把它当成设备输出
/// 一起复制走。这是显示注解，不是数据编码，因此不随主题变化。
const TIMESTAMP_COLOR: egui::Color32 = egui::Color32::from_rgb(117, 117, 117);

/// 接收数据是否加时间戳前缀，以及按什么粒度加。
///
/// 用三值枚举而不是两个 `bool`：`PerLine` 与 `PerBatch` 是真实存在的二选一，
/// 拆成两个开关就会造出"不加时间戳、但按行"这种没有意义的组合。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum TimestampPrefix {
    /// 不加（默认）。与没有这个功能时的行为逐字节相同。
    #[default]
    Off,
    /// 每个逻辑行首加一个；**一行跨多个数据块时只在行首加一次**。
    PerLine,
    /// 每收到一个数据块加一个（一个数据块里可能有多行）。
    PerBatch,
}

impl TimestampPrefix {
    /// 界面上显示用的短标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::PerLine => "每行",
            Self::PerBatch => "每批",
        }
    }
}

/// 把一个时刻渲染成 `[HH:MM:SS.mmm]`。
///
/// 抽成独立函数（而不是在收数据的热路径里内联 `Local::now().format(..)`）是为了让
/// 测试能喂一个固定时刻：格式串写错只会在界面上表现为一串怪字符，不会编译报错。
pub fn format_timestamp(now: chrono::DateTime<Local>) -> String {
    now.format(TIMESTAMP_FORMAT).to_string()
}

/// 构造一个时间戳前缀片段。
fn timestamp_span(prefix: &str) -> ColoredSpan {
    ColoredSpan {
        text: prefix.to_string(),
        style: AnsiStyle {
            fg: Some(TIMESTAMP_COLOR),
            bold: false,
        },
    }
}

/// 按 `mode` 给一批片段加上时间戳前缀。
///
/// 行边界定义为 `\n`，与原始字节流一致（ASCII 视图把 `\n` 显示成 `"\\n\n"`，
/// 其中仍有一个真实换行，因此行数与收到的字节流相同；HEX 视图每批一行，于是
/// "每行"在那里自然退化成"每批"，不需要特判）。`at_line_start` 说的是**这批之前**
/// 的内容是否停在行首——`PerLine` 靠它保证"一行被拆成多个数据块时只在行首加一次"。
///
/// 为什么在片段层插而不是在文本层插：`receive_text` 是由片段拼接而来的，两处各插
/// 一遍迟早走偏；只插一次，文本镜像与片段镜像自然一致。
fn timestamped_spans(
    spans: Vec<ColoredSpan>,
    mode: TimestampPrefix,
    prefix: &str,
    at_line_start: bool,
) -> Vec<ColoredSpan> {
    // 关闭时原样返回：不分配、不重排片段。没有内容时也一个前缀都不加——宁可什么
    // 都不写，也不要凭空造出一行空数据（`[T]` 后面什么都没有，看起来像收到了空行）。
    // 判据是"有非空片段"而不是"片段数组非空"：文本模式收到半个多字节序列时，
    // 片段可能是一个空片段。
    let has_content = spans.iter().any(|span| !span.text.is_empty());
    if mode == TimestampPrefix::Off || !has_content {
        return spans;
    }

    match mode {
        TimestampPrefix::PerBatch => {
            let mut out: Vec<ColoredSpan> = Vec::with_capacity(spans.len() + 1);
            out.push(timestamp_span(prefix));
            out.extend(spans);
            out
        }
        TimestampPrefix::PerLine => {
            let mut out: Vec<ColoredSpan> = Vec::with_capacity(spans.len() + 4);
            let mut line_start = at_line_start;

            for span in spans {
                let style = span.style;
                if span.text.is_empty() {
                    // 空片段原样保留：这个函数只负责插前缀，不顺手整理片段序列。
                    out.push(span);
                    continue;
                }

                let mut rest = span.text.as_str();
                while !rest.is_empty() {
                    if line_start {
                        out.push(timestamp_span(prefix));
                        line_start = false;
                    }
                    match rest.find('\n') {
                        // 换行本身留在本行末尾：前缀加在**下一**行的开头，
                        // 因此一行以 `\n` 结尾时不会拖出一个空的前缀行。
                        Some(idx) => {
                            out.push(ColoredSpan {
                                text: rest[..=idx].to_string(),
                                style,
                            });
                            rest = &rest[idx + 1..];
                            line_start = true;
                        }
                        None => {
                            out.push(ColoredSpan {
                                text: rest.to_string(),
                                style,
                            });
                            rest = "";
                        }
                    }
                }
            }
            out
        }
        // 上面已经提前返回，这里只是让 `match` 完备。
        TimestampPrefix::Off => spans,
    }
}

impl ComTool {
    /// 上一批接收内容的结尾是否停在行首（`PerLine` 用它决定本批开头要不要补前缀）。
    ///
    /// 从 `receive_text` 的尾部推导，而不是另设一个 `bool` 字段：三种显示模式、
    /// 程序提示（"已打开串口"这类）、以及「清空日志」都会写这个镜像，任何与它平行
    /// 的状态字段迟早会在某个分支上与它不一致——而那种不一致的表现是"少了一个
    /// 时间戳"或"多出一个空行"，很难在界面上看出来。
    fn at_receive_line_start(&self) -> bool {
        self.receive_text.is_empty() || self.receive_text.ends_with('\n')
    }

    /// 让「HEX 接收」与「字节查看」收敛为**互斥**的合法组合。
    ///
    /// 为什么两者必须互斥：`receive_data()` 的三种显示模式是一条 `if/else` 链，且
    /// **HEX 优先于字节查看**——两个都开着时片段只写 `display_spans`，而字节视图渲染
    /// 的是 `ascii_spans`。结果不是"两种格式混排"，而是字节视图**一片空白**。
    ///
    /// 参数是"本轮用户刚把哪一个勾上"（`.changed()` 且新值为 `true`）。本函数是**把点击
    /// 落实下来的那一处**：置上刚点的那项、关掉另一项，所以调用方不必先自己把值写回字段，
    /// 漏写也不会丢点击。不要把方向改成"一律让 HEX 胜出"：那样用户在 HEX 模式下点击
    /// 「字节查看」会被当场撤销，表现为"点了没反应"——比原来的空视图更难排查。
    ///
    /// 两者都为 `false` 时用于**启动修复**（旧存档可能把两个 `true` 都存了进去），此时
    /// 不改变任何值，只按 `receive_data()` 的分支优先级把冲突修成"HEX 胜出"。
    ///
    /// 菜单与接收设置行都调这一个函数——互斥规则只有一处定义，就不会出现"菜单里能
    /// 同时勾上、主界面里不能"这种两边不一致。
    pub fn resolve_display_mode(&mut self, hex_just_enabled: bool, ascii_just_enabled: bool) {
        if hex_just_enabled {
            self.hex_view = true;
            self.ascii_view = false;
        } else if ascii_just_enabled {
            self.ascii_view = true;
            self.hex_view = false;
        } else if self.hex_view && self.ascii_view {
            self.ascii_view = false;
        }
    }

    /// 按 [`ComTool::timestamp_prefix`] 给本批接收片段加上时间戳前缀。
    ///
    /// 关闭时原样返回，连时间都不取——没有开这个功能的人不该为此付出任何代价。
    fn apply_timestamp_prefix(&self, spans: Vec<ColoredSpan>) -> Vec<ColoredSpan> {
        if self.timestamp_prefix == TimestampPrefix::Off || spans.is_empty() {
            return spans;
        }
        // 行首状态必须在本批写入镜像**之前**读取，否则读到的就是本批自己的结尾。
        let at_line_start = self.at_receive_line_start();
        // 一批共用一个时刻：同一块数据里的多行本来就到达于同一瞬间，逐个取时间只
        // 会让同一批内的毫秒数互不相同，看上去像"每隔几毫秒来一行"的假象。
        let prefix = format_timestamp(Local::now());
        timestamped_spans(spans, self.timestamp_prefix, &prefix, at_line_start)
    }

    /// 从事件通道取走本轮到达的全部数据，按当前视图模式写入三份镜像。
    ///
    /// 写镜像的同时会把同一段文本投递给实时文件日志（见 [`ComTool::log_append`]），
    /// 因此「日志文件里有的内容」与「接收区里看到的内容」始终一致。
    pub fn receive_data(&mut self) {
        let mut received = Vec::new();
        // 一次性提示先攒着再落笔：`&self.event_rx` 的借用还活着时不能调用需要
        // `&mut self` 的 `push_receive_note`。`channel_down` 同理——停定时发送也要
        // `&mut self`，只能在借用结束之后再办。
        let mut notes: Vec<String> = Vec::new();
        let mut channel_down = false;

        if let Some(rx) = &self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ChannelEvent::Bytes(data) => {
                        received.extend_from_slice(&data);
                    }
                    ChannelEvent::Failed(msg) => {
                        notes.push(format!("串口错误: {msg}\n"));
                        self.connected = false;
                        channel_down = true;
                    }
                    ChannelEvent::Closed => {
                        notes.push("串口连接已关闭\n".to_string());
                        self.connected = false;
                        channel_down = true;
                    }
                }
            }
        }

        // 提示先落笔，保持与旧实现相同的顺序：本次的错误提示显示在本批数据之前。
        for note in notes {
            self.push_receive_note(note);
        }

        // 通道已经没了，定时发送必须一起停：否则重连成功后会自己接着往外发数据，
        // 而用户并不知道是谁在发。
        if channel_down {
            self.stop_repeat_send();
        }

        if !received.is_empty() {
            // 实时图表吃的是**原始字节**，与当前是哪种显示模式无关：三种模式是对
            // 同一份数据的不同解释，而图表是并列的第四种。放在这里而不是某个模式
            // 分支内，正是为了让"切到 HEX 看字节时曲线还在动"不依赖于分支顺序。
            // 时间戳前缀只加在下面那份显示文本上，绝不进这里——图表解析的必须是
            // 设备真正发来的字节。
            self.chart.push_bytes(&received, self.chart_max_samples);

            // 三种模式各自把本批字节解释成片段；"加时间戳 → 写镜像 → 投日志"这三步
            // 是共同的，放在分支外只实现一次：否则以后新增一种显示模式，很容易漏掉
            // 其中某一步，而漏掉的表现是"某个视图里没有时间戳"这种不显眼的缺陷。
            let mut spans = if self.hex_view {
                // HEX 模式：直接按字节显示，不做 ANSI/UTF-8 处理
                let mut line = String::new();
                for b in &received {
                    line.push_str(&format!("{:02X} ", b));
                }
                line.push('\n');
                vec![ColoredSpan {
                    text: line,
                    style: AnsiStyle::default(),
                }]
            } else if self.ascii_view {
                // ASCII 模式：逐字节解码，控制字符/高位字节转义为可见形式
                ascii_spans_from_bytes(&received)
            } else {
                // 文本模式：解析 ANSI 颜色并跨包拼接 UTF-8
                self.decoder.feed(&received)
            };

            spans = self.apply_timestamp_prefix(spans);

            // 镜像落点必须与上面那条 if/else 链**逐分支对应**：HEX 优先于字节查看，
            // 所以字节镜像的条件里也要把 hex_view 排除掉。
            //
            // 界面层已经让两者互斥（[`ComTool::resolve_display_mode`]，菜单与接收设置行
            // 都走它），所以正常操作下到不了这里。这个 `!hex_view` 是**纵深防御**：手改
            // 存档、或将来有别的调用方绕过 UI 直接置位时，落点仍然与分支一致，而不是把
            // HEX 片段塞进字节镜像、让字节视图显示出一份"看着像数据其实不该在这"的内容。
            let to_ascii_mirror = !self.hex_view && self.ascii_view;
            let mut plain = String::new();
            for span in spans {
                plain.push_str(&span.text);
                if to_ascii_mirror {
                    self.ascii_spans.push(span);
                } else {
                    self.display_spans.push(span);
                }
            }
            self.receive_text.push_str(&plain);
            if to_ascii_mirror {
                self.ascii_text.push_str(&plain);
            }
            self.log_append(&plain);
        }

        // 无论本次有没有新数据都裁一遍：开销是 O(日志长度) 的字符计数，且不会
        // 增长内存。放在这里而不是各分支内部，是为了让"上限"只有一处实现。
        // 注意裁剪只作用于内存镜像，实时文件日志不受这个上限约束。
        self.trim_receive_logs();
    }

    /// 把四份接收镜像各自裁剪到 [`crate::app::ComTool::max_log_chars`] 以内（0 = 不限）。
    ///
    /// 为什么是"各自裁剪"而不是"按 `receive_text` 裁好再同步"：三种模式写入的镜像
    /// 并不相同（HEX/文本模式写 `display_spans`，字节查看模式写 `ascii_spans`，而
    /// `receive_text` 三种模式都写），三者之间没有一一对应的下标关系。各自裁到同一
    /// 上限后，任何视图渲染时看到的长度都不会超过上限，这就够了。
    pub fn trim_receive_logs(&mut self) {
        let max = self.max_log_chars;
        if max == 0 {
            return;
        }
        trim_text_front(&mut self.receive_text, max);
        trim_spans_front(&mut self.display_spans, max);
        trim_text_front(&mut self.ascii_text, max);
        trim_spans_front(&mut self.ascii_spans, max);
    }

    /// 清空三份接收镜像、复位解码器残留，并把滚动位置一并归零。
    ///
    /// 只动内存：正在记录的日志文件保持原样（要停止记录请用「停止记录」），否则
    /// 用户清一下屏幕就会连带丢失磁盘上已经记下的内容。
    pub fn clear_receive(&mut self, ui: &egui::Ui) {
        self.receive_text.clear();
        self.display_spans.clear();
        self.ascii_text.clear();
        self.ascii_spans.clear();
        self.decoder.reset();
        let id = ui.make_persistent_id(SCROLL_AREA_ID);
        egui::scroll_area::State::default().store(ui.ctx(), id);
    }

    pub fn build_layout(&self, default_color: egui::Color32) -> egui::text::LayoutJob {
        layout_job_from(&self.display_spans, default_color)
    }

    /// ASCII 视图的 LayoutJob（与彩色视图同款等宽 14px 样式）
    pub fn build_ascii_layout(&self, default_color: egui::Color32) -> egui::text::LayoutJob {
        layout_job_from(&self.ascii_spans, default_color)
    }

    pub fn render_color_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 把 display_spans（含一次性开关状态提示）构造成带颜色的 LayoutJob
        let job = self.build_layout(egui::Color32::BLACK);

        // ③ 在函数内部绘制滚动区，替换掉主界面里默认的滚动区
        if show_receive_scroll(ui, job, self.auto_scroll) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }

    /// ASCII 视图：逐字节解码显示，控制字符/高位字节转义可见化
    pub fn render_ascii_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 由 ascii_spans 构造 LayoutJob
        let job = self.build_ascii_layout(egui::Color32::BLACK);

        // ③ 绘制滚动区（样式与彩色模式一致）
        if show_receive_scroll(ui, job, self.auto_scroll) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `with_ymd_and_hms` 由 trait 提供，调用 `chrono::Local` 时需要它在作用域内
    use chrono::TimeZone as _;
    use std::sync::mpsc;

    /// 把解析结果拼成纯文本，便于断言
    fn joined(bytes: &[u8]) -> String {
        ascii_spans_from_bytes(bytes)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn printable_passes_through() {
        assert_eq!(joined(b"Hello"), "Hello");
    }

    #[test]
    fn cr_lf_tab_escaped_and_newline_breaks_line() {
        // \r \n \t 显示为转义；\n 额外带一个真实换行
        assert_eq!(joined(b"A\r\nB\tC"), "A\\r\\n\nB\\tC");
    }

    #[test]
    fn control_and_high_bytes_are_visible() {
        assert_eq!(joined(&[0x00, 0x7F, 0xFF]), "[00][7F]\\xFF");
    }

    #[test]
    fn utf8_multibyte_is_shown_per_byte() {
        // “中” = E4 B8 AD → 按字节显示为三个 \xHH
        assert_eq!(joined("中".as_bytes()), "\\xE4\\xB8\\xAD");
    }

    /// 文本模式解码：把 AnsiDecoder 输出拼成纯文本
    fn decoded(bytes: &[u8]) -> String {
        AnsiDecoder::default()
            .feed(bytes)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn decoder_invalid_bytes_become_visible_hex_markers() {
        // 非法字节 0xFF 不再生成 U+FFFD，而是显示为灰色 [FF] 标记
        assert_eq!(decoded(b"AB\xFFCD"), "AB[FF]CD");
        // 非法序列 E0 80（第二字节越界）逐字节标记后继续正常解码
        assert_eq!(decoded(&[0xE0, 0x80, b'A']), "[E0][80]A");
        // 末尾不完整的序列先保留，等下一块数据补齐，不提前输出标记
        assert_eq!(decoded(&[0xE4, 0xB8]), "");
    }

    #[test]
    fn decoder_keeps_split_multibyte_across_feeds() {
        // “中” 被拆到两次 feed：第一次不完整 → 无输出；第二次补齐 → 完整字符
        let mut d = AnsiDecoder::default();
        assert!(d.feed(&[0xE4, 0xB8]).is_empty());
        assert_eq!(
            d.feed(&[0xAD, b'!'])
                .into_iter()
                .map(|s| s.text)
                .collect::<String>(),
            "中!"
        );
    }

    #[test]
    fn decoder_valid_utf8_passes_through() {
        assert_eq!(decoded("ok中⣿".as_bytes()), "ok中⣿");
    }

    // —— 日志长度上限（FIFO 淘汰）——

    fn span(text: &str) -> ColoredSpan {
        ColoredSpan {
            text: text.to_string(),
            style: AnsiStyle::default(),
        }
    }

    fn span_text(spans: &[ColoredSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn trim_text_noop_when_within_limit() {
        let mut text = String::from("abc");
        trim_text_front(&mut text, 3);
        assert_eq!(text, "abc");
    }

    #[test]
    fn trim_text_keeps_the_tail() {
        let mut text = String::from("0123456789");
        trim_text_front(&mut text, 4);
        assert_eq!(text, "6789");
    }

    /// 必须按字符边界切分：若用字节下标，"中" 会被切成半个导致 panic 或乱码。
    #[test]
    fn trim_text_cuts_on_char_boundary() {
        let mut text = String::from("中文abc");
        trim_text_front(&mut text, 3);
        assert_eq!(text, "abc");

        let mut text = String::from("a中文");
        trim_text_front(&mut text, 2);
        assert_eq!(text, "中文");
    }

    #[test]
    fn trim_spans_drops_whole_spans() {
        let mut spans = vec![span("aaaa"), span("bb"), span("c")];
        trim_spans_front(&mut spans, 3);
        assert_eq!(span_text(&spans), "bbc");
        assert_eq!(spans.len(), 2);
    }

    /// 边界片段要被**从中间切开**，而不是整段删除或整段保留。
    #[test]
    fn trim_spans_splits_the_boundary_span() {
        // 拼接后为 "abcdefghi"（9 字符），上限 5 → 保留尾部 "efghi"：
        // "abc" 整段淘汰，"defg" 被切掉首字符 d，剩余 "efg" + "hi"
        let mut spans = vec![span("abc"), span("defg"), span("hi")];
        trim_spans_front(&mut spans, 5);
        assert_eq!(span_text(&spans), "efghi");
        assert_eq!(span_text(&spans).chars().count(), 5);
        assert_eq!(spans.len(), 2);
    }

    /// 部分切分后必须保留该片段原本的样式，否则 ANSI 颜色会随裁剪丢失。
    #[test]
    fn trim_spans_keeps_style_of_the_kept_part() {
        let color = egui::Color32::from_rgb(1, 2, 3);
        let mut spans = vec![ColoredSpan {
            text: "abcdef".to_string(),
            style: AnsiStyle {
                fg: Some(color),
                bold: true,
            },
        }];
        trim_spans_front(&mut spans, 2);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "ef");
        assert_eq!(spans[0].style.fg, Some(color));
        assert!(spans[0].style.bold);
    }

    #[test]
    fn trim_spans_handles_empty_and_exact_length() {
        let mut spans: Vec<ColoredSpan> = Vec::new();
        trim_spans_front(&mut spans, 10);
        assert!(spans.is_empty());

        let mut spans = vec![span("abcd")];
        trim_spans_front(&mut spans, 4);
        assert_eq!(span_text(&spans), "abcd");
    }

    #[test]
    fn trim_spans_multibyte_boundary() {
        // 拼接后为 "中文字符ab"（6 字符），上限 3 → 保留尾部 "符ab"
        let mut spans = vec![span("中文字符"), span("ab")];
        trim_spans_front(&mut spans, 3);
        assert_eq!(span_text(&spans), "符ab");
        assert_eq!(span_text(&spans).chars().count(), 3);
    }

    #[test]
    fn trim_receive_logs_treats_zero_as_unlimited() {
        let mut app = ComTool {
            max_log_chars: 0,
            ..ComTool::default()
        };
        app.receive_text = "x".repeat(100);
        app.display_spans = vec![span(&"y".repeat(100))];
        app.trim_receive_logs();
        assert_eq!(app.receive_text.chars().count(), 100);
        assert_eq!(app.display_spans[0].text.chars().count(), 100);
    }

    /// 四份镜像各写各的（三种模式写入的镜像并不相同），但都必须被裁到同一上限。
    #[test]
    fn trim_receive_logs_bounds_all_four_mirrors() {
        let mut app = ComTool {
            max_log_chars: 10,
            ..ComTool::default()
        };
        app.receive_text = "0123456789abc".to_string();
        app.display_spans = vec![span("01234"), span("56789"), span("abc")];
        app.ascii_text = "0123456789abc".to_string();
        app.ascii_spans = vec![span("0123456789abc")];

        app.trim_receive_logs();

        assert_eq!(app.receive_text.chars().count(), 10);
        assert_eq!(span_text(&app.display_spans).chars().count(), 10);
        assert_eq!(app.ascii_text.chars().count(), 10);
        assert_eq!(span_text(&app.ascii_spans).chars().count(), 10);
        // 淘汰的是最旧的内容，保留的是尾部
        assert_eq!(app.receive_text, "3456789abc");
        assert_eq!(span_text(&app.display_spans), "3456789abc");
    }

    // —— 时间戳前缀（P3）——

    /// 固定时刻，让前缀断言与当前时间无关。
    fn fixed_time() -> chrono::DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 9, 21, 12, 16, 46)
            .single()
            .expect("本地时间应唯一")
            + chrono::Duration::milliseconds(123)
    }

    /// 格式串写错不会编译报错，只会在界面上显示成一串怪字符——所以用固定时刻钉住。
    /// 这里同时固定了三件事：方括号、冒号分隔、`%.3f` 是"小数点 + 三位毫秒"。
    #[test]
    fn timestamp_format_has_brackets_and_millis() {
        let text = format_timestamp(fixed_time());
        assert_eq!(text, "[12:16:46.123]");
        assert_eq!(text.chars().count(), 14, "长度固定才不会让各行前缀参差不齐");
    }

    /// 不足两位的时分秒要补零，否则行首会左右跳动。
    #[test]
    fn timestamp_pads_single_digit_fields() {
        let t = Local
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("本地时间应唯一");
        assert_eq!(format_timestamp(t), "[03:04:05.000]");
    }

    /// 关闭时必须**原样**返回：片段个数、顺序、样式都不能被这层包装改动。
    #[test]
    fn timestamp_off_returns_spans_untouched() {
        let out = timestamped_spans(
            vec![span("a\nb"), span("c")],
            TimestampPrefix::Off,
            "[T]",
            true,
        );
        assert_eq!(span_text(&out), "a\nbc");
        assert_eq!(out.len(), 2, "关闭时连片段都不该被重排");
        assert!(out.iter().all(|s| s.style.fg.is_none()));
    }

    #[test]
    fn per_batch_prefix_is_added_once_at_the_front() {
        let out = timestamped_spans(vec![span("a\nb\n")], TimestampPrefix::PerBatch, "[T]", true);
        assert_eq!(span_text(&out), "[T]a\nb\n", "每批只加一个前缀，且在最前面");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].style.fg,
            Some(TIMESTAMP_COLOR),
            "前缀要用灰色标出它是注解而不是数据"
        );
    }

    /// 空批次不加前缀：宁可不写，也不要凭空造出一行。
    #[test]
    fn empty_batch_never_gets_a_prefix() {
        for mode in [TimestampPrefix::PerLine, TimestampPrefix::PerBatch] {
            assert!(timestamped_spans(Vec::new(), mode, "[T]", true).is_empty());
            // 只有空片段的批次同理（文本模式收到半个多字节序列时就是这种输入）
            let out = timestamped_spans(vec![span("")], mode, "[T]", true);
            assert_eq!(span_text(&out), "");
        }
    }

    #[test]
    fn per_line_prefix_goes_to_each_line_start() {
        let out = timestamped_spans(vec![span("a\nb\nc")], TimestampPrefix::PerLine, "[T]", true);
        assert_eq!(span_text(&out), "[T]a\n[T]b\n[T]c");
    }

    /// 末尾换行后不该多出一个"空行前缀"——否则每批都比数据多一行。
    #[test]
    fn per_line_prefix_is_not_dangled_after_a_trailing_newline() {
        let out = timestamped_spans(vec![span("a\n")], TimestampPrefix::PerLine, "[T]", true);
        assert_eq!(span_text(&out), "[T]a\n");
    }

    /// 行的后半段到达时不能重复加前缀——这正是"每行"与"每批"的区别所在。
    #[test]
    fn per_line_prefix_is_not_repeated_mid_line() {
        let out = timestamped_spans(vec![span("b\n")], TimestampPrefix::PerLine, "[T]", false);
        assert_eq!(span_text(&out), "b\n", "上一批没有停在行首时不该补前缀");
    }

    /// 换行与文本落在不同片段时（文本模式的一整段 `"a\nb"`、字节查看视图的
    /// `"\\n"` 与真实换行），前缀仍要加在正确的位置。
    #[test]
    fn per_line_prefix_survives_span_boundaries() {
        let out = timestamped_spans(
            vec![span("a\n"), span("b"), span("\nc")],
            TimestampPrefix::PerLine,
            "[T]",
            true,
        );
        assert_eq!(span_text(&out), "[T]a\n[T]b\n[T]c");
    }

    /// 经事件通道把一批字节送进接收链路。`receive_data` 是唯一的数据入口，
    /// 因此这里覆盖的正是真实串口走的同一条路径（不需要串口硬件）。
    fn feed(app: &mut ComTool, bytes: &[u8]) {
        let (tx, rx) = mpsc::channel();
        app.event_rx = Some(rx);
        tx.send(ChannelEvent::Bytes(bytes.to_vec()))
            .expect("投递应成功");
        app.receive_data();
    }

    /// 默认（关闭）时接收内容必须与没有这个功能时逐字节相同。
    #[test]
    fn timestamp_off_keeps_receive_text_identical() {
        let mut app = ComTool::default();
        assert_eq!(app.timestamp_prefix, TimestampPrefix::Off);
        feed(&mut app, b"A\nB\n");
        assert_eq!(app.receive_text, "A\nB\n");
        assert_eq!(app.display_spans.len(), 1, "关闭时也不该把片段切碎");
    }

    /// 每行：一行跨两个数据块时只在行首加一次，换行之后重新加。
    #[test]
    fn timestamps_are_per_line_across_batches() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerLine,
            ..ComTool::default()
        };

        feed(&mut app, b"AB");
        assert_eq!(count_prefixes(&app.receive_text), 1, "{}", app.receive_text);
        assert!(app.receive_text.ends_with("AB"));

        feed(&mut app, b"CD");
        assert_eq!(
            count_prefixes(&app.receive_text),
            1,
            "同一行里不该再来一个前缀: {}",
            app.receive_text
        );

        feed(&mut app, b"\nEF\n");
        assert_eq!(
            count_prefixes(&app.receive_text),
            2,
            "换行之后是新的一行，应当重新标注: {}",
            app.receive_text
        );
        assert!(app.receive_text.ends_with("EF\n"));
    }

    /// 每批：一个数据块一个前缀，块内的第二行不再单独标注——与"每行"形成对照。
    #[test]
    fn per_batch_prefix_marks_each_chunk() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerBatch,
            ..ComTool::default()
        };
        feed(&mut app, b"a\nb\n");
        feed(&mut app, b"c\n");

        let lines: Vec<&str> = app.receive_text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with('['));
        assert!(
            !lines[1].starts_with('['),
            "同一批里的第二行不该带前缀: {:?}",
            lines
        );
        assert!(lines[2].starts_with('['));
    }

    /// 三种显示模式都要加前缀：否则"切到 HEX 就没时间戳了"会是个莫名其妙的差异。
    #[test]
    fn timestamps_are_added_in_every_display_mode() {
        for (hex_view, ascii_view) in [(true, false), (false, true), (false, false)] {
            let mut app = ComTool {
                timestamp_prefix: TimestampPrefix::PerBatch,
                hex_view,
                ascii_view,
                ..ComTool::default()
            };
            feed(&mut app, b"AB\n");

            assert!(
                app.receive_text.starts_with('['),
                "HEX={hex_view} 字节查看={ascii_view} 模式应带前缀，实际为: {}",
                app.receive_text
            );
            assert!(app.receive_text.ends_with('\n'), "本批内容本身不能被吃掉");
        }
    }

    /// HEX 与字节查看同时打开时走 HEX 分支（HEX 优先），片段落进 `display_spans`。
    ///
    /// 这条锁定的是这次改写**没有**悄悄改变镜像落点：`receive_data` 的三条分支刚被
    /// 收敛成一条公共尾部，镜像落点若不再与分支一一对应，就是这个配置下露出来。
    ///
    /// 界面层现在让两者互斥（[`ComTool::resolve_display_mode`]），所以这个组合只能由
    /// 直接置位构造出来——这条测试守的正是那条**纵深防御**：无论谁绕过 UI 把两个都设成
    /// `true`，落点都必须跟着分支走，而不是把 HEX 片段塞进字节镜像。
    #[test]
    fn hex_takes_precedence_over_ascii_like_before() {
        let mut app = ComTool {
            hex_view: true,
            ascii_view: true,
            ..ComTool::default()
        };
        feed(&mut app, b"AB\n");

        assert!(
            app.receive_text.contains("41 42 0A "),
            "{}",
            app.receive_text
        );
        assert_eq!(app.display_spans.len(), 1, "HEX 分支应写入 display_spans");
        assert!(app.ascii_spans.is_empty(), "HEX 优先时不该再写 ascii_spans");
        assert!(app.ascii_text.is_empty());
    }

    /// 勾上「HEX 接收」时关掉「字节查看」。
    ///
    /// 这个组合就是缺陷本身：HEX 分支只写 `display_spans`，而字节视图渲染的是
    /// `ascii_spans`——两个都开着时接收区是**一片空白**，不是"两种格式混排"。
    #[test]
    fn enabling_hex_turns_the_byte_view_off() {
        let mut app = ComTool {
            ascii_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(true, false);

        assert!(app.hex_view);
        assert!(!app.ascii_view, "字节视图留着必然是空的");
    }

    /// 勾上「字节查看」时关掉「HEX 接收」。
    ///
    /// 方向很重要：这一项必须是**用户刚点的那个**。若函数一律让 HEX 胜出，用户点击
    /// 「字节查看」会被当场撤销，表现为"点了没反应"——比原来的空视图更难排查。
    #[test]
    fn enabling_the_byte_view_turns_hex_off() {
        let mut app = ComTool {
            hex_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(false, true);

        assert!(!app.hex_view);
        assert!(app.ascii_view, "刚点的那一项必须留下来");
    }

    /// 没有点击事件时（启动修复），非法组合按 HEX 优先修好。
    #[test]
    fn stale_conflicting_modes_are_repaired_in_favour_of_hex() {
        let mut app = ComTool {
            hex_view: true,
            ascii_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(false, false);

        assert!(app.hex_view, "与 receive_data 的分支顺序一致");
        assert!(!app.ascii_view);
    }

    /// 合法组合、以及"两个都关掉"（纯文本视图）都不该被这个函数改动。
    #[test]
    fn legal_display_combinations_are_left_alone() {
        for (hex_view, ascii_view) in [(true, false), (false, true), (false, false)] {
            let mut app = ComTool {
                hex_view,
                ascii_view,
                ..ComTool::default()
            };
            app.resolve_display_mode(false, false);

            assert_eq!(
                (app.hex_view, app.ascii_view),
                (hex_view, ascii_view),
                "HEX={hex_view} 字节查看={ascii_view} 是合法组合，不该被改"
            );
        }
    }

    /// HEX 视图里前缀是"顶在字节串前面的注解"：方括号本身就把注解与数据分开了，
    /// 后面不再补空格（前缀是独立片段，不该悄悄改动数据那一侧的字符）。
    /// 这条断言把前缀在 HEX 模式下的确切样子固定下来（时间本身是动态的，所以只能
    /// 断言后缀部分）。
    #[test]
    fn hex_view_shows_the_prefix_before_the_byte_dump() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerBatch,
            hex_view: true,
            ..ComTool::default()
        };
        feed(&mut app, b"AB\n");
        assert!(
            app.receive_text.contains("]41 42 0A \n"),
            "实际为: {}",
            app.receive_text
        );
    }

    /// **前缀绝不能进图表**：图表解析的必须是设备真正发来的字节。
    #[test]
    fn timestamps_do_not_leak_into_the_chart() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerLine,
            ..ComTool::default()
        };
        feed(&mut app, b"1,2\n3,4\n");

        assert!(app.receive_text.starts_with('['), "先确认前缀确实加上了");
        assert_eq!(app.chart.channel_count(), 2);
        assert_eq!(app.chart.sample_count(), 4, "2 通道 × 2 组 = 4 个样本点");
        assert_eq!(
            app.chart.ignored_fields(),
            0,
            "前缀文本不是数字；它若混进字节流，这里会被记成无效字段"
        );
    }

    /// 程序提示（"串口已关闭"这类）不是收到的数据，不加前缀；但它以换行结尾，
    /// 因此下一条数据落在新行上、带自己的前缀。
    #[test]
    fn program_notes_are_not_timestamped_but_do_start_a_new_line() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerLine,
            ..ComTool::default()
        };
        app.push_receive_note("串口已关闭\n".to_string());
        assert_eq!(app.receive_text, "串口已关闭\n", "提示不该被加上时间戳");

        feed(&mut app, b"X\n");
        assert_eq!(count_prefixes(&app.receive_text), 1);
        assert!(app.receive_text.contains("串口已关闭\n["));
        assert!(app.receive_text.ends_with("X\n"));
    }

    /// 「清空日志」之后下一条数据要重新带前缀：清空后接收区本来就是空的，
    /// 从行首开始才对，否则清空后的第一行会缺一个时间戳。
    #[test]
    fn clearing_the_receive_log_restarts_the_line() {
        let mut app = ComTool {
            timestamp_prefix: TimestampPrefix::PerLine,
            ..ComTool::default()
        };
        feed(&mut app, b"AB"); // 故意停在行中间
        assert_eq!(count_prefixes(&app.receive_text), 1);

        egui::__run_test_ui(|ui| app.clear_receive(ui));
        assert!(app.receive_text.is_empty());

        feed(&mut app, b"CD\n");
        assert_eq!(
            count_prefixes(&app.receive_text),
            1,
            "清空后应从行首重新标注: {}",
            app.receive_text
        );
    }

    /// 前缀只出现在行首，且不吞掉任何数据。
    fn count_prefixes(text: &str) -> usize {
        text.matches('[').count()
    }
}
