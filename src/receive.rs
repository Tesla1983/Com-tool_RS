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
// 遗留编码（GBK / GB18030 / UTF-16LE）的增量解码。UTF-8 不用它——那条路是本项目
// 自己的严格解码实现，行为已被一组测试固定（见 `AnsiDecoder::flush_pending`）。
use encoding_rs::{DecoderResult, Encoding, GB18030, GBK, UTF_8, UTF_16LE};

/// 接收区滚动视图的持久化 ID。
///
/// 普通文本、彩色、字节查看三种模式共用同一个 ID：切换模式时滚动位置得以延续，
/// 「清空日志」也用它把滚动状态一并复位。
pub(crate) const SCROLL_AREA_ID: &str = "receive_scroll";

/// 接收区滚动块的高度上限（像素）。
///
/// 与 [`SCROLL_AREA_ID`] 一样属于"布局预算"：日志竖向最多占这么多，剩下的留给
/// 同一列里的其它区块。
pub(crate) const RECEIVE_SCROLL_MAX_HEIGHT: f32 = 400.0;

/// 滚动块的滚动方向。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScrollAxis {
    /// 只纵向滚动。内容按现有宽度折行时用。
    Vertical,
    /// 纵横双向都可滚。不折行的等宽日志用。
    Both,
}

/// 造一块「高度有上限、放不下就自己滚」的内容容器。
///
/// **实时图表复用日志这一套**（2026-09-24 用户要求）：两块都是"挂在中央区里、
/// 高度有上限、内容溢出时自己滚"的区块。共用同一个构造函数之后，`auto_shrink`
/// 与上限语义只有一处定义，不会出现"日志改了、图表没跟上"。
///
/// ⚠️ `ScrollArea` 的 `max_height` 是**至多**：可用空间更小时它自己会缩，因此不会
/// 越界。这恰好补上了 `egui_plot::Plot::height()` 的短板——后者同时写 `min_size.y`
/// 与 `height` 并取 `at_least`，矩形**恒为该值**，可用空间再小也不缩（实测把中央区
/// 挤爆、把下面的发送区顶出可视区，见 [`crate::panels::chart`] 的注释）。
///
/// ⚠️ 滚轮归属（读 egui 0.36.1 `containers/scroll_area.rs:1219-1254`）：滚动区
/// **只在"内容确实超出可视区、且该方向还能滚"时**才消费 `smooth_scroll_delta`。内容
/// 装得下时两个条件都不成立，滚轮原样留给内部控件——所以图表放在里面时，只要内容
/// 恰好装下，滚轮缩放/平移照旧；只有矮到出现滚动条，滚动块才接管滚轮。
pub(crate) fn capped_scroll_area(
    axis: ScrollAxis,
    id_salt: &str,
    max_height: f32,
) -> egui::ScrollArea {
    let area = match axis {
        ScrollAxis::Vertical => egui::ScrollArea::vertical(),
        ScrollAxis::Both => egui::ScrollArea::both(),
    };
    area.auto_shrink([false, false])
        .max_height(max_height)
        .id_salt(id_salt)
}

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

/// 文本的字符编码（接收解码与发送编码共用同一个偏好）。
///
/// **GBK 与 GB18030 的接收行为完全相同**：`encoding_rs` 的文档写明
/// "The decoder for this encoding is the same as the decoder for gb18030"——
/// 两者的差异只在**编码器**上（GB18030 能表示四字节序列对应的字符，GBK 不能）。
/// 因此接收侧这两档可以混用，发送侧才需要区分。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum TextEncoding {
    #[default]
    Utf8,
    Gbk,
    Gb18030,
    Utf16Le,
}

impl TextEncoding {
    /// 界面上按这个顺序单选。
    pub const ALL: [Self; 4] = [Self::Utf8, Self::Gbk, Self::Gb18030, Self::Utf16Le];

    /// 界面显示名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Gbk => "GBK",
            Self::Gb18030 => "GB18030",
            Self::Utf16Le => "UTF-16LE",
        }
    }

    /// `encoding_rs` 里对应的编码。解码器与编码器都从这里取。
    ///
    /// 注意 GBK 这一项取到的是 `GBK`（而不是 `GB18030`）是有意的：两者的**解码器**
    /// 相同，但**编码器**不同——`GBK.encode()` 的行为贴近 Windows 代码页 936，
    /// 对老设备的兼容性更好；要用四字节序列表示生僻字请选 GB18030。
    pub(crate) fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 => UTF_8,
            Self::Gbk => GBK,
            Self::Gb18030 => GB18030,
            Self::Utf16Le => UTF_16LE,
        }
    }

    /// 该编码是否与 ASCII 兼容——决定能不能做**字节级**的 ANSI 转义序列识别。
    ///
    /// UTF-16LE 里 `ESC` 是 `1B 00` 两个字节：若仍按字节找 `0x1B`，会把 `1B` 吃进
    /// 转义序列、再因为下一字节不是 `[` 而把这两个字节一起丢掉——**那是丢数据**。
    /// 所以 UTF-16LE 干脆不做 ANSI 解析（真实世界里也没有 UTF-16 的彩色终端）。
    fn is_ascii_compatible(self) -> bool {
        !matches!(self, Self::Utf16Le)
    }
}

/// 为一个非法字节序列生成灰色标记。
///
/// `consumed` 是本次解码调用**累计**消费到的绝对位置（含坏字节本身，以及解码器为
/// 试探下个序列而多吃的 `extra` 个字节）；坏字节因此位于 `consumed - extra` 之前。
///
/// 关键是**宁可退化也不编造**：`DecoderResult::Malformed` 的文档说明坏字节"可能来自
/// 更早的输入缓冲"，这时它并不在本次缓冲区里。取不到就只给一个 `[??]`——显示成别的
/// 字节比显示得含糊更糟，那会让用户去排查一个根本不存在的字节。
fn malformed_mark(bytes: &[u8], consumed: usize, bad_len: usize, extra: usize) -> String {
    let end = consumed.saturating_sub(extra);
    let start = end.saturating_sub(bad_len);
    if start < end && end <= bytes.len() && bad_len > 0 {
        bytes[start..end]
            .iter()
            .map(|b| format!("[{b:02X}]"))
            .collect()
    } else {
        "[??]".to_string()
    }
}

/// 接收数据的文本视图解码器。
///
/// 持有两份跨数据块的残留：`pending_bytes` 保存被截断的多字节序列，
/// `pending_ansi` 保存尚未收齐的 CSI 转义序列。样式状态 `style` 同样跨块延续，
/// 因为一条 SGR 序列的影响范围是其后所有字节，而不是当前数据块。
pub struct AnsiDecoder {
    pending_ansi: Vec<u8>,
    pending_bytes: Vec<u8>,
    /// 当前生效的编码。由 `feed` 传入并在变化时同步，不单独暴露 setter——
    /// "忘记同步编码"会让同一批字节被两种编码解释，而这种错很难看出来。
    encoding: TextEncoding,
    /// `encoding_rs` 的增量解码器。它自己保存"被截断的半个字符"，因此跨包拼接
    /// 不需要手工判定边界（见 `flush_non_utf8` 的注释）。
    decoder: encoding_rs::Decoder,
    pub style: AnsiStyle,
}

