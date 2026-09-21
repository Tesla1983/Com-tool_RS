//! 接收链路：原始字节 → 可显示的彩色片段 → egui 布局。
//!
//! 一份接收数据在这里维护三份互为镜像的状态：
//!
//! - `receive_text`：纯文本，供普通视图渲染与导出；
//! - `display_spans`：带 ANSI 颜色信息的片段，供彩色视图渲染；
//! - `ascii_text` / `ascii_spans`：逐字节转义后的可见化文本，供字节查看视图渲染。
//!
//! 三者必须同步清空、同步追加——任何只动其中一份的改动都会让切换视图时
//! 看到不一致的内容。
//!
//! 两种解码路径：
//!
//! - [`AnsiDecoder`]：解析 CSI/SGR 转义序列并跨数据块拼接 UTF-8，用于文本视图。
//!   遇到非法字节不生成 U+FFFD（字体缺字形时会渲染成不可读的 □），而是输出
//!   灰色 `[FF]` 标记。
//! - [`ascii_spans_from_bytes`]：完全不做字符集解释，逐字节转义，用于字节查看视图。

use crate::app::ComTool;
use crate::communicationtrait::ChannelEvent;
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

/// 绘制接收区滚动视图（可选中、右键复制全部日志），返回是否点击了「复制全部日志」。
fn show_receive_scroll(ui: &mut Ui, job: egui::text::LayoutJob) -> bool {
    let mut copy_all = false;
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
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
pub(crate) fn show_plain_receive(ui: &mut Ui, text: &str) {
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::monospace(14.0),
        egui::Color32::BLACK,
        f32::INFINITY,
    );
    egui::ScrollArea::both()
        .stick_to_bottom(true)
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

impl ComTool {
    /// 从事件通道取走本轮到达的全部数据，按当前视图模式写入三份镜像。
    pub fn receive_data(&mut self) {
        let mut received = Vec::new();

        if let Some(rx) = &self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ChannelEvent::Bytes(data) => {
                        received.extend_from_slice(&data);
                    }
                    ChannelEvent::Failed(msg) => {
                        self.receive_text.push_str(&format!("串口错误: {}\n", msg));
                        self.connected = false;
                    }
                    ChannelEvent::Closed => {
                        self.receive_text.push_str("串口连接已关闭\n");
                        self.connected = false;
                    }
                }
            }
        }

        if !received.is_empty() {
            if self.hex_view {
                // HEX 模式：直接按字节显示，不做 ANSI/UTF-8 处理
                let mut line = String::new();
                for b in &received {
                    line.push_str(&format!("{:02X} ", b));
                }
                line.push('\n');
                self.receive_text.push_str(&line);
                self.display_spans.push(ColoredSpan {
                    text: line,
                    style: AnsiStyle::default(),
                });
            } else if self.ascii_view {
                // ASCII 模式：逐字节解码，控制字符/高位字节转义为可见形式
                let spans = ascii_spans_from_bytes(&received);
                for span in spans {
                    self.ascii_text.push_str(&span.text);
                    self.receive_text.push_str(&span.text);
                    self.ascii_spans.push(span);
                }
            } else {
                // 文本模式：解析 ANSI 颜色并跨包拼接 UTF-8
                let spans = self.decoder.feed(&received);
                for span in spans {
                    self.receive_text.push_str(&span.text);
                    self.display_spans.push(span);
                }
            }
        }
    }

    /// 清空三份接收镜像、复位解码器残留，并把滚动位置一并归零。
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
        if show_receive_scroll(ui, job) {
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
        if show_receive_scroll(ui, job) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
