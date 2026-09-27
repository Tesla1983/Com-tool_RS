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

/// 用户翻到历史里阅读时，在 [`crate::app::ComTool::max_log_chars`] 之上**额外**允许
/// 保留的字符数。
///
/// 为什么需要这个"放宽"：淘汰是从最旧端做的，而它同时改变视口里内容的身份——滚动
/// 位置（像素）不变，内容却整体上移了一截。贴底时用户看不见（贴着的是最新内容），
/// 但用户往上翻着读的时候，就等于阅读位置每帧被抽走一行，观感与"自动滚动没停"完全
/// 一样（2026-09-26 用户报告："用滚轮往上应该自动停止日志滚动，好像没有实现"）。
///
/// 放宽是**有额度**的，不是"不裁"：上限首先是内存约束，长时间停在历史里而设备一直
/// 在喷数据，不设额度就是无界增长。额度给一个**固定字符数**而不是按倍数放大，这样
/// 用户把 `max_log_chars` 调到很大时内存不会跟着再翻几倍。30 000 字符 ≈ 1 000 行
/// 30 字符的日志，按 1 KB/s 的常见速率够读半分钟。
///
/// 额度用满之后**不再意味着阅读位置要让位**：那之后靠
/// [`ComTool::take_front_trim_compensation`] 把滚动位置同步往回退，淘汰照旧进行而
/// 用户读的那一行原地不动（2026-09-26 用户报告的第二轮："停留超过一定的时间，日志又
/// 自动滚动，再往上滚也停不住"——额度用尽后旧代码把 offset 一路压到 0 并夹住，
/// 于是视口被钉在"最旧保留行"上，那一行自己却在逐帧变新）。
pub(crate) const RECEIVE_TRIM_DEFER_ALLOWANCE: usize = 30_000;

/// 读历史（视口不在底部）时，内存允许在 [`crate::app::ComTool::max_log_chars`] 之上
/// 再涨多少字符——这是**硬上限**，越过之后恢复淘汰、阅读位置让位。
///
/// 为什么"保住阅读位置"必须允许内存继续涨：用户停在历史里读第 L 行，设备还在喷新数据，
/// 那么"第 L 行到现在"的全部内容都得留在内存里，缓冲区必然随时间增长——任何定长缓冲
/// 都保不住这个承诺。所以只能给一个**足够大但有限**的上限：60 000 字符 ≈ 2 000 行
/// 30 字符的日志，默认配置下合计 90 000 字符（≈ 90 KB），既够读很久，又不会让
/// 每帧排版 1 MB 文本把界面拖慢。
///
/// 用固定字符数而不是"倍率"：用户把 `max_log_chars` 调到 100 万时再乘一个倍率，
/// 排版开销会先于内存爆掉。
pub(crate) const RECEIVE_PARKED_EXTRA_CAP: usize = 60_000;

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
    // 四档是 UTF-8 lead 字节的取值范围，写成区间才看得出"它们是连续的四段"；
    // 手写的 `>= && <=` 会被 clippy 判成 manual RangeInclusive::contains。
    let width = if lead < 0x80 {
        1
    } else if (0xC0..=0xDF).contains(&lead) {
        2
    } else if (0xE0..=0xEF).contains(&lead) {
        3
    } else if (0xF0..=0xF7).contains(&lead) {
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
///
/// `Default` 由 derive 生成：两个字段都是各自的零值（`None` / `false`），手写 impl
/// 只是把同一件事再说一遍（clippy::derivable_impls）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct AnsiStyle {
    pub fg: Option<egui::Color32>,
    pub bold: bool,
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

/// 接收区**筛选**：只保留**含 `needle` 的整行**，其余行整行丢弃。
///
/// 返回 `(要显示的文本, 命中行数, 总行数)`。三个刻意的选择：
///
/// * **`needle` 为空时原样借用**（`Cow::Borrowed`）⇒ 不复制、不扫描，关掉筛选这条
///   路径的开销是零。调用方**不需要**先判"筛不筛"，直接调它即可。
/// * **按逻辑行（`\n`）判定，不按折行后的视觉行**：彩色与字节视图会按窗口宽度折行，
///   若按视觉行筛，同一个关键字在窄窗口里会把半行留下、把另外半行筛掉。
/// * **整行保留**（而不是只保留命中的那几个字符）：串口日志里一行往往是一个完整
///   读数，砍掉行内上下文反而没法看；时间戳前缀也一并保住。
///
/// 末尾**没有换行的半行**（设备正发到一半）同样参与匹配：含关键字就显示，否则连同
/// 不算一行。这就是"总行数"的口径——与 [`filter_span_lines`] 完全一致（有测试对拍）。
///
/// ⚠️ `needle` 不含换行（界面上的输入框是单行输入，敲不进 `\n`）。
pub(crate) fn filter_lines<'a>(
    text: &'a str,
    needle: &str,
) -> (std::borrow::Cow<'a, str>, usize, usize) {
    if needle.is_empty() {
        return (std::borrow::Cow::Borrowed(text), 0, 0);
    }

    let mut kept = String::new();
    let mut matched = 0usize;
    let mut total = 0usize;
    for line in text.split_inclusive('\n') {
        total += 1;
        if line.contains(needle) {
            matched += 1;
            kept.push_str(line);
        }
    }
    (std::borrow::Cow::Owned(kept), matched, total)
}

/// 同 [`filter_lines`]，但作用在**带样式的片段**上（彩色视图与字节查看视图渲染的是
/// 片段，不是纯文本）。
///
/// 返回 `(保留的片段, 命中行数, 总行数)`；`needle` 为空时返回空 `Vec`——调用方在筛选
/// 关闭时应当直接走原路径（渲染未筛的镜像），`Vec::new()` 是为了**不静默给出一份空
/// 视图**：真走到这里又拿到空结果，接收区会明显是空的，比"悄悄换了内容"好排查。
///
/// **样式必须保住**：颜色是这两个视图存在的理由，筛完丢了颜色等于换了视图。
/// 一个片段可以跨多行（一段着色文本里可能有若干 `\n`），所以按 `\n` 把片段切开、
/// 按逻辑行判定；**只有该行被保留时才把它的各段拼回去**，且相邻同样式的段会合并
/// （否则输出会被切成一行一小段，`LayoutJob` 的 section 数暴涨）。
pub(crate) fn filter_span_lines(
    spans: &[ColoredSpan],
    needle: &str,
) -> (Vec<ColoredSpan>, usize, usize) {
    if needle.is_empty() {
        return (Vec::new(), 0, 0);
    }

    /// 往一行里追加一段：样式相同就并进上一段，避免碎片化。
    fn push_piece(line: &mut Vec<ColoredSpan>, text: &str, style: AnsiStyle) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = line.last_mut()
            && last.style == style
        {
            last.text.push_str(text);
            return;
        }
        line.push(ColoredSpan {
            text: text.to_string(),
            style,
        });
    }

    let mut out: Vec<ColoredSpan> = Vec::new();
    let mut line: Vec<ColoredSpan> = Vec::new();
    let mut line_hit = false;
    let mut matched = 0usize;
    let mut total = 0usize;

    for span in spans {
        for piece in span.text.split_inclusive('\n') {
            let ends_line = piece.ends_with('\n');
            let body = if ends_line {
                &piece[..piece.len() - 1]
            } else {
                piece
            };
            if body.contains(needle) {
                line_hit = true;
            }
            push_piece(&mut line, body, span.style);

            if ends_line {
                total += 1;
                if line_hit {
                    matched += 1;
                    out.append(&mut line);
                    // 换行要补回去，否则所有命中的行会粘成一行。
                    push_piece(&mut out, "\n", span.style);
                } else {
                    line.clear();
                }
                line_hit = false;
            }
        }
    }

    // 末尾那半行（没有换行）与 `filter_lines` 的口径一致：算一行、参与匹配。
    if !line.is_empty() {
        total += 1;
        if line_hit {
            matched += 1;
            out.append(&mut line);
        }
    }

    (out, matched, total)
}