impl Default for AnsiDecoder {
    fn default() -> Self {
        Self {
            pending_ansi: Vec::new(),
            pending_bytes: Vec::new(),
            encoding: TextEncoding::default(),
            decoder: TextEncoding::default().encoding().new_decoder(),
            style: AnsiStyle::default(),
        }
    }
}

impl AnsiDecoder {
    /// 丢弃全部残留状态与样式，回到初始态。「清空日志」时调用。
    ///
    /// 只清残留、**不动编码偏好**：那是用户设置，不该因为点了一下「清空日志」就变。
    pub fn reset(&mut self) {
        self.pending_ansi.clear();
        self.pending_bytes.clear();
        self.decoder = self.encoding.encoding().new_decoder();
        self.style = AnsiStyle::default();
    }

    /// 送入一个数据块，返回本次可确定输出的片段。
    ///
    /// 编码是**参数**而不是字段赋值：这样"这一批字节按哪种编码解释"只由调用点决定，
    /// 不存在"设了编码但忘了让解码器知道"的中间态。
    pub fn feed(&mut self, bytes: &[u8], encoding: TextEncoding) -> Vec<ColoredSpan> {
        self.sync_encoding(encoding);

        let mut spans = Vec::new();
        for &b in bytes {
            if self.pending_ansi.is_empty() {
                if self.encoding.is_ascii_compatible() && b == 0x1B {
                    // 遇到 ESC：先输出已积累的普通文本，再开始收集转义序列
                    self.flush_pending(&mut spans);
                    self.pending_ansi.push(b);
                } else {
                    self.pending_bytes.push(b);
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
        self.flush_pending(&mut spans);
        spans
    }

    /// 编码变化时**丢弃跨包残留**并换上新的解码器。
    ///
    /// 这一步是必须的：把上一批留下的半截字节用新编码解一遍，会输出一段随机汉字，
    /// 而且看上去"像是设备发的"。
    fn sync_encoding(&mut self, encoding: TextEncoding) {
        if self.encoding == encoding {
            return;
        }
        self.encoding = encoding;
        self.decoder = encoding.encoding().new_decoder();
        self.pending_bytes.clear();
    }

    /// 把积累的字节解成片段。UTF-8 与其它编码走两条路，原因见各自分支的注释。
    fn flush_pending(&mut self, spans: &mut Vec<ColoredSpan>) {
        if self.pending_bytes.is_empty() {
            return;
        }
        if self.encoding == TextEncoding::Utf8 {
            self.flush_utf8(spans);
        } else {
            self.flush_non_utf8(spans);
        }
    }

    /// 非 UTF-8 编码：交给 `encoding_rs` 的**增量**解码器。
    ///
    /// 为什么不自己判定"完整前缀"：各编码的边界规则各不相同（GB18030 有四字节
    /// 序列、UTF-16 有代理对），手写一遍就是给自己种 bug——而 `Decoder` 正是为
    /// 流式输入设计的，它自己保存被截断的字符，跨包拼接由构造保证。
    ///
    /// 代价是**非法序列只能标一个 `[??]`**：`DecoderResult::Malformed` 的文档明确
    /// 说明它报出的坏字节"可能来自更早的输入缓冲"，因此无法可靠地逐字节定位。
    /// UTF-8 那条路仍然逐字节给出 `[XX]`，不受影响。
    fn flush_non_utf8(&mut self, spans: &mut Vec<ColoredSpan>) {
        let bad_style = AnsiStyle {
            fg: Some(ASCII_NONPRINT_COLOR),
            bold: false,
        };
        let bytes = std::mem::take(&mut self.pending_bytes);
        let mut good = String::new();
        let mut offset = 0usize;

        while offset < bytes.len() {
            // 这个 API 把 `String` 的**剩余容量**当输出上限，并且明确保证不会自己
            // 扩容（"guarantees not to cause a reallocation"），所以必须先 reserve。
            // 用解码器自己给的界，比拍一个倍数可靠。
            let need = self
                .decoder
                .max_utf8_buffer_length_without_replacement(bytes.len() - offset)
                .unwrap_or((bytes.len() - offset) * 4 + 8);
            good.reserve(need);

            let base = offset;
            // `*_without_replacement` 这一族返回的是 `DecoderResult`（不是 3 元组的
            // `CoderResult`）：元组里只有"读了几个字节"，写入量看 `good.len()` 的变化。
            let (result, read) =
                self.decoder
                    .decode_to_string_without_replacement(&bytes[base..], &mut good, false);
            offset = base + read;

            match result {
                DecoderResult::InputEmpty => break,
                DecoderResult::OutputFull => {
                    // 上面已按解码器给出的上界预留，正常到不了这里；真到了也只在
                    // 完全没有进展时退出，绝不空转。
                    if read == 0 {
                        break;
                    }
                }
                DecoderResult::Malformed(bad_len, extra) => {
                    if !good.is_empty() {
                        spans.push(ColoredSpan {
                            text: std::mem::take(&mut good),
                            style: self.style,
                        });
                    }
                    spans.push(ColoredSpan {
                        text: malformed_mark(&bytes, offset, bad_len as usize, extra as usize),
                        style: bad_style,
                    });
                }
            }
        }

        if !good.is_empty() {
            spans.push(ColoredSpan {
                text: good,
                style: self.style,
            });
        }
    }

    fn flush_utf8(&mut self, spans: &mut Vec<ColoredSpan>) {
        if self.pending_bytes.is_empty() {
            return;
        }
        let valid_len = utf8_complete_prefix_len(&self.pending_bytes);
        let (complete, tail) = self.pending_bytes.split_at(valid_len);
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
        self.pending_bytes = tail.to_vec();
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

/// 接收区等宽文字的**基准**字号。
///
/// 用它的地方一律先乘上全局字体档位系数（见 [`monospace_font_id`]）——
/// 这个常量只表示"档位为标准时是多大"。
pub(crate) const RECEIVE_MONOSPACE_BASE_SIZE: f32 = 14.0;

/// 按当前全局字体档位取接收区/发送框用的等宽 `FontId`。
///
/// **为什么不直接写 `FontId::monospace(14.0)`**：那样全局字体档位对接收区完全不起作用
/// ——用户在设置里把字体调到"特大"，结果日志和发送框还是 14px，"全局字体大小"就名不副实。
/// 从 `style.text_styles` 里取 `TextStyle::Monospace` 的实际字号，档位一改它自动跟着变，
/// 不需要在每条调用链上多传一个参数。
///
/// `fallback_size` 是 `ui` 拿不到（或表里缺 Monospace 项）时用的基准值；正常情况下
/// 取到的就是 `RECEIVE_MONOSPACE_BASE_SIZE × 档位系数`。
pub(crate) fn monospace_font_id(ui: &egui::Ui, fallback_size: f32) -> egui::FontId {
    if let Some(font_id) = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Monospace)
        .cloned()
    {
        return font_id;
    }
    egui::FontId::monospace(fallback_size)
}

/// 由一组 `ColoredSpan` 构造 `LayoutJob`（彩色/字节查看两种视图共用）。
///
/// `font_id` 由调用方给出：通常是 [`monospace_font_id`] 的结果，于是字号跟随全局档位。
fn layout_job_from(
    spans: &[ColoredSpan],
    default_color: egui::Color32,
    font_id: egui::FontId,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in spans {
        job.append(
            span.text.as_str(),
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
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
///
/// `copy_all_label` 是右键菜单那一项的文案。这两个接收视图函数是**自由函数**（不持有
/// `ComTool`），所以文案只能由调用方传进来——`Language::strings()` 是 `&'static`，
/// 传 `&str` 没有额外开销。
fn show_receive_scroll(
    ui: &mut Ui,
    job: egui::text::LayoutJob,
    stick_to_bottom: bool,
    copy_all_label: &str,
) -> bool {
    let mut copy_all = false;
    capped_scroll_area(
        ScrollAxis::Vertical,
        SCROLL_AREA_ID,
        RECEIVE_SCROLL_MAX_HEIGHT,
    )
    .stick_to_bottom(stick_to_bottom)
    .show(ui, |ui| {
        let response = ui.add(egui::Label::new(job).wrap().selectable(true));
        response.context_menu(|ui| {
            if ui.button(copy_all_label).clicked() {
                copy_all = true;
                ui.close();
            }
        });
    });
    copy_all
}

/// 普通文本模式接收区：不折行（Extend）+ 纵/横双向滚动，
/// 等宽 galley 绘制保证盲文 ASCII 图案整行对齐；可选中、右键复制全部日志。
///
/// **默认文字色必须取自主题**（`ui.visuals().text_color()`），不能写死黑色：
/// 深色主题下接收区背景也是深的，黑字会直接看不见。
pub(crate) fn show_plain_receive(
    ui: &mut Ui,
    text: &str,
    stick_to_bottom: bool,
    copy_all_label: &str,
) {
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE),
        ui.visuals().text_color(),
        f32::INFINITY,
    );
    capped_scroll_area(ScrollAxis::Both, SCROLL_AREA_ID, RECEIVE_SCROLL_MAX_HEIGHT)
        .stick_to_bottom(stick_to_bottom)
        .show(ui, |ui| {
            let response = ui.add(
                egui::Label::new(job)
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true),
            );
            response.context_menu(|ui| {
                if ui.button(copy_all_label).clicked() {
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

// 这里原本有一个 `TimestampPrefix::label()`，返回硬编码的"关闭 / 每行 / 每批"。
// F3 把它删掉了：那是**界面词汇**，不是领域概念，留在领域类型上迟早有人顺手用上，
// 于是那几个词就再也不会被翻译。现在它叫 `crate::strings::timestamp_prefix_label`，
// 必须显式传文案表。

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

/// 接收区当前该显示哪一种视图 —— **"显示模式"的唯一判定**。
///
/// 「本批字节怎么解释成片段」「写进哪一份镜像」「渲染时画哪一份」这三件事必须由同一个
/// 判定决定。2026-09-24 之前它们各判各的：片段构造与镜像落点只看 `hex_view` /
/// `ascii_view`，渲染分支却让 `color_view` 排在最前 ⇒ 「颜色」与「字节查看」同时勾上时，
/// 数据写进了 `ascii_spans`，界面画的却是**没人写过**的 `display_spans`，接收区一片
/// 空白（用户目视发现；见 `HANDOVER.md` §3.16）。
///
/// 判定顺序**只在开关组合非法**（手改存档、或将来有调用方绕过互斥直接置位）时才有多解，
/// 方向与 [`ComTool::resolve_display_mode`] 的启动修复一致：`hex` 胜 `ascii`、`ascii` 胜
/// `color`（更强的视图胜出）。注意顺序**不承担正确性**——落点与渲染同源，所以任何组合都
/// 不会空白；顺序只决定"非法组合该显示成哪一个"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayMode {
    /// 彩色文本（`color_view`）
    Color,
    /// 字节查看（`ascii_view`）
    Bytes,
    /// HEX 接收（`hex_view`）
    Hex,
    /// 普通文本
    Text,
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

    /// 见 [`DisplayMode`]。渲染、片段构造、镜像落点三处都调它，不要各写一份判断。
    ///
    /// 前两个分支处理的是**互斥失败**（手改存档、或将来有调用方绕过 UI 直接置位）时该
    /// 认哪个视图，方向与 [`ComTool::resolve_display_mode`] 的启动修复**一致**：
    /// `hex` 胜 `ascii`、`ascii` 胜 `color`（更强的视图胜出）。第三个分支里的
    /// `hex + color` 不是冲突而是**合法**组合，返回 `Color` 正是为了保住它改动前的外观
    /// （彩色的 HEX 文本）。
    pub(crate) fn display_mode(&self) -> DisplayMode {
        if self.ascii_view && !self.hex_view {
            DisplayMode::Bytes
        } else if self.color_view && !self.ascii_view {
            DisplayMode::Color
        } else if self.hex_view {
            DisplayMode::Hex
        } else {
            DisplayMode::Text
        }
    }

    /// 让三个显示开关收敛为**合法的互斥组合**。
    ///
    /// 合法组合有五种：文本 / HEX / 字节查看 / 彩色文本 / **HEX + 彩色**。两条禁令：
    /// - **HEX 与字节查看不能同时为真**：片段构造是一条 `if/else` 链，HEX 优先 ⇒ 字节
    ///   镜像 `ascii_spans` 没人写过，字节视图**一片空白**。
    /// - **字节查看与颜色不能同时为真**：颜色视图渲染的是 `display_spans`，而字节模式
    ///   只写 `ascii_spans` ⇒ 彩色视图**一片空白**（同一个缺陷，空的换成另一份镜像）。
    ///
    /// 参数是"本轮用户刚把哪一个勾上"（`.changed()` 且新值为 `true`）。本函数是**把点击
    /// 落实下来的那一处**：置上刚点的那项、关掉与它冲突的项，所以调用方不必先自己把值写回
    /// 字段，漏写也不会丢点击。不要把方向改成"一律让视图胜出"：那样用户在 HEX 模式下点
    /// 「字节查看」会被当场撤销，表现为"点了没反应"——比原来的空视图更难排查。
    ///
    /// 三个参数都为 `false` 时用于**启动修复**（旧存档可能把非法组合存了进去），此时只修
    /// 冲突、不改用户的选择：`hex` 关 `ascii`、`ascii` 关 `color`。后者是**字节视图胜出**
    /// 而非颜色胜出，因为"只开字节查看"是合法配置（`config.rs::ascii_view_alone_survives_
    /// round_trip` 钉着），而颜色只是文本模式的修饰、在转义字节上本来就没有意义。
    ///
    /// 菜单（`panels/menu_bar.rs`）与接收设置行（`app.rs`）都调这一个函数——互斥规则只有
    /// 一处定义，就不会出现"菜单里能同时勾上、主界面里不能"这种两边不一致。
    pub fn resolve_display_mode(
        &mut self,
        hex_just_enabled: bool,
        ascii_just_enabled: bool,
        color_just_enabled: bool,
    ) {
        if hex_just_enabled {
            self.hex_view = true;
            self.ascii_view = false;
        } else if ascii_just_enabled {
            self.ascii_view = true;
            self.hex_view = false;
            self.color_view = false;
        } else if color_just_enabled {
            self.color_view = true;
            self.ascii_view = false;
        } else {
            if self.hex_view && self.ascii_view {
                self.ascii_view = false;
            }
            if self.ascii_view && self.color_view {
                self.color_view = false;
            }
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

        // 名词先取好：`if let Some(rx) = &self.event_rx` 那段里 `self` 正被借走，
        // 在循环体内再调方法要跟 `self.connected = false` 的字段写操作抢借用。
        // 提示行本身不翻译（见 `crate::strings` 的模块文档），但名词必须跟着
        // 传输类型走——否则 TCP 掉线会写一句"串口连接已关闭"。
        let noun = self.transport_noun();

        if let Some(rx) = &self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ChannelEvent::Bytes(data) => {
                        received.extend_from_slice(&data);
                        // 真正收到数据 = 连接确实是通的：把自动重连的"连续失败次数"
                        // 清零。串口的 `open()` 是乐观置位（`connected = true` 先于
                        // 真实打开结果），只看 `connected` 无法区分"打开了"与"待会儿
                        // 才失败"，而收到字节是**不可伪造**的成功证据。
                        self.reconnect_failures = 0;
                    }
                    ChannelEvent::Failed(msg) => {
                        notes.push(format!("{noun}错误: {msg}\n"));
                        self.connected = false;
                        // 掉线了 ⇒ 后继重连若成功要重新补一条"已打开"。
                        // 不清它的话，"连上→掉线→自动重连"之后那句成功提示永远不会
                        // 再出现（记账位还停在上一轮的"已说过"）。
                        self.announced_open = false;
                        channel_down = true;
                        // 记一次失败，供指数退避使用（见 `ComTool::auto_reconnect`）。
                        // 用 `saturating_add` 防溢出：纯计数器不该在极端情况下 panic。
                        let before = self.reconnect_failures;
                        self.reconnect_failures = self.reconnect_failures.saturating_add(1);
                        // 刚好撞到上限的那一刻**留一条说明**：此后 `auto_reconnect`
                        // 会彻底停手，用户必须知道"它不会再自动试了"，否则会一直
                        // 盯着界面等一个不会发生的重连。只在"刚跨过阈值"时提示一次。
                        if before < crate::app::MAX_RECONNECT_ATTEMPTS
                            && self.reconnect_failures >= crate::app::MAX_RECONNECT_ATTEMPTS
                            && self.connection_kind == crate::app::ConnectionKind::Serial
                        {
                            notes.push(format!(
                                "自动重连已停止（连续失败 {} 次），请手动点「连接」重试\n",
                                self.reconnect_failures
                            ));
                        }
                    }
                    ChannelEvent::Closed => {
                        notes.push(format!("{noun}连接已关闭\n"));
                        self.connected = false;
                        self.announced_open = false;
                        channel_down = true;
                        // 对端**正常**关闭不是"重连失败"——它多半是用户主动关的或
                        // 设备正常下线，不该被算进退避次数里（算进去会让正常的
                        // "关掉再开"变得一次比一次慢）。
                    }
                    ChannelEvent::Notice(msg) => {
                        // 非致命：只显示，**不动** `connected`、也不停定时发送。
                        // 判成掉线的话，拉不动一根控制线会让界面从"已连接"跳走，
                        // 而数据其实照样在收发（见 `ChannelEvent::Notice` 的文档）。
                        notes.push(format!("{noun}: {msg}\n"));
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
            // 本批字节怎么解释，由**唯一判定**决定（见 [`ComTool::display_mode`]）。
            // 在取任何 `&mut self.xxx` 之前先算好：`mode` 是 `Copy`，之后不再借用 `self`。
            let mode = self.display_mode();
            let mut spans = match mode {
                DisplayMode::Hex => {
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
                }
                DisplayMode::Bytes => {
                    // 字节查看：逐字节解码，控制字符/高位字节转义为可见形式
                    ascii_spans_from_bytes(&received)
                }
                // 彩色视图与纯文本视图吃的是**同一份**解释结果：颜色只是渲染方式不同，
                // 数据怎么解析完全一样。编码必须是 `feed` 的参数：编码解码只在这条路上
                // 发生，而 HEX / 字节查看两个视图看到的是原始字节，与编码无关。
                DisplayMode::Color | DisplayMode::Text => {
                    self.decoder.feed(&received, self.text_encoding)
                }
            };

            spans = self.apply_timestamp_prefix(spans);

            // 落点与渲染**同源**：只有 `Bytes` 模式写字节镜像，其余都写显示镜像。
            //
            // 这里不再写 `!self.hex_view && self.ascii_view` 那种"手工与分支对齐"的表达式
            // ——它已经漏改过一次：`color_view` 加进来时没人同步改它，于是「颜色 + 字节查看」
            // 同时勾上时数据只进 `ascii_spans`，而彩色视图画的是空的 `display_spans`，
            // 接收区一片空白（用户目视发现，见 `HANDOVER.md` §3.16）。判定收敛到
            // `display_mode()` 之后，新增视图只可能在**一处**造成遗漏，且那条路径上有测试。
            let to_ascii_mirror = mode == DisplayMode::Bytes;
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

    /// 彩色视图的 `LayoutJob`。
    ///
    /// `font_id` 由调用方给：渲染路径传 [`monospace_font_id`]（跟随全局字体档位），
    /// 测试传 `FontId::monospace(RECEIVE_MONOSPACE_BASE_SIZE)`（只关心颜色，与档位无关）。
    /// **不接受 `Option` 然后内部兜底**：那样渲染路径写错了也看不出来，兜底会静默生效。
    pub fn build_layout(
        &self,
        default_color: egui::Color32,
        font_id: egui::FontId,
    ) -> egui::text::LayoutJob {
        layout_job_from(&self.display_spans, default_color, font_id)
    }

    /// ASCII（字节查看）视图的 `LayoutJob`，与彩色视图同款等宽字形。
    pub fn build_ascii_layout(
        &self,
        default_color: egui::Color32,
        font_id: egui::FontId,
    ) -> egui::text::LayoutJob {
        layout_job_from(&self.ascii_spans, default_color, font_id)
    }

    /// 接收区的渲染分发：**画哪一份镜像由 [`ComTool::display_mode`] 决定**。
    ///
    /// 抽成一个函数是**为了可测**。写在 `eframe::App::ui()` 里的话，测试得造一个
    /// `eframe::Frame`（造不出来），于是"画的是哪一份镜像"就只剩人工目视一条路——
    /// 而这正是 2026-09-24 那个空白缺陷逃过全部自动化测试的原因：镜像写得没错，
    /// 错的是"写的那份与画的那份不是同一份"。
    ///
    /// 现在它与 `receive_data()` 的落点读的是**同一个判定**，改一处不会漏另一处。
    pub(crate) fn render_receive_area(&mut self, ui: &mut Ui) {
        // 先取值再 `match`：`Strings` 的字段是 `&'static str`，先拿出来就不会让
        // `self.language` 的不可变借用横跨后面需要 `&mut self` 的那两个分支。
        let copy_all_log = self.language.strings().copy_all_log;
        match self.display_mode() {
            DisplayMode::Color => self.render_color_receive(ui),
            // 字节查看：逐字节解码，控制字符/高位字节转义为可见形式
            DisplayMode::Bytes => self.render_ascii_receive(ui),
            // HEX 与普通文本共用同一个渲染：差别只在片段本身（HEX 模式下片段已是
            // 十六进制文本），不需要两套绘制。
            DisplayMode::Hex | DisplayMode::Text => {
                show_plain_receive(ui, &self.receive_text, self.auto_scroll, copy_all_log)
            }
        }
    }

    pub fn render_color_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 把 display_spans（含一次性开关状态提示）构造成带颜色的 LayoutJob
        //
        // 默认色取主题的正文色：深色主题下是浅色，浅色主题下是深色。写死黑色的话
        // 深色主题里接收区就是"黑底黑字"。只有**设备真的发了 ANSI 颜色**时才会
        // 用 `spans` 里的自定义色覆盖它——那是有意为之的显式着色，不该被主题改写。
        let job = self.build_layout(
            ui.visuals().text_color(),
            monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE),
        );

        // ③ 在函数内部绘制滚动区，替换掉主界面里默认的滚动区
        if show_receive_scroll(
            ui,
            job,
            self.auto_scroll,
            self.language.strings().copy_all_log,
        ) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }

    /// ASCII 视图：逐字节解码显示，控制字符/高位字节转义可见化
    pub fn render_ascii_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 由 ascii_spans 构造 LayoutJob
        //
        // 同 `render_color_receive`：默认色随主题。字节视图自己给控制字符/高位
        // 字节上了显式颜色（`ASCII_CTRL_COLOR` / `ASCII_NONPRINT_COLOR`），
        // 剩下的可打印字节走这个默认色，因此它必须在深色主题下也看得清。
        let job = self.build_ascii_layout(
            ui.visuals().text_color(),
            monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE),
        );

        // ③ 绘制滚动区（样式与彩色模式一致）
        if show_receive_scroll(
            ui,
            job,
            self.auto_scroll,
            self.language.strings().copy_all_log,
        ) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在给定容器高度下画一次公共滚动块，返回 `(可视区高, 内容高)`。
    fn drive_scroll_block(viewport_h: f32, cap: f32, content_h: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(500.0, viewport_h),
            )),
            ..Default::default()
        };

        let inner = std::cell::Cell::new(0.0_f32);
        let content = std::cell::Cell::new(0.0_f32);
        let mut out = ctx.run_ui(input, |ui| {
            let out = capped_scroll_area(ScrollAxis::Vertical, "capped_scroll_area_test", cap)
                .show(ui, |ui| {
                    ui.allocate_space(egui::vec2(10.0, content_h));
                });
            inner.set(out.inner_rect.height());
            content.set(out.content_size.y);
        });
        // `FullOutput` 带着纹理增量，直接 drop 会 panic（看着像抓到缺陷，其实是清理没做）。
        out.textures_delta.clear();

        (inner.get(), content.get())
    }

    /// 公共滚动块的两个性质——它们正是"图表复用日志滚动区"赖以成立的东西：
    ///
    /// ① `max_height` 是**至多**：容器比它高时只占上限那么高；
    /// ② 内容比可视区高时，egui 会报"需要滚动"（`ScrollAreaOutput` 文档里的原文判据是
    ///    "If this content_size is larger than inner_rect, then there was need for
    ///    scrolling"）。
    ///
    /// ② 是这条的关键：只断 ① 的话，把上限设成"永远装得下"也照样绿，等于没测到"能滚"。
    /// 图表那条路径依赖的正是 ②——矮窗口下夹住容器，而不是压扁绘图区。
    #[test]
    fn capped_scroll_area_caps_the_height_and_can_scroll() {
        const CAP: f32 = 400.0;
        const TALL_VIEWPORT: f32 = 600.0;
        const SHORT_VIEWPORT: f32 = 120.0;
        const CONTENT_H: f32 = 300.0;

        // ① 容器(600) 比上限(400) 高 ⇒ 只占 400，而不是把 600 都用掉。
        let (inner, content) = drive_scroll_block(TALL_VIEWPORT, CAP, CONTENT_H);
        assert!(
            inner <= CAP + 0.5,
            "容器有 {TALL_VIEWPORT} 点高，滚动块却占了 {inner:.1} 点——上限 {CAP} 没生效"
        );
        assert!(
            content <= inner + 0.5,
            "内容 {content:.1} 点没能装进 {inner:.1} 点的可视区，这一段的场景不对"
        );

        // ② 容器(120) 比内容(300) 矮 ⇒ 可视区被夹住，且 egui 认为"需要滚动"。
        let (inner, content) = drive_scroll_block(SHORT_VIEWPORT, CAP, CONTENT_H);
        assert!(
            inner <= SHORT_VIEWPORT + 0.5,
            "可视区被内容顶到 {inner:.1} 点（容器只有 {SHORT_VIEWPORT} 点）"
        );
        assert!(
            content > inner,
            "content_size({content:.1}) 未超过 inner_rect({inner:.1})，egui 认为不需要滚动"
        );
    }

    // `with_ymd_and_hms` 由 trait 提供，调用 `chrono::Local` 时需要它在作用域内
    use chrono::TimeZone as _;
    use std::sync::mpsc;

    /// 测试用的等宽字体。
    ///
    /// 这里**故意写死基准字号**而不是走 [`monospace_font_id`]：这批测试断言的是
    /// "颜色/换行怎么落笔"，与全局字体档位无关。若跟着档位走，将来有人改了默认档位，
    /// 这些测试会跟着一起变——那是另一条测试（字体档位那条）该管的事，不该在这里混着测。
    const TEST_MONO: egui::FontId = egui::FontId::monospace(RECEIVE_MONOSPACE_BASE_SIZE);

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

    /// 文本模式解码：把 AnsiDecoder 输出拼成纯文本。
    ///
    /// 明确传 `Utf8`：下面这一组测试锁的是**本项目自己的 UTF-8 严格解码**，
    /// 它不因为 F2 引入 `encoding_rs` 而改变（见 `AnsiDecoder::flush_pending`）。
    fn decoded(bytes: &[u8]) -> String {
        decoded_with(bytes, TextEncoding::Utf8)
    }

    fn decoded_with(bytes: &[u8], encoding: TextEncoding) -> String {
        AnsiDecoder::default()
            .feed(bytes, encoding)
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
        assert!(d.feed(&[0xE4, 0xB8], TextEncoding::Utf8).is_empty());
        assert_eq!(
            d.feed(&[0xAD, b'!'], TextEncoding::Utf8)
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

    // ---------- 文本编码（F2） ----------

    #[test]
    fn gbk_decodes_two_byte_chinese() {
        // “你好” 的 GBK 编码
        assert_eq!(
            decoded_with(&[0xC4, 0xE3, 0xBA, 0xC3], TextEncoding::Gbk),
            "你好"
        );
    }

    /// **F2 的关键测试**：GBK 字符被拆到两批里仍然完整。
    ///
    /// 若实现改成"收到一批就 lossy 解一次"，第一批会变成 U+FFFD，这条一定失败——
    /// 而那种退化在界面上看起来只是"偶尔出现一个乱码字"，很难被发现。
    #[test]
    fn gbk_keeps_split_multibyte_across_feeds() {
        let mut d = AnsiDecoder::default();
        assert!(
            d.feed(&[0xC4], TextEncoding::Gbk).is_empty(),
            "半截 GBK 字符不该提前输出"
        );
        assert_eq!(
            d.feed(&[0xE3], TextEncoding::Gbk)
                .into_iter()
                .map(|s| s.text)
                .collect::<String>(),
            "你"
        );
    }

    /// 编码切换必须丢弃跨包残留：否则剩下的半截字节会被新编码解成一个随机汉字。
    #[test]
    fn switching_encoding_discards_pending_bytes() {
        let mut d = AnsiDecoder::default();
        assert!(d.feed(&[0xC4], TextEncoding::Gbk).is_empty());

        // 换成 UTF-8：那个孤立的 0xC4 是非法起首字节，但**更不该**被当成 GBK 的
        // 前半个字与下一步拼起来。它要么被丢弃，要么被标成非法——都不会变成汉字。
        let out: String = d
            .feed(&[0xE3], TextEncoding::Utf8)
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_ne!(out, "你", "切换编码后不该还把两批拼成一个 GBK 字符");
    }

    /// GBK 与 GB18030 共用解码器，所以同一段字节在两档下解出同样的文字。
    #[test]
    fn gbk_and_gb18030_decode_identically() {
        let bytes = [0xC4, 0xE3, 0xBA, 0xC3];
        assert_eq!(
            decoded_with(&bytes, TextEncoding::Gbk),
            decoded_with(&bytes, TextEncoding::Gb18030)
        );
    }

    #[test]
    fn gbk_malformed_bytes_are_marked() {
        // 0xFF 在 GBK 里不是合法字节：要标出来，而不是悄悄丢掉或显示成方块
        let out = decoded_with(&[0xFF, b'A'], TextEncoding::Gbk);
        assert!(
            out.contains("[FF]") || out.contains("[??]"),
            "输出为 {out:?}"
        );
        assert!(
            out.ends_with('A'),
            "坏字节之后的内容必须继续解出来：{out:?}"
        );
    }

    /// **UTF-16LE 不做 ANSI 解析**：`ESC` 在它里面是 `1B 00` 两个字节，按字节找
    /// `0x1B` 会把这两个字节一起吃进转义序列再丢掉——那是丢数据。
    #[test]
    fn utf16le_does_not_swallow_escape_bytes() {
        // "A" + ESC 的 UTF-16LE 字节
        let bytes = [0x41, 0x00, 0x1B, 0x00];
        let out = decoded_with(&bytes, TextEncoding::Utf16Le);
        assert_eq!(out, "A\u{1B}", "ESC 不能被当成 ANSI 序列吃掉");
    }

    #[test]
    fn utf16le_decodes_chinese() {
        // “中” = U+4E2D → 字节 2D 4E
        assert_eq!(decoded_with(&[0x2D, 0x4E], TextEncoding::Utf16Le), "中");
    }

    #[test]
    fn utf16le_keeps_split_code_unit_across_feeds() {
        let mut d = AnsiDecoder::default();
        assert!(d.feed(&[0x2D], TextEncoding::Utf16Le).is_empty());
        assert_eq!(
            d.feed(&[0x4E], TextEncoding::Utf16Le)
                .into_iter()
                .map(|s| s.text)
                .collect::<String>(),
            "中"
        );
    }

    /// 编码只影响**文本视图**。HEX 与字节查看两条路看到的是原始字节，与编码无关。
    #[test]
    fn hex_and_byte_views_ignore_the_encoding() {
        let bytes = [0xC4, 0xE3];
        assert_eq!(
            ascii_spans_from_bytes(&bytes)
                .into_iter()
                .map(|s| s.text)
                .collect::<String>(),
            "\\xC4\\xE3"
        );
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
        app.resolve_display_mode(true, false, false);

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
        app.resolve_display_mode(false, true, false);

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
        app.resolve_display_mode(false, false, false);

        assert!(app.hex_view, "与 receive_data 的分支顺序一致");
        assert!(!app.ascii_view);
    }

    /// 勾上「字节查看」时关掉「颜色」——**用户 2026-09-24 目视发现的那个缺陷**。
    ///
    /// 彩色视图渲染 `display_spans`，而字节模式只写 `ascii_spans`：两个都开着时彩色视图
    /// 是**一片空白**，看上去就像"接收的内容丢了"。
    #[test]
    fn enabling_the_byte_view_turns_color_off() {
        let mut app = ComTool {
            color_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(false, true, false);

        assert!(app.ascii_view);
        assert!(!app.color_view, "彩色视图留着必然是空的");
    }

    /// 勾上「颜色」时关掉「字节查看」——方向必须对称：**刚点的那一项要留下**。
    #[test]
    fn enabling_color_turns_the_byte_view_off() {
        let mut app = ComTool {
            ascii_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(false, false, true);

        assert!(app.color_view);
        assert!(!app.ascii_view, "刚点的那一项必须留下来");
    }

    /// 启动修复：旧存档里「字节查看」与「颜色」同时为真时留下**字节查看**。
    ///
    /// 留字节查看而不是颜色，有两个理由：① "只开字节查看"是合法配置（`config.rs` 那边
    /// 有一条专门的往返测试）；② 颜色只是文本模式的修饰，在转义后的字节文本上本来就没有
    /// 意义。方向与 `hex` 关 `ascii` 一致——**更强的视图胜出**。
    #[test]
    fn stale_color_and_byte_view_are_repaired_in_favour_of_the_byte_view() {
        let mut app = ComTool {
            ascii_view: true,
            color_view: true,
            ..ComTool::default()
        };
        app.resolve_display_mode(false, false, false);

        assert!(app.ascii_view);
        assert!(!app.color_view);
    }

    /// 当前模式下**渲染读的那一份镜像**是否为空。
    ///
    /// 测试侧的这份映射必须与 `app.rs` 的渲染分支同源，否则测试会自己骗过自己——两边
    /// 都跟着 [`DisplayMode`] 走，是同一个判定的两次读取，不是两份各自维护的规则。
    fn rendered_mirror_is_empty(app: &ComTool) -> bool {
        match app.display_mode() {
            DisplayMode::Bytes => app.ascii_spans.is_empty(),
            DisplayMode::Color => app.display_spans.is_empty(),
            DisplayMode::Hex | DisplayMode::Text => app.receive_text.is_empty(),
        }
    }

    /// ⭐ **回归测试：任何开关组合都不能让接收区空白。**
    ///
    /// 这就是 2026-09-24 用户目视发现的缺陷（「颜色」+「字节查看」同时勾上 ⇒ 内容"丢了"）。
    /// 根因不是"少写了一份镜像"，而是**写进去的那份和画出来的那份不是同一份**——所以这里
    /// 穷举全部 8 种组合、每种都真的喂一批数据，断言"当前模式下渲染的那份镜像非空"。
    ///
    /// 刻意**不做** `resolve_display_mode`：要覆盖的正是"互斥没兜住"的那些组合（手改存档、
    /// 将来有调用方绕过 UI 直接置位）。注入验证：把 `receive_data()` 的落点改回旧表达式
    /// `!hex_view && ascii_view`，本测试必须变红（`颜色+字节查看` 与 `HEX+字节查看` 会空）。
    #[test]
    fn no_display_combination_leaves_the_receive_area_empty() {
        for hex_view in [false, true] {
            for ascii_view in [false, true] {
                for color_view in [false, true] {
                    let mut app = ComTool {
                        hex_view,
                        ascii_view,
                        color_view,
                        ..ComTool::default()
                    };
                    feed(&mut app, b"AB\n");

                    assert!(
                        !rendered_mirror_is_empty(&app),
                        "HEX={hex_view} 字节查看={ascii_view} 颜色={color_view} 这组开关让\
                         接收区空白了 —— 写镜像与画镜像用的不是同一个判定"
                    );
                }
            }
        }
    }

    /// 合法组合不该被修复动到：`ascii` 与 `hex`、`ascii` 与 `color` 各自互斥，
    /// 但 `hex + color` 是**合法**的（HEX 文本 + 彩色组件），必须原样保留。
    #[test]
    fn legal_display_combinations_are_left_alone() {
        for (hex_view, ascii_view, color_view) in [
            (true, false, false),
            (false, true, false),
            (false, false, false),
            (false, false, true),
            (true, false, true),
        ] {
            let mut app = ComTool {
                hex_view,
                ascii_view,
                color_view,
                ..ComTool::default()
            };
            app.resolve_display_mode(false, false, false);

            assert_eq!(
                (app.hex_view, app.ascii_view, app.color_view),
                (hex_view, ascii_view, color_view),
                "HEX={hex_view} 字节查看={ascii_view} 颜色={color_view} 是合法组合，不该被改"
            );
        }
    }

    /// 把一次**接收区**绘制里落笔的文字抠出来（拼成一串）。
    ///
    /// 走 `Context::run_ui` 而不是 `__run_test_ui`——后者把输出丢掉，什么也观察不到，
    /// 那正是"空转测试"的来源（见 `panels/mod.rs::painted_texts` 的同款说明）。
    fn painted_receive_texts(app: &mut ComTool) -> String {
        let ctx = egui::Context::default();
        // 屏幕尺寸必须给：`run_ui` 的第一个参数是**输入**而不是矩形。不给的话容器尺寸是
        // 默认值，滚动区里可能一行都排不下，于是"画出来了没"这件事本身就不可信。
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.render_receive_area(ui);
        });

        let mut texts = String::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push_str(text_shape.galley.text());
            }
        }

        // `FullOutput` 带着纹理增量，直接 drop 会 panic（`__run_test_ui` 内部就是靠
        // `drop_without_applying_deltas()` 绕开的）。不需要纹理，显式丢弃——否则这条
        // 测试会因为一个与断言无关的清理问题变红，看起来像抓到了缺陷。
        out.textures_delta.clear();
        texts
    }

    /// ⭐ **接收区画的是"当前模式那一份"镜像** —— 三处同源的直接判据。
    ///
    /// 上面那条穷举测试只看"镜像非空"，看不到渲染分发；而 2026-09-24 那个缺陷恰恰是
    /// **写对了、画错了**。这条测试补上这一环：三份镜像内容各不相同，从"画出来的字"
    /// 反推画的是哪一份。
    ///
    /// 注入验证：把 `render_receive_area` 的分发改回 `if self.color_view { … } else if
    /// self.ascii_view { … }`，`颜色+字节查看` 那一档会画出 `COLOR-MARK`，而这里期望
    /// `BYTES-MARK` ⇒ 变红。
    #[test]
    fn the_receive_area_paints_the_mirror_of_the_current_mode() {
        for (hex_view, ascii_view, color_view, expected) in [
            (false, false, false, "PLAIN-MARK"),
            (true, false, false, "PLAIN-MARK"),
            (false, false, true, "COLOR-MARK"),
            (false, true, false, "BYTES-MARK"),
            (true, false, true, "COLOR-MARK"),
            (false, true, true, "BYTES-MARK"),
            (true, true, false, "PLAIN-MARK"),
            (true, true, true, "PLAIN-MARK"),
        ] {
            let mut app = ComTool {
                hex_view,
                ascii_view,
                color_view,
                ..ComTool::default()
            };
            // 三份镜像摆上互不相同的标记。（`render_*` 内部会调一次 `receive_data()`，
            // 但没有新数据时它不会追加，手工摆好的镜像不会被冲掉。）
            app.display_spans.push(ColoredSpan {
                text: "COLOR-MARK\n".into(),
                style: AnsiStyle::default(),
            });
            app.ascii_spans.push(ColoredSpan {
                text: "BYTES-MARK\n".into(),
                style: AnsiStyle::default(),
            });
            app.receive_text = "PLAIN-MARK\n".into();
            app.ascii_text = "BYTES-MARK\n".into();

            let painted = painted_receive_texts(&mut app);

            assert!(
                painted.contains(expected),
                "HEX={hex_view} 字节查看={ascii_view} 颜色={color_view} 应当画出 \
                 {expected}，实际画出: {painted:?}"
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

    /// 接收区的默认文字色必须**跟随主题**，不能写死黑色。
    ///
    /// 这是个真实修过的缺陷：三个视图原先都把默认色硬编码成 `Color32::BLACK`，
    /// 深色主题下背景是深的，日志就成了"黑底黑字"，整块看不见。
    /// 这个测试直接从 LayoutJob 里读实际落笔的颜色，因为"画得出来不 panic"
    /// 那类冒烟测试对颜色一无所知——写死黑色它照样是绿的。
    #[test]
    fn default_text_color_follows_theme() {
        // 深色主题下的正文色（egui 的深色主题给的是浅灰白，不是纯白）
        let dark = egui::Color32::from_rgb(200, 200, 200);
        // 浅色主题下的正文色（深灰）
        let light = egui::Color32::from_rgb(20, 20, 20);

        let mut app = ComTool::default();
        feed(&mut app, b"plain text\n");

        // 普通文本模式：颜色由 show_plain_receive 自己从 ui.visuals() 取，
        // 这里换一条等价路径（同一个默认色来源）来断言它确实被用上了。
        let job = app.build_layout(dark, TEST_MONO);
        let color = job.sections[0].format.color;
        assert_eq!(color, dark, "彩色视图的默认色必须是传进来的主题色");
        assert_ne!(
            color,
            egui::Color32::BLACK,
            "深色主题下默认色绝不能是黑色——那就是这次修的缺陷本身"
        );

        // 换浅色主题色，落笔颜色必须跟着换
        let job = app.build_layout(light, TEST_MONO);
        assert_eq!(
            job.sections[0].format.color, light,
            "同一个 span 在浅色主题下必须用浅色主题的正文色"
        );
    }

    /// 字节查看视图的默认色同样必须跟随主题（它写的是另一份镜像 `ascii_spans`，
    /// 所以必须把它真的切到字节模式才看得到内容——不能靠文本模式的残留状态）。
    #[test]
    fn ascii_view_default_color_follows_theme() {
        let dark = egui::Color32::from_rgb(200, 200, 200);

        let mut app = ComTool {
            ascii_view: true,
            ..ComTool::default()
        };
        feed(&mut app, b"AB\x01\n");
        assert!(
            !app.ascii_spans.is_empty(),
            "字节模式下 ascii_spans 应该有内容，否则这条测试是空转的"
        );

        let job = app.build_ascii_layout(dark, TEST_MONO);
        assert!(
            job.sections.iter().any(|s| s.format.color == dark),
            "字节查看的默认色必须跟随主题（深色主题下不能是黑字）"
        );
        assert!(
            job.sections
                .iter()
                .all(|s| s.format.color != egui::Color32::BLACK),
            "字节查看里不该出现写死的纯黑——控制字符有自己的显式色，其余走主题色"
        );
    }

    /// 设备**真的发了 ANSI 颜色**时，主题色不能把它覆盖掉。
    ///
    /// 上面那条测试管的是"默认色"，这条管的是它的边界：显式着色是有意为之的
    /// 数据编码（设备说"这段是红的"），不该被主题改写。
    #[test]
    fn explicit_ansi_color_overrides_theme_default() {
        let mut app = ComTool::default();
        // `\x1b[31m` = ANSI 红
        feed(&mut app, b"\x1b[31mRED\x1b[0m normal\n");

        let theme_default = egui::Color32::from_rgb(200, 200, 200);
        let job = app.build_layout(theme_default, TEST_MONO);

        let has_custom = job
            .sections
            .iter()
            .any(|s| s.format.color != theme_default && s.format.color != egui::Color32::BLACK);
        assert!(
            has_custom,
            "设备显式发的 ANSI 色必须保留下来，不能被主题默认色抹平"
        );

        // 反过来说：没有着色的那段应该拿到主题默认色
        let has_default = job.sections.iter().any(|s| s.format.color == theme_default);
        assert!(has_default, "未着色的片段应该走主题默认色");
    }

    // ——— 等宽字号必须跟随全局字体档位 ———

    /// `monospace_font_id` 必须**从样式表里读**当前等宽字号，而不是返回一个写死的值。
    ///
    /// 这条是"全局字体大小"这个功能能对接收区/发送框生效的**唯一支点**：
    /// 三处接收视图 + 发送框都调它。如果它被写回 `FontId::monospace(14.0)`，
    /// 那么用户把字体调到"特大"后，正文、按钮都变大了，日志和发送框却纹丝不动——
    /// 而所有"画得出来不 panic"的冒烟测试都会照样绿（它们不关心字号）。
    ///
    /// 这里把样式表里的 Monospace 字号**改掉**，再看函数是不是跟着变。
    /// 只断言"返回了一个等宽 FontId"是空转的：写死的那种实现也满足。
    #[test]
    fn monospace_font_id_follows_the_style_not_a_constant() {
        // 故意用一个**远离基准值**的字号，避免与 `RECEIVE_MONOSPACE_BASE_SIZE`
        // 恰好相等导致"写死也能过"。
        const SENTINEL: f32 = 37.5;

        let mut seen = None;
        egui::__run_test_ui(|ui| {
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Monospace,
                egui::FontId::monospace(SENTINEL),
            );

            let font_id = monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE);
            seen = Some(font_id.size);
        });

        assert_eq!(
            seen,
            Some(SENTINEL),
            "样式表里 Monospace 是 {SENTINEL}px，函数却返回了别的大小——\
             说明它没读样式表（而是返回了写死的 {}）。全局字体档位对接收区就失效了",
            RECEIVE_MONOSPACE_BASE_SIZE
        );
    }

    /// 样式表里没有 Monospace 项时才用 `fallback_size`。
    ///
    /// 与上一条配对：上一条钉"优先读样式表"，这一条钉"读不到时的退路"。
    /// 少了这条，`fallback_size` 这个参数就是死的——删掉它测试也不会红，
    /// 那样将来有人把 `get(...)` 改成 `unwrap()` 就会在缺项时 panic。
    #[test]
    fn monospace_font_id_falls_back_when_the_style_lacks_it() {
        const FALLBACK: f32 = 21.0;

        let mut seen = None;
        egui::__run_test_ui(|ui| {
            ui.style_mut()
                .text_styles
                .remove(&egui::TextStyle::Monospace);
            assert!(
                !ui.style()
                    .text_styles
                    .contains_key(&egui::TextStyle::Monospace),
                "先确认 Monospace 项确实被移除了，否则这条测试测的是另一条路径"
            );

            let font_id = monospace_font_id(ui, FALLBACK);
            seen = Some(font_id.size);
        });

        assert_eq!(
            seen,
            Some(FALLBACK),
            "样式表缺 Monospace 项时应当退到 fallback_size({FALLBACK})"
        );
    }
}