/// 一次接收区绘制的结果。
///
/// 除了"右键菜单有没有点「复制全部日志」"，还要把**这一帧结束时视口的位置**带出去：
/// [`crate::app::ComTool::receive_at_bottom`] 靠它更新，而它又决定下一帧能不能淘汰旧
/// 日志。把两件事放在一个结构里，是为了让"绘制"与"消费绘制结果"之间没有隐式的顺序
/// 依赖——调用方拿到什么就得处理什么。
pub(crate) struct ReceiveScrollOutcome {
    /// 右键菜单里点了「复制全部日志」。
    pub copy_all: bool,
    /// 绘制结束时视口是否贴着底部（见 [`is_at_bottom`]）。
    pub at_bottom: bool,
    /// 下一帧最多淘汰多少字符（见 [`trim_budget_per_frame`]）。
    pub trim_budget: usize,
    /// 可视区宽度（见 [`crate::app::ComTool::receive_viewport_w`]）。
    ///
    /// 折行视图要拿它排版被裁掉的那段文本——折行宽度不同，同一段文字的行数就不同，
    /// 补偿量也就不同。不折行的视图用 `INFINITY`，这个字段对它没有意义。
    pub viewport_w: f32,
    /// 本帧绘制**结束时**的纵向滚动位置（见
    /// [`crate::app::ComTool::receive_last_offset_y`]）。
    pub offset_y: f32,
    /// 本帧实测的"每字符多少像素高"（`内容高 / 字符数`）。
    ///
    /// 把"视口顶行之上有多少像素"换算成"还能安全裁多少字符"要用它（见
    /// [`crate::app::ComTool::receive_px_per_char`]）。0.0 = 这一帧量不出来。
    pub px_per_char: f32,
}

/// 视口是否贴着底部（或内容根本装得下，没有可翻的历史）。
///
/// `max_offset` 是 egui 允许的最大滚动位置（`content_size - inner_rect`）；内容比可视
/// 区矮时它是负数，此时一律算贴底——用户眼前就是全部内容，没有"上面的历史"这回事。
///
/// 容差 1 点：egui 在贴底时把 `offset` 直接写成 `content_size - inner_rect.size()`
/// （`containers/scroll_area.rs` 里 `stick_to_end` 分支），但内容高度每帧都在变，
/// 浮点比较不适合要求严格相等。
fn is_at_bottom(offset_y: f32, content_h: f32, viewport_h: f32) -> bool {
    let max_offset = (content_h - viewport_h).max(0.0);
    offset_y >= max_offset - 1.0
}

/// 下一帧最多淘汰多少字符，由上一帧实测的三个数推出。
///
/// 约束只有一个：**一帧内的"缩短量"必须明显小于可视区高度**。因为 egui 用上一帧的滚动
/// 位置给内容布局，把 offset 夹回合法范围发生在内容画完之后——一帧里缩短超过一个可视区
/// 高度，这一帧的内容就整个落到视口上方，用户看到接收区空一帧。
///
/// 取**半个可视区**做安全边界：按每字符高换算成字符数，于是被裁掉的旧内容至多占半屏，
/// 内容绝不会被推出视口。字符高度由"上一帧内容高 / 上一帧字符数"实测，因此字体档位
/// （`ui_font_scale`）与窄窗口都自动跟上——写死"12 行"这类行数在窄窗口下会失效。
///
/// 数据不可用时（还没画过、可视区高为 0、内容为空）返回 `usize::MAX` = 不限制：宁可
/// 空一帧也不能让"读历史不淘汰"把内存上限放开。
fn trim_budget_per_frame(content_h: f32, chars: usize, viewport_h: f32) -> usize {
    if chars == 0 || content_h <= 0.0 || viewport_h <= 0.0 {
        return usize::MAX;
    }
    let px_per_char = content_h / chars as f32;
    let half_viewport = viewport_h * 0.5;
    if px_per_char <= 0.0 {
        return usize::MAX;
    }
    (half_viewport / px_per_char) as usize
}

/// 绘制接收区滚动视图（可选中、右键复制全部日志）。
///
/// `stick_to_bottom` 由 [`crate::app::ComTool::auto_scroll`] 决定：关掉自动滚动后，
/// 用户向上翻阅历史时不会被强行拉回底部。**注意"滚轮往上会解粘"是 egui 自己做的**
/// （`stick_to_bottom` 的文档原文：*"The scroll handle remains stuck until user manually
/// changes position"*，实现见 `containers/scroll_area.rs` 里把
/// `state.scroll_stuck_to_end` 置 false 的那几处），本项目不需要额外实现——需要的是
/// **别在用户读历史时把内容从他眼皮底下裁掉**，那件事在 [`ComTool::trim_receive_logs`]。
///
/// `copy_all_label` 是右键菜单那一项的文案。这两个接收视图函数是**自由函数**（不持有
/// `ComTool`），所以文案只能由调用方传进来——`Language::strings()` 是 `&'static`，
/// 传 `&str` 没有额外开销。
///
/// `offset_y` 是本帧被裁掉旧内容后的**补偿后**滚动位置（`None` = 这一帧没裁/不用补）。
/// 把它传给 `ScrollArea` 而不是自己写状态：egui 在 `show` 一开始就把它落到
/// `state.offset.y`（`containers/scroll_area.rs` 里 `offset_y.unwrap_or(state.offset.y)`），
/// 之后的贴底判定照旧跑——用户正在跟随最新内容时，贴底逻辑会覆盖这个补偿，方向正确。
fn show_receive_scroll(
    ui: &mut Ui,
    job: egui::text::LayoutJob,
    stick_to_bottom: bool,
    copy_all_label: &str,
    offset_y: Option<f32>,
) -> ReceiveScrollOutcome {
    let mut copy_all = false;
    // 字符数要和 `content_size` 对上：两者一个来自这份 job、一个来自它排版的结果。
    let job_chars = job.text.chars().count();
    let mut area = capped_scroll_area(
        ScrollAxis::Vertical,
        SCROLL_AREA_ID,
        RECEIVE_SCROLL_MAX_HEIGHT,
    )
    .stick_to_bottom(stick_to_bottom);
    if let Some(y) = offset_y {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| {
        let response = ui.add(egui::Label::new(job).wrap().selectable(true));
        response.context_menu(|ui| {
            if ui.button(copy_all_label).clicked() {
                copy_all = true;
                ui.close();
            }
        });
    });
    ReceiveScrollOutcome {
        copy_all,
        at_bottom: is_at_bottom(
            out.state.offset.y,
            out.content_size.y,
            out.inner_rect.height(),
        ),
        trim_budget: trim_budget_per_frame(out.content_size.y, job_chars, out.inner_rect.height()),
        viewport_w: out.inner_rect.width(),
        offset_y: out.state.offset.y,
        px_per_char: px_per_char(out.content_size.y, job_chars),
    }
}

/// 实测的"每字符多少像素高"。测不出来（内容为空 / 没画出来）时返回 0.0。
///
/// 这是一个**平均值**，只在"保阅读位置"里当换算尺用，误差方向是安全的：日志里短行
/// 多的时候它会偏高 ⇒ 换算出的可裁字符数偏小 ⇒ 少裁，不会裁到用户正在读的那一行。
fn px_per_char(content_h: f32, chars: usize) -> f32 {
    if chars == 0 || content_h <= 0.0 {
        return 0.0;
    }
    content_h / chars as f32
}

/// 普通文本模式接收区：不折行（Extend）+ 纵/横双向滚动，
/// 等宽 galley 绘制保证盲文 ASCII 图案整行对齐；可选中、右键复制全部日志。
///
/// 返回值见 [`ReceiveScrollOutcome`]：调用方拿它更新
/// [`crate::app::ComTool::receive_at_bottom`] 与 `receive_trim_budget`。
///
/// **默认文字色必须取自主题**（`ui.visuals().text_color()`），不能写死黑色：
/// 深色主题下接收区背景也是深的，黑字会直接看不见。
///
/// `offset_y` 见 [`show_receive_scroll`]：本帧裁掉旧内容后补偿过的滚动位置。
pub(crate) fn show_plain_receive(
    ui: &mut Ui,
    text: &str,
    stick_to_bottom: bool,
    copy_all_label: &str,
    offset_y: Option<f32>,
) -> ReceiveScrollOutcome {
    let text_chars = text.chars().count();
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE),
        ui.visuals().text_color(),
        f32::INFINITY,
    );
    let mut area = capped_scroll_area(ScrollAxis::Both, SCROLL_AREA_ID, RECEIVE_SCROLL_MAX_HEIGHT)
        .stick_to_bottom(stick_to_bottom);
    if let Some(y) = offset_y {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| {
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
    ReceiveScrollOutcome {
        copy_all: false, // 文本模式自己复制，不经右键菜单的返回值
        at_bottom: is_at_bottom(
            out.state.offset.y,
            out.content_size.y,
            out.inner_rect.height(),
        ),
        trim_budget: trim_budget_per_frame(out.content_size.y, text_chars, out.inner_rect.height()),
        // 不折行视图：宽度不参与补偿换算（用 `INFINITY` 排版），这个字段仅作记录。
        viewport_w: out.inner_rect.width(),
        offset_y: out.state.offset.y,
        px_per_char: px_per_char(out.content_size.y, text_chars),
    }
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

    /// 收下上一帧的绘制结果（见 [`ReceiveScrollOutcome`]）。三个接收视图共用这一处，
    /// "画完要记什么"就不会有一路漏记。
    fn note_receive_scroll(&mut self, out: &ReceiveScrollOutcome) {
        self.receive_at_bottom = out.at_bottom;
        self.receive_trim_budget = out.trim_budget;
        self.receive_viewport_w = out.viewport_w;
        self.receive_last_offset_y = Some(out.offset_y);
        self.receive_px_per_char = out.px_per_char;
    }

    /// 记下这一帧的筛选计数，供「筛选」开关旁边的指示器显示。
    ///
    /// 三条渲染路径（文本/HEX、彩色、字节）各算自己的那一份，但**行口径完全一致**
    /// ——三份镜像的文本内容逐字相同（见 `receive_data` 的落点），而筛选按逻辑行判定。
    /// 关掉筛选时传 `(0, 0)`：界面只在 `filter_text` 非空时才显示这个数。
    fn note_filter_counts(&mut self, matched: usize, total: usize) {
        self.filter_matched_lines = matched;
        self.filter_total_lines = total;
    }

    /// 把四份接收镜像各自裁剪到 [`crate::app::ComTool::max_log_chars`] 以内（0 = 不限）。
    ///
    /// 为什么是"各自裁剪"而不是"按 `receive_text` 裁好再同步"：三种模式写入的镜像
    /// 并不相同（HEX/文本模式写 `display_spans`，字节查看模式写 `ascii_spans`，而
    /// `receive_text` 三种模式都写），三者之间没有一一对应的下标关系。各自裁到同一
    /// 上限后，任何视图渲染时看到的长度都不会超过上限，这就够了。
    ///
    /// **上限不是常数，取决于视口在哪**（见下面的 `limit`）：用户翻到历史里读的时候
    /// 放宽，贴底时按"每帧一小步"追平欠账。这条规则的存在理由就是本次修复的缺陷
    /// （阅读位置被前沿淘汰一行行抽走，见 [`RECEIVE_TRIM_DEFER_ALLOWANCE`]）。
    pub fn trim_receive_logs(&mut self) {
        let max = self.max_log_chars;
        if max == 0 {
            return;
        }
        let len = self.receive_text.chars().count();
        // "用户正停在历史里读" = 接收区画着 + 视口不在底部。两条都成立才谈得上有人会
        // 被"抽走"——接收区没画时没人看，贴底时用户眼前本来就是最新内容。
        let parked = self.show_receive_panel && !self.receive_at_bottom;

        // 淘汰发生在**最旧**的一端，而它同时改变视口里内容的身份：滚动位置（像素）
        // 不变、内容却整体上移。用户贴着底部看最新数据时这无所谓；可他一旦往上翻着
        // 读历史，阅读位置就每帧被抽走一行——观感就是"自动滚动根本没停"。所以只在
        // **贴底**（或接收区根本没画：没人看，也就没人会被抽走）时才裁到上限。
        //
        // 不贴底时放宽到 `max + RECEIVE_TRIM_DEFER_ALLOWANCE`：那不是"不裁"，内存
        // 上限依然有界（长时间停在历史里 + 设备一直在喷数据 ⇒ 到额度后照旧裁，只是
        // 裁多少由下面两条规则决定）。
        let limit = if !self.show_receive_panel {
            // 接收区没画：没人看，也就没人会被"抽走"；而且这时量不到可视区大小，
            // 额度是无效的（默认值 = 不限制）⇒ 直接裁到上限，内存优先。
            max
        } else if self.receive_at_bottom {
            // 贴底：欠账要追平，但**每帧只追一部分**（`receive_trim_budget` = 上一帧
            // 实测的半个可视区能装多少字符）。一次裁完会让这一帧的内容整个跑到视口
            // 上方——用户看到接收区空一帧（见 [`trim_budget_per_frame`]）。
            max.max(len.saturating_sub(self.receive_trim_budget))
        } else {
            max.saturating_add(RECEIVE_TRIM_DEFER_ALLOWANCE)
        };

        let mut cut = len.saturating_sub(limit);

        // 读历史时再加两条规则（顺序要紧：先"保阅读位置"，再"保内存"）：
        //
        // ① **不许裁掉用户正在读的那一行**：最多裁到视口顶行为止。裁掉的都在视口
        //    上方，配合 [`ComTool::take_front_trim_compensation`] 把 offset 同步往回
        //    退，用户读的那一行原地不动。额度用尽之后靠这一条继续停住——否则每帧从
        //    最旧端淘汰，视口顶行会被逐帧换成更晚的行（本次缺陷的形态）。
        // ② **内存兜底**：涨过硬上限之后恢复淘汰。`max()` 放最后，保证它不被 ① 抵消。
        // 会不会走补偿：只有"量得出视口顶行之上还有多少"才谈得上把 offset 同步往回退。
        let mut compensating = false;
        if parked {
            if let Some(headroom) = self.reading_headroom_chars() {
                cut = cut.min(headroom);
                compensating = true;
            }
            cut = cut.max(len.saturating_sub(self.parked_hard_cap()));
        } else {
            // 不读历史时没人需要补偿：留着只会在下一次绘制时被当成"刚裁掉的一段"。
            self.receive_trimmed_head.clear();
        }

        // `receive_text` 这一份额外做两件事，其余三份照"实际裁掉多少"裁：
        //
        // ① **对齐到行首**：把截口往回收到上一个 `\n` 之后（见
        //    [`ComTool::floor_cut_to_line_boundary`]）。只裁完整行，排版被裁的那段得到的
        //    高度才等于"这些行在屏幕上占的高度"——截在半行上，剩下那半行照样占一整行，
        //    补偿就会算过头，视口被凭空往上推。
        // ② **留下被裁的那段原文**，供下一次绘制补偿滚动位置（见
        //    [`ComTool::take_front_trim_compensation`]）。
        //
        // 只有"有人正在看"才留：接收区没画时没人会被抽走，留着等于养第二份日志。
        // 量不出视口（还没画过）时 `compensating` 为 false ⇒ 没有补偿 ⇒ 也就不必对齐，
        // `applied` 保持等于 `cut`，照常规裁到上限。
        let mut applied = cut;
        if cut > 0 && parked && compensating {
            // 量得出视口 ⇒ 一定要对齐；不足一行就这一帧干脆不裁（宁可让内存涨，
            // 也不能用半行把用户正在读的那一行吃掉——那正是漂移的来源）。
            let aligned = Self::floor_cut_to_line_boundary(&self.receive_text, cut);
            if aligned > 0 {
                applied = aligned;
                self.receive_trimmed_head
                    .push_str(&self.receive_text.chars().take(aligned).collect::<String>());
            } else if len > self.parked_hard_cap() {
                // 例外：硬上限已经压不住，而这一段又对不齐行首（设备发的是不带换行的
                // 长流）⇒ 照原样裁。此时阅读位置会跳一次，但内存不能无界增长。
                applied = cut;
            } else {
                applied = 0;
            }
        }
        let keep = len.saturating_sub(applied);
        trim_text_front(&mut self.receive_text, keep);
        trim_spans_front(&mut self.display_spans, keep);
        trim_text_front(&mut self.ascii_text, keep);
        trim_spans_front(&mut self.ascii_spans, keep);
    }

    /// 读历史时的硬上限（见 [`RECEIVE_PARKED_EXTRA_CAP`]）。
    fn parked_hard_cap(&self) -> usize {
        self.max_log_chars.saturating_add(RECEIVE_PARKED_EXTRA_CAP)
    }

    /// 视口顶行**之上**还有多少字符可以安全裁掉。
    ///
    /// = 上一帧的滚动位置换算成字符数。返回 `None` = 量不出来（还没画过、或内容为空），
    /// 此时不做"保阅读位置"的收紧——宁可让硬上限兜底，也不要用一个假的 0 把淘汰全停掉。
    fn reading_headroom_chars(&self) -> Option<usize> {
        let px_per_char = self.receive_px_per_char;
        if px_per_char <= 0.0 {
            return None;
        }
        let offset = self.receive_last_offset_y?;
        Some((offset / px_per_char) as usize)
    }

    /// 把"要裁掉的字符数"往回收到**上一个换行之后**，使截口落在某行的行首。
    ///
    /// 方向是**宁可少裁**：裁到用户正在读的那一行中间（哪怕只是半行），补偿就会把
    /// offset 压到 0 以下被夹住，那一行当场被换掉——漂移正是从这里来的。
    ///
    /// 收不到行首时（要裁的还不到第一行，或整段压根没有换行）**照原样返回 `cut`**：
    /// 收不到行首时（要裁的还不到一行，或整段压根没有换行）返回 **0 = 这一帧不裁**。
    ///
    /// 为什么不是"照原样裁"：实测过——每帧切掉 29 个字符（差一个字符就够一行）时，
    /// 每一次切都会把最前面那一行的绝大部分连同它前面的换行一起吃掉，于是"视口顶行"
    /// 逐帧被换成下一行，正是要治的漂移。宁可让内存涨（上面还有硬上限兜着），也不能
    /// 用半行把用户正在读的那一行吃掉。
    fn floor_cut_to_line_boundary(text: &str, cut: usize) -> usize {
        let mut last_break = 0usize;
        for (seen, ch) in text.chars().enumerate() {
            if ch != '\n' {
                continue;
            }
            let after = seen + 1;
            if after > cut {
                break;
            }
            last_break = after;
        }
        last_break
    }

    /// 取走本帧被裁掉的那段原文，换算成"滚动位置该往回退多少像素"。
    ///
    /// 返回 `None` = 这次不做补偿（没有裁过、或量不出宽度）。**宁可不补也不要乱补**：
    /// 少补一次只是阅读位置慢半拍，补错了方向（补过头）反而是持续倒退，更难看。
    ///
    /// `wrap_width` 必须与接收区实际用的折行宽度一致：普通文本/HEX 视图是
    /// `f32::INFINITY`（不折行），彩色与字节查看视图是上一帧的可视区宽度
    /// （见 [`ComTool::receive_viewport_w`]）。
    fn take_front_trim_compensation(&mut self, ui: &Ui, wrap_width: f32) -> Option<f32> {
        let head = std::mem::take(&mut self.receive_trimmed_head);
        // **只补偿"整行被裁"**：截口落在行中间时，剩下的半行还占着完整一行高 ⇒ 内容
        // 高度根本没变 ⇒ 补偿反而是凭空把视口往上推（排版那段半行会给出一个整行的高度）。
        // 对齐过 ⇒ 被裁的那段以 `\n` 结尾，这就是"整行"的凭据。
        if !head.ends_with('\n') {
            return None;
        }
        // 末尾那个 `\n` 不属于任何一行的内容高度——排版时带着它，多出来的空行会让
        // 补偿过头。
        let head = head.trim_end_matches('\n');
        if head.is_empty() || wrap_width <= 0.0 {
            return None;
        }
        let font_id = monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE);
        let galley = ui.fonts_mut(|f| {
            f.layout(
                head.to_owned(),
                font_id,
                ui.visuals().text_color(),
                wrap_width,
            )
        });
        let height = galley.size().y;
        if height <= 0.0 {
            return None;
        }
        // 内容上移了 `height`，视口要指着原来那一行就得同步往回退 `height`。
        //
        // 基准取自上一次绘制的结果（见 [`crate::app::ComTool::receive_last_offset_y`]
        // 里记的坑：这里读 `State::load` 恒为 `None`，会把视口钉死在顶部）。
        let base = self.receive_last_offset_y?;
        Some((base - height).max(0.0))
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
        self.reset_receive_scroll(ui);
    }

    /// 把接收区的滚动状态复位到"最新"。
    ///
    /// **两处共用**：「清空日志」与「筛选关键字变了」。共同点是**显示的内容被整体换掉
    /// 了**——保持旧的滚动位置没有意义，而且筛选后内容变短，旧 `offset` 可能越过新内容
    /// 的底部，视口会落在空白处（看着像"筛选把日志筛没了"）。
    ///
    /// 顺带把 `receive_at_bottom` 置回 `true`：内容换了 ⇒ 必然贴底。不清这一位的话，
    /// 用户"停在历史里 → 清空日志 / 改筛选"之后仍处于放宽状态，淘汰上限要等到下一次
    /// 绘制才恢复。
    pub(crate) fn reset_receive_scroll(&mut self, ui: &egui::Ui) {
        let id = ui.make_persistent_id(SCROLL_AREA_ID);
        egui::scroll_area::State::default().store(ui.ctx(), id);
        self.receive_at_bottom = true;
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
                // 这个视图**不折行** ⇒ 排版被裁文本时用 `INFINITY`。
                // `receive_data()` 已经在本帧更早处跑过（`ComTool::ui` 开头），裁剪
                // 发生在这里之前，所以补偿取到的是完整的一段。
                let offset_y = self.take_front_trim_compensation(ui, f32::INFINITY);
                // 筛选是**显示层**的：被筛掉的行仍然留在 `receive_text` 里，导出、
                // 实时日志文件、图表解析都不受影响。关掉筛选时这里是 `Cow::Borrowed`
                // ⇒ 不复制、不扫描。
                let (shown, matched, total) = filter_lines(&self.receive_text, &self.filter_text);
                let out = show_plain_receive(ui, &shown, self.auto_scroll, copy_all_log, offset_y);
                self.note_receive_scroll(&out);
                self.note_filter_counts(matched, total);
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
        let default_color = ui.visuals().text_color();
        let font_id = monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE);
        let (job, matched, total) = if self.filter_text.is_empty() {
            // 关掉筛选：走原路径，不复制整份片段表
            (self.build_layout(default_color, font_id), 0, 0)
        } else {
            let (kept, matched, total) = filter_span_lines(&self.display_spans, &self.filter_text);
            (
                layout_job_from(&kept, default_color, font_id),
                matched,
                total,
            )
        };

        // ③ 在函数内部绘制滚动区，替换掉主界面里默认的滚动区
        //
        // 补偿放在 ①（`receive_data`，裁剪在那里面）**之后**：先裁才谈得上补。
        // 这个视图会折行 ⇒ 按上一帧的可视区宽度排版被裁文本。
        let offset_y = self.take_front_trim_compensation(ui, self.receive_viewport_w);
        let out = show_receive_scroll(
            ui,
            job,
            self.auto_scroll,
            self.language.strings().copy_all_log,
            offset_y,
        );
        self.note_receive_scroll(&out);
        self.note_filter_counts(matched, total);
        if out.copy_all {
            // 复制的是**屏幕上那一份**（筛选后）——与文本/HEX 模式一致（那里 `show_plain_receive`
            // 复制的就是它渲染的那份文本）。导出与实时日志文件仍是不筛选的完整原文。
            let (shown, _, _) = filter_lines(&self.receive_text, &self.filter_text);
            ui.ctx().copy_text(shown.into_owned());
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
        let default_color = ui.visuals().text_color();
        let font_id = monospace_font_id(ui, RECEIVE_MONOSPACE_BASE_SIZE);
        let (job, matched, total) = if self.filter_text.is_empty() {
            (self.build_ascii_layout(default_color, font_id), 0, 0)
        } else {
            let (kept, matched, total) = filter_span_lines(&self.ascii_spans, &self.filter_text);
            (
                layout_job_from(&kept, default_color, font_id),
                matched,
                total,
            )
        };

        // ③ 绘制滚动区（样式与彩色模式一致）。补偿同上：在 `receive_data` 之后、
        //    绘制之前，按折行宽度换算。
        let offset_y = self.take_front_trim_compensation(ui, self.receive_viewport_w);
        let out = show_receive_scroll(
            ui,
            job,
            self.auto_scroll,
            self.language.strings().copy_all_log,
            offset_y,
        );
        self.note_receive_scroll(&out);
        self.note_filter_counts(matched, total);
        if out.copy_all {
            let (shown, _, _) = filter_lines(&self.receive_text, &self.filter_text);
            ui.ctx().copy_text(shown.into_owned());
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

    /// 测试用日志行：**定长**（含换行共 30 字符）。
    ///
    /// 定长是这批测试的算术前提：`trim_text_front` 按字符裁，只有"每行一样长"才能把
    /// "裁掉多少字符"整除成"裁掉多少行"，视口顶行的绝对行号才有唯一答案。
    const LOG_LINE_CHARS: usize = 30;

    fn log_line(n: usize) -> String {
        format!("line {n:06} {}\n", "x".repeat(LOG_LINE_CHARS - 13))
    }

    /// 一帧接收区绘制里，**用户真正看得见的东西**。
    struct PaintedLog {
        /// 视口顶部压着的那一行在**当前 galley 里**的行号。
        top_row: usize,
        /// 接收区可视区高度。
        viewport_h: f32,
        /// 内容底部 − 可视区底部。≈ 0 表示已经贴到最新内容；越大说明下面的内容越多。
        bottom_gap: f32,
    }

    /// 从**真实绘制出来的文字 shape** 里读出视口状态。
    ///
    /// 为什么不读 `ScrollArea` 的 `offset`：`offset` 不变**不等于**阅读位置不动——这正是
    /// 本次缺陷的形态（淘汰发生在最旧端 ⇒ 内容上移、像素位置不变），拿 offset 做判据
    /// 会一路绿到底。`t.pos` 是含滚动位移后的落笔位置，行矩形由 galley 给出，两者相减
    /// 得到的就是屏幕坐标，再用该 shape 的 `clip_rect`（= 接收区可视区）去截。
    fn painted_log(out: &egui::FullOutput) -> PaintedLog {
        let mut drawn: Vec<String> = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(t) = &clipped.shape {
                drawn.push(t.galley.text().chars().take(20).collect());
                // 认出日志那一份：行数远多于这一帧里任何其它文字，且正文里必然出现整行
                // 标记。**不能**要求文本以 "line " 开头——分帧追平欠账时截口落在行中间，
                // 第一行是半行。
                if t.galley.rows.len() < 10 || !t.galley.text().contains("line ") {
                    continue;
                }
                let viewport_top = clipped.clip_rect.top();
                let top_row = t
                    .galley
                    .rows
                    .iter()
                    .position(|r| t.pos.y + r.rect().max.y > viewport_top + 1.0)
                    .expect("日志 galley 里应当至少有一行落在可视区内");
                return PaintedLog {
                    top_row,
                    viewport_h: clipped.clip_rect.height(),
                    bottom_gap: t.pos.y + t.galley.rect.height() - clipped.clip_rect.bottom(),
                };
            }
        }
        panic!("这一帧没有画出接收区日志文本；实际画出的文字 shape 有 {drawn:?}");
    }

    /// [`is_at_bottom`] 的三态：内容装得下 / 贴在底部 / 翻到了历史里。
    #[test]
    fn is_at_bottom_covers_the_three_interesting_positions() {
        // 内容比可视区矮：没有可翻的历史，一律算贴底（max_offset 被夹到 0）。
        assert!(is_at_bottom(0.0, 100.0, 400.0));
        // 内容超出：贴底 = offset 用满。
        assert!(is_at_bottom(600.0, 1000.0, 400.0));
        // 往上翻了一点就不算贴底——差 1 点以内仍算（egui 的浮点比较容差）。
        assert!(!is_at_bottom(500.0, 1000.0, 400.0));
        assert!(is_at_bottom(599.5, 1000.0, 400.0));
    }

    /// [`trim_budget_per_frame`]：一帧的淘汰额度必须**明显小于一个可视区**能装的字符数。
    ///
    /// 这是"不许出现空一帧"的量化形式：egui 用上一帧的 offset 布局，一帧里缩短超过一个
    /// 可视区高度，内容就整个跑到视口上方。
    #[test]
    fn trim_budget_stays_well_under_one_viewport() {
        // 实测口径（见 `wheel_up_freezes_the_reading_position` 里的真实数字）：
        // 5850 字符 ≈ 3151 点高，可视区 400 点 ⇒ 每字符约 0.54 点。
        let budget = trim_budget_per_frame(3_151.0, 5_850, 400.0);
        assert!(
            budget < 400,
            "额度 {budget} 字符 ≈ 215 点高，已经接近可视区高度 400 点，不够安全"
        );
        assert!(budget > 100, "额度 {budget} 太小，追平欠账会慢到跟不上数据");

        // 字体放大 / 窗口变矮都通过实测值自动跟上：视口矮一半 ⇒ 额度也减半。
        let half = trim_budget_per_frame(3_151.0, 5_850, 200.0);
        assert!(
            half < budget && half > 50,
            "视口减半后额度应同步变小：{half}"
        );

        // 窄到量不出东西时不能返回 0（那会让上限失效、内存无界增长）。
        assert_eq!(trim_budget_per_frame(0.0, 0, 400.0), usize::MAX);
        assert_eq!(trim_budget_per_frame(3_151.0, 5_850, 0.0), usize::MAX);
    }

    /// 用户翻到历史里阅读时，旧内容**先不淘汰**；贴底后立刻恢复原上限。
    ///
    /// 这里钉的是**结构性规则**（放宽的额度有多大、什么时候恢复），"阅读位置是否真的
    /// 停住"由 [`wheel_up_freezes_the_reading_position`] 从绘制结果上钉——两条缺一不可：
    /// 只有这一条时，把"读历史"的判定接到别的东西上照样绿。
    #[test]
    fn front_trim_is_deferred_while_the_user_reads_history() {
        const MAX: usize = 1_000;
        let mut app = ComTool {
            max_log_chars: MAX,
            ..ComTool::default()
        };

        // 贴底（默认）：裁到上限。这是原有契约，不能被"放宽"顺手改掉。
        assert!(app.receive_at_bottom, "默认应当贴底");
        app.receive_text = "x".repeat(MAX + 5_000);
        app.trim_receive_logs();
        assert_eq!(app.receive_text.chars().count(), MAX);

        // 翻到历史里：一个字符都不裁。
        app.receive_at_bottom = false;
        app.receive_text = "x".repeat(MAX + 5_000);
        app.trim_receive_logs();
        assert_eq!(
            app.receive_text.chars().count(),
            MAX + 5_000,
            "用户正在读历史，却把旧内容裁掉了——阅读位置会被一行行抽走"
        );

        // 放宽有额度：到顶后照旧淘汰，内存不能无界增长。
        app.receive_text = "x".repeat(MAX * 100);
        app.trim_receive_logs();
        assert_eq!(
            app.receive_text.chars().count(),
            MAX + RECEIVE_TRIM_DEFER_ALLOWANCE,
            "额度用满后应当停在「上限 + 额度」，而不是继续不裁"
        );

        // 回到贴底 → 恢复原有上限……
        app.receive_at_bottom = true;
        app.trim_receive_logs();
        assert_eq!(app.receive_text.chars().count(), MAX);

        // ……但追平欠账要分帧：一帧最多淘汰 `receive_trim_budget` 个字符，否则一帧里
        // 缩短超过一个可视区高度，内容会被画到视口上方（用户看到空一帧）。
        app.receive_at_bottom = true;
        app.receive_trim_budget = 100;
        app.receive_text = "x".repeat(MAX + 5_000);
        app.trim_receive_logs();
        assert_eq!(
            app.receive_text.chars().count(),
            MAX + 4_900,
            "一帧应只淘汰 100 个字符，欠账留给后面几帧"
        );

        // 接收区被藏起来时同样裁到上限：没人看，也就没人会被"抽走"，不能让上限失效。
        app.receive_at_bottom = false;
        app.show_receive_panel = false;
        app.receive_text = "x".repeat(MAX * 100);
        app.trim_receive_logs();
        assert_eq!(app.receive_text.chars().count(), MAX);
    }

    /// **本次缺陷的回归测试**：日志到上限、设备还在持续喷数据时，用户用滚轮往上翻，
    /// 屏幕上的内容必须**停住**——不是"滚动条不再回到底部"就算完。
    ///
    /// 走真实路径与真实时序：`ComTool::render_receive_area` → `show_plain_receive` →
    /// `capped_scroll_area`；每帧先"追加 + 裁剪"（对应 `ComTool::ui` 里 `receive_data()`
    /// 的位置）再绘制。唯一被替代的是数据来源：真实数据要经事件通道，那条路另有测试。
    ///
    /// 空转验证：把 `trim_receive_logs` 里"读历史时放宽"那一支删掉（改回一律裁到
    /// `max`），下面的断言 ① 会每帧前进 5 行而变红。
    #[test]
    fn wheel_up_freezes_the_reading_position() {
        const MAX: usize = 3_000; // 100 行 × 30 字符
        const LINES_PER_FRAME: usize = 5;
        const WHEEL_UP_FRAME: usize = 6;
        const WHEEL_DOWN_FIRST: usize = 20;
        const WHEEL_DOWN_LAST: usize = 25;
        // 滚回底部之后还要留够帧数让"分帧追平欠账"跑完（额度约 371 字符/帧、欠账
        // 约 2 850 字符 ⇒ 十几帧），最后一帧才谈得上"内容重新贴住底边"。
        const LAST_FRAME: usize = 44;

        let mut app = ComTool {
            max_log_chars: MAX,
            ..ComTool::default()
        };

        // 先装满：观察场景是"已经跑了一阵子的日志"，不是刚开始收。
        let mut next_line = 0usize;
        let mut appended_chars = 0usize;
        while appended_chars < MAX {
            app.receive_text.push_str(&log_line(next_line));
            next_line += 1;
            appended_chars += LOG_LINE_CHARS;
        }

        let ctx = egui::Context::default();
        let mut reading = Vec::new();
        let mut following = Vec::new();
        let mut reading_flags = Vec::new();
        let mut following_flags = Vec::new();
        let mut viewport_h = 0.0_f32;
        let mut last_gap = 0.0_f32;

        for frame in 0..=LAST_FRAME {
            // —— 真实时序的第一步：追加 + 裁剪（`receive_data` 里做的那两件事）——
            for _ in 0..LINES_PER_FRAME {
                app.receive_text.push_str(&log_line(next_line));
                next_line += 1;
                appended_chars += LOG_LINE_CHARS;
            }
            app.trim_receive_logs();

            // —— 真实时序的第二步：绘制（滚轮事件只在这一步进来）——
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(200.0, 100.0))];
            let delta = if frame == WHEEL_UP_FRAME {
                50.0
            } else if (WHEEL_DOWN_FIRST..=WHEEL_DOWN_LAST).contains(&frame) {
                -400.0
            } else {
                0.0
            };
            if delta != 0.0 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, delta),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| app.render_receive_area(ui));
            let painted = painted_log(&out);
            // 清纹理：不清理会在 drop 时 panic（与断言无关的清理问题，不是缺陷）。
            out.textures_delta.clear();

            // 视口顶行的**绝对**行号 = 已经被裁掉的行数 + 落在可视区顶部的行号。
            //
            // 整除在"分帧追平"期间不精确（按额度裁剩的截口可能落在行中间，顶行是半行），
            // 但下面的断言只看**逐帧的前进/后退方向**，系统性偏差不影响结论；而"往上翻
            // 之后必须停住"那一段（读历史，完全不裁）恰好是整除精确的。
            let trimmed_lines =
                (appended_chars - app.receive_text.chars().count()) / LOG_LINE_CHARS;
            let top_line = trimmed_lines + painted.top_row;
            viewport_h = painted.viewport_h;
            last_gap = painted.bottom_gap;

            if (WHEEL_UP_FRAME..WHEEL_DOWN_FIRST).contains(&frame) {
                reading.push(top_line);
                reading_flags.push(app.receive_at_bottom);
            }
            if frame > WHEEL_DOWN_LAST {
                following.push(top_line);
                following_flags.push(app.receive_at_bottom);
            }
        }

        // 前提检查：可视区确实被夹住了、而且确实装不下全部内容——否则"视口顶行"根本
        // 不会动，下面两条断言都会变成空转。
        assert!(
            viewport_h > 50.0 && viewport_h < 450.0,
            "接收区可视区高 {viewport_h:.1} 点，与预期不符（高度上限 400）"
        );
        assert!(
            following.len() >= 5 && reading.len() >= 8,
            "样本不足，测不出什么：reading={reading:?} following={following:?}"
        );

        // ⓪ 先钉住"绘制结果 → 状态"这一步确实接上了：滚轮往上那一段必须被记成"没贴底"，
        //    滚回底部那一段必须被记成"贴底"。它单独成立的意义是——上面两条断言都依赖
        //    `receive_at_bottom` 被真实更新，如果哪天忘了把绘制结果写回字段（而恰好
        //    默认值是 `true`），表现会是"读历史时又在裁"，而这条会第一时间指认原因。
        assert!(
            !reading_flags.iter().any(|at_bottom| *at_bottom),
            "滚轮往上之后 receive_at_bottom 仍然是 true：绘制结果没有写回字段？{reading_flags:?}"
        );
        assert!(
            following_flags.iter().all(|at_bottom| *at_bottom),
            "滚回底部之后 receive_at_bottom 没恢复成 true：{following_flags:?}"
        );

        // ① 往上翻之后这几帧，视口顶行只许往回走（看到更早的内容），**不许往前走**。
        //    缺陷复活时这里每帧 +5 行：`trim_receive_logs` 从最旧端淘汰，滚动位置不变
        //    而内容整体上移，用户读到的那一行就会不断被换成更晚的行。
        assert!(
            reading.windows(2).all(|w| w[1] <= w[0]),
            "滚轮往上之后阅读位置还在被往前推（视口顶行逐帧）：{reading:?}"
        );

        // ② 滚回底部 → 恢复跟随：视口顶行重新往前走，且底部内容重新贴住可视区底边。
        assert!(
            *following.last().unwrap() > following.first().unwrap() + 3,
            "滚回底部后没有恢复跟随（视口顶行不再前进）：{following:?}"
        );
        assert!(
            last_gap.abs() < 1.0,
            "滚回底部后内容底部与可视区底边差 {last_gap:.1} 点，说明没真的贴底"
        );
    }

    /// [`ComTool::floor_cut_to_line_boundary`]：方向必须是"宁可少裁"，而且收不到行首时
    /// **不裁**而不是照原样裁。
    ///
    /// 半行回退的代价是实测出来的（见那条函数的注释）：差一个字符就够一行时，每切一次
    /// 都会把最前面那一行吃掉 ⇒ 视口顶行逐帧被换成下一行 ⇒ 正是要治的漂移。
    #[test]
    fn floor_cut_never_cuts_into_a_partial_line() {
        // 每行 4 字符（含换行）。
        let text = "aaa\nbbb\nccc\n";
        assert_eq!(ComTool::floor_cut_to_line_boundary(text, 8), 8); // 正好两行
        assert_eq!(ComTool::floor_cut_to_line_boundary(text, 10), 8); // 收回到第二行末
        assert_eq!(ComTool::floor_cut_to_line_boundary(text, 3), 0); // 不足一行 ⇒ 不裁
        assert_eq!(ComTool::floor_cut_to_line_boundary("", 10), 0);
        // 整段没有换行（设备一直发不带换行的数据）：对不齐 ⇒ 这一帧不裁，让硬上限兜底。
        assert_eq!(
            ComTool::floor_cut_to_line_boundary("x".repeat(100).as_str(), 40),
            0
        );
    }

    /// 读历史时的硬上限**不会**因为"对不齐行首"而失效。
    ///
    /// 场景：设备发的是不带换行的长流（`floor_cut_to_line_boundary` 收不到行首），而
    /// 用户正停在历史里。这时"保阅读位置"与"对齐"都做不到 ⇒ 内存优先，照原样裁。
    #[test]
    fn parked_hard_cap_still_trims_when_no_line_boundary_exists() {
        const MAX: usize = 1_000;
        let mut app = ComTool {
            max_log_chars: MAX,
            ..ComTool::default()
        };
        let cap = MAX + RECEIVE_PARKED_EXTRA_CAP;
        // 停在历史里，且视口顶行之上已经没有可裁的余量（headroom = 0）。
        app.show_receive_panel = true;
        app.receive_at_bottom = false;
        app.receive_last_offset_y = Some(0.0);
        app.receive_px_per_char = 0.5;
        // 整段没有换行 —— 对不齐行首的那种日志。
        app.receive_text = "x".repeat(cap + 5_000);
        app.trim_receive_logs();
        assert_eq!(
            app.receive_text.chars().count(),
            cap,
            "硬上限失效了：没有换行的日志在读历史时永远不裁，内存会无界增长"
        );
    }

    /// **本次缺陷的回归测试**：用户停在历史里读日志、设备一直在喷数据 —— 放宽额度
    /// （[`RECEIVE_TRIM_DEFER_ALLOWANCE`]）**用尽之后**，阅读位置也必须继续停住。
    ///
    /// 与 [`wheel_up_freezes_the_reading_position`] 的分工：那条钉"额度之内"（完全不裁）
    /// 的那一段，这条钉"额度之外"（照旧裁，靠补偿 + 不裁到视口顶行）的那一段。缺了这条，
    /// 用户的观感就是"停了一会儿又自己滚起来了，而且再往上滚也停不住"。
    ///
    /// "再往上滚也停不住"的成因单独记一笔：旧代码把 offset 一路压到 0 被 egui 夹住 ⇒
    /// 视口被钉在"最旧保留行"上，而那一行自己每帧都在变新 ⇒ 滚轮往上已经没有余量
    /// （已经在顶部），于是怎么滚都停不住。
    ///
    /// 空转验证（改完必须确认会红）：
    /// ① 删掉 `trim_receive_logs` 里 `cut = cut.min(headroom)` 那一行 ⇒ 断言 ① 每帧 +5 行；
    /// ② 让 [`ComTool::take_front_trim_compensation`] 恒返回 `None` ⇒ 断言 ① 同样变红。
    #[test]
    fn parked_reading_position_holds_after_the_allowance_runs_out() {
        const MAX: usize = 3_000; // 100 行 × 30 字符
        const LINES_PER_FRAME: usize = 5;
        const WHEEL_UP_FRAME: usize = 6;
        const WHEEL_UP_AGAIN: usize = 240; // 额度早该用尽，再往上滚一次
        const LAST_FRAME: usize = 300;

        let mut app = ComTool {
            max_log_chars: MAX,
            ..ComTool::default()
        };
        let mut next_line = 0usize;
        let mut appended_chars = 0usize;
        while appended_chars < MAX {
            app.receive_text.push_str(&log_line(next_line));
            next_line += 1;
            appended_chars += LOG_LINE_CHARS;
        }

        let ctx = egui::Context::default();
        // 每帧记 (视口顶行的绝对行号, 已淘汰行数, 当前字符数, 是否贴底)
        let mut rows: Vec<(i64, i64, usize, bool)> = Vec::new();
        let mut max_len = 0usize;

        for frame in 0..=LAST_FRAME {
            for _ in 0..LINES_PER_FRAME {
                app.receive_text.push_str(&log_line(next_line));
                next_line += 1;
                appended_chars += LOG_LINE_CHARS;
            }
            app.trim_receive_logs();

            let mut events = vec![egui::Event::PointerMoved(egui::pos2(200.0, 100.0))];
            let delta = if frame == WHEEL_UP_FRAME {
                50.0
            } else if frame == WHEEL_UP_AGAIN {
                400.0
            } else {
                0.0
            };
            if delta != 0.0 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, delta),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| app.render_receive_area(ui));
            let painted = painted_log(&out);
            out.textures_delta.clear();

            let trimmed = (appended_chars - app.receive_text.chars().count()) / LOG_LINE_CHARS;
            let len = app.receive_text.chars().count();
            max_len = max_len.max(len);
            rows.push((
                trimmed as i64 + painted.top_row as i64,
                trimmed as i64,
                len,
                app.receive_at_bottom,
            ));
        }

        // 读历史那一段的窗口：从"滚轮往上"之后一帧起，到最后一帧。
        let window = &rows[WHEEL_UP_FRAME + 1..];
        let tops: Vec<i64> = window.iter().map(|(top, _, _, _)| *top).collect();
        let flags: Vec<bool> = window
            .iter()
            .map(|(_, _, _, at_bottom)| *at_bottom)
            .collect();

        // ⓪ 前提一：这一段**不是**"自动滚动"在起作用——全程都没贴底。这条同时把用户
        //    那个疑问（"是不是我勾选了自动滚动的原因"）钉成了可执行的判据：不是。
        assert!(
            !flags.iter().any(|at_bottom| *at_bottom),
            "读历史这一段被记成了贴底：绘制结果没写回字段，或视口被拉回了底部"
        );

        // ⓪ 前提二：**确实吃满了额度，而且额度用尽之后淘汰仍在发生**。没有这一条，
        //    下面"没漂移"可能只是因为根本没裁——那才是真正的空转。
        let allowance = MAX + RECEIVE_TRIM_DEFER_ALLOWANCE;
        let exhausted_at = rows
            .iter()
            .position(|(_, _, len, _)| *len >= allowance)
            .unwrap_or_else(|| panic!("日志量始终没到放宽额度 {allowance} 字符：样本不足"));
        let trimmed_at_exhaust = rows[exhausted_at].1;
        let trimmed_at_end = rows.last().unwrap().1;
        assert!(
            trimmed_at_end > trimmed_at_exhaust,
            "额度用尽之后就不再淘汰了（淘汰 {trimmed_at_exhaust} → {trimmed_at_end} 行）：\
             这段窗口里根本没在裁，下面的断言测不到补偿"
        );

        // ① 主断言：整个读历史窗口里，视口顶行**一次都不许往前走**。缺陷复活时这里
        //    每帧 +5 行（每帧淘汰 5 行 ⇒ 视口顶行被换成 5 行之后的内容）。
        let advanced: Vec<i64> = tops.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            advanced.iter().all(|d| *d <= 0),
            "额度用尽之后阅读位置仍在前进（逐帧增量）：{advanced:?}"
        );

        // ② 主断言：首尾之间只允许"被滚轮往上带回去"，不许被推着走。第二次滚轮
        //    （400 点 ≈ 27 行）是唯一合法的倒退来源，留 40 行余量。
        let first = tops[0];
        let last = *tops.last().unwrap();
        assert!(
            last <= first && first - last <= 40,
            "读历史 {n} 帧之后视口顶行从 {first} 变成了 {last}（只允许被滚轮往上带回去）",
            n = tops.len()
        );

        // ③ 用户报告的第二条："再往上滚也停不住" —— 第二次滚轮之后仍不许前进。
        let after_second_wheel = &tops[(WHEEL_UP_AGAIN - WHEEL_UP_FRAME)..];
        assert!(
            after_second_wheel.windows(2).all(|w| w[1] <= w[0]),
            "第二次滚轮往上之后阅读位置又开始前进：{after_second_wheel:?}"
        );

        // ④ 内存兜底：读历史也不能让日志无界增长。
        assert!(
            max_len <= MAX + RECEIVE_PARKED_EXTRA_CAP,
            "日志涨到 {max_len} 字符，超过读历史时的硬上限 {}",
            MAX + RECEIVE_PARKED_EXTRA_CAP
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
    // ——— 接收区筛选（2026-09-27）———

    /// 整行保留：命中行连同它的换行一起留下，未命中行整行丢弃。
    #[test]
    fn filter_keeps_whole_lines_that_contain_the_needle() {
        let (kept, matched, total) = filter_lines(
            "12345
abc
x345y
",
            "345",
        );
        assert_eq!(
            kept.as_ref(),
            "12345
x345y
"
        );
        assert_eq!((matched, total), (2, 3), "命中 2 行 / 共 3 行");
    }

    /// 关掉筛选：**原样借用**、不复制整份日志，计数为 0。
    #[test]
    fn filter_borrows_the_text_untouched_when_the_needle_is_empty() {
        let text = String::from(
            "12345
abc
",
        );
        let (kept, matched, total) = filter_lines(&text, "");
        assert!(
            matches!(kept, std::borrow::Cow::Borrowed(_)),
            "筛关闭时不该复制整份日志（这条路径每帧都会走）"
        );
        assert_eq!(kept.as_ref(), text.as_str());
        assert_eq!((matched, total), (0, 0));
    }

    /// 末尾没有换行的半行同样参与匹配（设备正发到一半时不能当它不存在）。
    #[test]
    fn filter_keeps_a_trailing_partial_line() {
        let (kept, matched, total) = filter_lines(
            "abc
xy1", "1",
        );
        assert_eq!(kept.as_ref(), "xy1");
        assert_eq!((matched, total), (1, 2));
    }

    /// 区分大小写——界面上的说明就是这么写的，实现不能自己折叠。
    #[test]
    fn filter_is_case_sensitive() {
        let (kept, matched, total) = filter_lines(
            "ABC
abc
", "abc",
        );
        assert_eq!(
            kept.as_ref(),
            "abc
"
        );
        assert_eq!((matched, total), (1, 2));
    }

    /// 片段版本：颜色必须保住，且**保留的文本与行计数要和文本版本逐字一致**。
    ///
    /// 对拍是关键：文本/HEX 两个视图走 [`filter_lines`]，彩色/字节两个视图走
    /// [`filter_span_lines`]，两份实现口径一旦分叉，就会出现"同一个筛选在两个视图里
    /// 筛出不同的行"。
    #[test]
    fn span_filter_keeps_colours_and_agrees_with_the_text_filter() {
        let red = AnsiStyle {
            fg: Some(egui::Color32::RED),
            bold: false,
        };
        let spans = vec![
            ColoredSpan {
                text: "12345
"
                .to_string(),
                style: red,
            },
            ColoredSpan {
                text: "abc
"
                .to_string(),
                style: AnsiStyle::default(),
            },
            ColoredSpan {
                text: "x345y
"
                .to_string(),
                style: red,
            },
        ];
        let (kept, matched, total) = filter_span_lines(&spans, "345");

        assert_eq!(
            span_text(&kept),
            "12345
x345y
"
        );
        assert_eq!((matched, total), (2, 3));
        assert!(
            kept.iter().all(|s| s.style == red),
            "筛选把样式弄丢了：{:?}",
            kept.iter().map(|s| s.style).collect::<Vec<_>>()
        );

        let (text_kept, text_matched, text_total) = filter_lines(
            "12345
abc
x345y
",
            "345",
        );
        assert_eq!(span_text(&kept), text_kept.as_ref());
        assert_eq!((matched, total), (text_matched, text_total));
    }

    /// 一个片段跨多行时按**逻辑行**判定，不是把整段当成一行。
    #[test]
    fn span_filter_splits_a_multi_line_span_into_logical_lines() {
        let spans = vec![span(
            "aa
bb 7
cc
",
        )];
        let (kept, matched, total) = filter_span_lines(&spans, "7");
        assert_eq!(
            span_text(&kept),
            "bb 7
"
        );
        assert_eq!((matched, total), (1, 3));
    }

    /// 一行都没命中 ⇒ 结果为空，但**总行数照实报**（界面据此显示"命中 0 / N 行"，
    /// 用户才知道是筛没了、不是没收到数据）。两份实现都要守这一条。
    #[test]
    fn filter_reports_zero_matches_without_hiding_the_total() {
        let (kept, matched, total) = filter_lines(
            "aa
bb
", "zzz",
        );
        assert!(kept.is_empty());
        assert_eq!((matched, total), (0, 2));

        let spans = vec![span(
            "aa
bb
",
        )];
        let (kept, matched, total) = filter_span_lines(&spans, "zzz");
        assert!(kept.is_empty());
        assert_eq!((matched, total), (0, 2));
    }
    /// 驱动接收区一次，返回**画出来的全部文字**（拼接后）。
    ///
    /// 判据必须是"画出来的东西"：筛选是显示层行为，读 `receive_text` 只证明"内部状态
    /// 改了"，证明不了屏幕上真的少了那几行。
    fn painted_receive(app: &mut ComTool) -> String {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(700.0, 500.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.render_receive_area(ui));
        let mut painted = String::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(t) = &clipped.shape {
                painted.push_str(t.galley.text());
            }
        }
        out.textures_delta.clear();
        painted
    }

    /// 把三份镜像摆成同一份内容——真实运行时它们逐字相同（见 `receive_data` 的落点），
    /// 这里手工摆好，才能在一个测试里覆盖"文本镜像"与"片段镜像"两条渲染路径。
    fn set_all_mirrors(app: &mut ComTool, text: &str) {
        app.receive_text = text.to_string();
        app.display_spans = vec![span(text)];
        app.ascii_spans = vec![span(text)];
    }

    /// **筛选真的作用在画出来的内容上**（走 `render_receive_area` 的真实分发）。
    ///
    /// 空转验证：把渲染路径里的 `filter_lines(..)` 换回直接传 `&self.receive_text`
    /// （或让 `filter_lines` 恒返回原文）⇒ 第一条断言立刻变红。
    #[test]
    fn filtering_hides_the_lines_that_do_not_match() {
        let mut app = ComTool::default();
        set_all_mirrors(
            &mut app,
            "aaa 111
bbb 222
ccc 333
",
        );
        app.filter_text = "222".to_string();

        assert_eq!(
            painted_receive(&mut app),
            "bbb 222
",
            "筛选后接收区应只剩命中那一行"
        );
        assert_eq!(
            (app.filter_matched_lines, app.filter_total_lines),
            (1, 3),
            "命中数 / 总行数没如实记下——顶部那个指示器靠它"
        );
    }

    /// **彩色**与**字节查看**两个视图同样要筛（它们渲染的是片段，走的是另一份实现）。
    #[test]
    fn filtering_applies_to_the_colour_and_byte_views_too() {
        let text = "aaa 111
bbb 222
ccc 333
";
        for (label, mut app) in [
            (
                "彩色视图",
                ComTool {
                    color_view: true,
                    ..ComTool::default()
                },
            ),
            (
                "字节查看视图",
                ComTool {
                    ascii_view: true,
                    ..ComTool::default()
                },
            ),
        ] {
            set_all_mirrors(&mut app, text);
            app.filter_text = "222".to_string();
            // 前提：这一轮确实走的是片段那条路（否则测的还是文本路径）
            assert_ne!(
                app.display_mode(),
                DisplayMode::Text,
                "前提：{label}不该是文本模式"
            );

            assert_eq!(
                painted_receive(&mut app),
                "bbb 222
",
                "{label}没有筛掉不该显示的行"
            );
            assert_eq!(
                (app.filter_matched_lines, app.filter_total_lines),
                (1, 3),
                "{label}的行计数不对"
            );
        }
    }

    /// 关掉筛选（关键字为空）⇒ **原样显示**，且计数归零。
    #[test]
    fn clearing_the_filter_shows_every_line_again() {
        let mut app = ComTool::default();
        set_all_mirrors(
            &mut app,
            "aaa 111
bbb 222
",
        );
        app.filter_text = "222".to_string();
        assert_eq!(
            painted_receive(&mut app),
            "bbb 222
"
        );

        app.filter_text.clear();
        assert_eq!(
            painted_receive(&mut app),
            "aaa 111
bbb 222
",
            "清空关键字后应当恢复显示全部行"
        );
        assert_eq!((app.filter_matched_lines, app.filter_total_lines), (0, 0));
    }

    /// 被筛掉的行**仍然留在镜像里**——筛选只改显示，不改数据。
    ///
    /// 这条挡的是"为了筛得干净顺手把源数据也删了"：导出、实时日志文件、图表解析都读
    /// 原镜像，删了就是静默丢数据。
    #[test]
    fn filtering_never_touches_the_stored_log() {
        let text = "aaa 111
bbb 222
ccc 333
";
        let mut app = ComTool::default();
        set_all_mirrors(&mut app, text);
        app.filter_text = "222".to_string();
        let _ = painted_receive(&mut app);

        assert_eq!(app.receive_text, text, "筛选动了源文本");
        assert_eq!(span_text(&app.display_spans), text, "筛选动了显示镜像");
        assert_eq!(span_text(&app.ascii_spans), text, "筛选动了字节镜像");
    }
}
