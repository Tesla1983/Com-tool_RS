//! 发送侧的纯逻辑：负载构建（文本 / HEX）、行尾、发送历史。
//!
//! 这个模块不接触 egui，也不持有通道句柄——它只把「用户输入 + 选项」翻译成
//! 「要写进通道的字节」，外加维护一份发送历史与历史浏览光标。分成独立模块
//! 有一个具体目的：**这三件事都能直接单测**，不需要窗口、不需要串口硬件。
//!
//! 三条设计约定（都来自实际会踩的坑）：
//!
//! 1. **非法 HEX 一律报错，绝不静默丢弃。** 静默丢掉半个字节后用户看到的是
//!    "设备没反应"，排查成本远高于一条明确的错误提示。报错时调用方**不得**
//!    清空输入框——用户还要改它。
//! 2. **行尾在字节层追加，不在字符串层。** HEX 模式下输入是十六进制文本，
//!    对它追加 `"\r\n"` 再解析会直接报错或发出错误的字节。
//! 3. **空负载不等于"要发一个空包"。** 输入为空且行尾为「无」时没有任何字节要
//!    发出去；而输入为空但行尾为 LF 时，发一个 `0x0A` 是完全合理的用法。

use crate::receive::TextEncoding;
use serde::{Deserialize, Serialize};

/// 发送历史的容量上限（条）。超出后淘汰最旧的一条。
pub const MAX_SEND_HISTORY: usize = 50;

/// 常用数据的容量上限（条）。超出后拒绝新增（**不淘汰旧的**）。
///
/// 与 [`MAX_SEND_HISTORY`] 的淘汰策略不同，理由是这两份列表的性质相反：历史是
/// 自动攒出来的，淘汰最旧的天经地义；常用数据是用户**亲手挑出来**的，静默删掉
/// 第一条会让人以为程序丢了数据。满了就明确拒绝，由用户自己先删一条。
pub const MAX_SEND_PRESETS: usize = 16;

/// 常用数据按钮上短标签的字符上限（N2）。
///
/// 放在这里而不是界面里：这是**标签策略**（多长算太长）而不是布局参数，改了它
/// 会影响所有渲染这一列表的地方。12 个字符按等宽字体算约 90px，加上内边距正好
/// 容纳三四个按钮一行。
pub const PRESET_LABEL_CHARS: usize = 12;

/// 定时发送间隔的下限（毫秒）。
///
/// 10ms = 100 次/秒。再快就已经超出手动调试的意义，只会把 UI 线程和串口驱动
/// 一起拖住（每次发送都要走一遍 `build_payload`）。
pub const MIN_REPEAT_INTERVAL_MS: u64 = 10;

/// 定时发送间隔的上限（毫秒）：10 分钟。
pub const MAX_REPEAT_INTERVAL_MS: u64 = 600_000;

/// 定时发送间隔的默认值（毫秒）。
pub const DEFAULT_REPEAT_INTERVAL_MS: u64 = 1_000;

/// 发送负载的解析方式。
///
/// 与接收侧的「HEX 接收 / 字节查看」**无关**：那两个是"怎么显示收到的字节"，
/// 这个是"怎么解读用户敲进来的文本"。不要把它们混成一套互斥规则。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SendFormat {
    /// 按文本发送（`F2` 落地后，这里按用户所选的编码编码）。
    #[default]
    Text,
    /// 按十六进制发送：输入是十六进制文本，解析成字节后发出。
    Hex,
    /// 按转义序列发送（N3）：`\r` `\n` `\t` `\0` `\xNN` `\\` 变成字节，
    /// 其余字面文本仍按用户所选的编码编码。
    ///
    /// 单独立一档而不是给文本模式加一个 `escape: bool`：这正是"输入的解析方式"
    /// 这根轴上的第三个取值，与 [`crate::app::ComTool::send_format`] 的定义完全一致。
    /// 加 bool 会让"文本 + 转义 + HEX"这些组合里出现无意义的格子。
    Escape,
}

/// 发送时追加的行尾。
///
/// 用三值枚举而不是"两个 bool"（`append_cr` + `append_lf` 之类）：那四个组合里
/// 只有三个有意义，用 bool 表达会让 `CR only` 这种无意义状态进入存档。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SendLineEnding {
    /// 不追加。
    #[default]
    None,
    /// 追加 `LF`（`0x0A`）。
    Lf,
    /// 追加 `CRLF`（`0x0D 0x0A`）。
    Crlf,
}

impl SendLineEnding {
    /// 要追加的字节。空切片表示不追加。
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => &[],
            Self::Lf => b"\n",
            Self::Crlf => b"\r\n",
        }
    }
}

/// 把十六进制文本解析成字节。
///
/// 接受的写法（三种在同一个输入里可以混用）：
///
/// - 连续数字：`48656c6c6f`
/// - 空格 / 逗号 / 分号 / 冒号 / 连字符分隔：`48 65 6C 6C 6F`、`48,65,6c,6c,6f`
/// - 带 `0x` / `0X` 前缀：`0x48 0x65`、`0x4865`
///
/// 分隔符只作分组用途：**每一组必须自成整数个字节**，因此 `48 656c` 是三个字节
/// （`48`、`65`、`6c`），而不是把它当成 `48656c` 的另一种写法。
///
/// 空输入（或只有分隔符）返回空 `Vec`，不是错误——"什么都没输入"是合法状态。
pub fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();

    for raw in input.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-')) {
        if raw.is_empty() {
            continue;
        }

        let digits = raw
            .strip_prefix("0x")
            .or_else(|| raw.strip_prefix("0X"))
            .unwrap_or(raw);

        if digits.is_empty() {
            return Err(format!("`{raw}` 里没有十六进制数字"));
        }
        if let Some(bad) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(format!("`{raw}` 里的 `{bad}` 不是十六进制字符"));
        }
        if digits.len() % 2 != 0 {
            return Err(format!(
                "`{raw}` 有 {} 位，是奇数——凑不满整字节。补一位，或用空格 / 逗号分隔",
                digits.len()
            ));
        }

        // 到这里 `digits` 全是 ASCII 十六进制字符，按字节切片是安全的。
        for i in (0..digits.len()).step_by(2) {
            let byte = u8::from_str_radix(&digits[i..i + 2], 16)
                .expect("已校验为两位十六进制，必然可解析");
            out.push(byte);
        }
    }

    Ok(out)
}

/// 转义序列解析出来的一个片段。
///
/// 拆成"字面文本"与"原始字节"两种，而不是直接产出 `Vec<u8>`：字面文本还得按当前
/// 编码编码（GBK 下"你好"是两个字节），而转义产出的是**原样字节**。混在一起的话
/// `\xC4\xE3` 会被当成文本再编码一遍。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EscItem {
    /// 一段普通文本，交给调用方按当前编码编码。
    Literal(String),
    /// 转义直接产出的字节。
    Bytes(Vec<u8>),
}

/// 单字符转义表。
///
/// **不解释 `\u{...}`**：那会把"文本编码"偷偷混进来——用户选了 GBK 就说明字面文本
/// 的字节由 GBK 决定，此时再给一条绕过编码的入口，同一段输入就有了两种互不相容的
/// 解释。要指定原始字节，用 `\xNN`。
const ESCAPES: &[(char, u8)] = &[
    ('r', 0x0D),
    ('n', 0x0A),
    ('t', 0x09),
    ('0', 0x00),
    ('\\', b'\\'),
];

/// 把带转义的文本解析成片段序列。
///
/// 与 [`parse_hex`] 同一条规则：**写错了就报错，绝不静默丢弃**（模块文档第 1 条）。
/// 未知转义（`\q`）、`\x` 后面位数不足或含非十六进制字符、结尾孤立的 `\`，一律
/// 返回 `Err`，调用方据此"不发、也不清空输入框"。
pub(crate) fn parse_escapes(input: &str) -> Result<Vec<EscItem>, String> {
    let mut items = Vec::new();
    let mut literal = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\\' {
            literal.push(c);
            continue;
        }

        // 见到 `\` 先把攒着的字面文本冲出去：顺序必须保住，`a\x41b` 里那两段文本
        // 不能因为"先收集再统一编码"而被合并成一个整体（合并后仍等价，但一旦以后再
        // 加"字面文本可被某个开关影响"的行为，这里就会变成 bug）。
        if !literal.is_empty() {
            items.push(EscItem::Literal(std::mem::take(&mut literal)));
        }

        let Some(next) = chars.next() else {
            return Err("结尾有一个孤立的 `\\`：它后面还差一个转义字符".to_string());
        };

        if let Some((_, byte)) = ESCAPES.iter().find(|(name, _)| *name == next) {
            items.push(EscItem::Bytes(vec![*byte]));
            continue;
        }

        if next == 'x' {
            let digits: String = chars.by_ref().take(2).collect();
            if digits.chars().count() < 2 {
                return Err(format!("`\\x` 后面需要两位十六进制数，只给了 `{digits}`"));
            }
            if let Some(bad) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
                return Err(format!("`\\x{digits}` 里的 `{bad}` 不是十六进制字符"));
            }
            let byte = u8::from_str_radix(&digits, 16).expect("已校验为两位十六进制，必然可解析");
            items.push(EscItem::Bytes(vec![byte]));
            continue;
        }

        return Err(format!(
            "认不出转义 `\\{next}`；可用的是 \\\\ \\r \\n \\t \\0 \\xNN"
        ));
    }

    if !literal.is_empty() {
        items.push(EscItem::Literal(literal));
    }
    Ok(items)
}

/// 一次发送要写出去的字节，以及一个必须让用户知道的警告。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    pub bytes: Vec<u8>,
    /// 有字符在当前编码里无法表示，已被编码器替换。
    ///
    /// `encoding_rs` 在无法映射时会写入 **HTML 数字字符引用**（例如把某个字符写成
    /// `&#1234;`），这在串口场景下几乎肯定不是用户想要的。所以**照样发出去，但必须
    /// 明说**——静默换掉用户要发的字符比直接报错更难排查。
    pub had_unmappable: bool,
}

/// 文本负载的编码。
///
/// 三个分支各有原因：
///
/// - **UTF-8**：直接取字节，不走 `encoding_rs`（省一次调用，行为也最直白）。
/// - **UTF-16LE**：**必须自己写**。`encoding_rs` 的 UTF-16LE 是**只解码**的编码——
///   它的 `output_encoding()` 返回 UTF-8，`encode()` 会原样把 UTF-8 字节吐回来。
///   这里按小端写出每个 UTF-16 码元。**不加 BOM**：设备要不要 BOM 是设备的事，
///   替用户多塞两个字节只会更容易让设备认不出帧头。
/// - **GBK / GB18030**：交给 `encoding_rs`。注意两者的**编码器不同**（GB18030 支持
///   四字节序列），虽然**解码器相同**——所以"接收看着一样、发出去不一样"是正常的。
pub fn encode_text(text: &str, encoding: TextEncoding) -> (Vec<u8>, bool) {
    match encoding {
        TextEncoding::Utf8 => (text.as_bytes().to_vec(), false),
        TextEncoding::Utf16Le => {
            let mut out = Vec::with_capacity(text.len() * 2);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            // UTF-16 能表示全部 Unicode 标量值，不存在"表示不了"的字符
            (out, false)
        }
        _ => {
            let (bytes, _detected, had_unmappable) = encoding.encoding().encode(text);
            (bytes.into_owned(), had_unmappable)
        }
    }
}

/// 组装本次要写进通道的字节：先解析负载，再在**字节层**追加行尾。
///
/// HEX 模式**不经过编码**：输入本身就是字节的十六进制写法，再按编码解释一遍等于
/// 多此一举，还会把 `41 42` 变成别的意思。
///
/// 转义模式（N3）介于两者之间：**转义产出的字节原样、字面文本照常编码**。写成
/// "整个模式都不编码"是错的——用户在 GBK 档下敲的中文仍该是 GBK 字节。
pub fn build_payload(
    text: &str,
    format: SendFormat,
    ending: SendLineEnding,
    encoding: TextEncoding,
) -> Result<Payload, String> {
    let (mut bytes, had_unmappable) = match format {
        SendFormat::Text => encode_text(text, encoding),
        SendFormat::Hex => (parse_hex(text)?, false),
        SendFormat::Escape => {
            let mut out = Vec::new();
            let mut unmappable = false;
            for item in parse_escapes(text)? {
                match item {
                    EscItem::Literal(literal) => {
                        let (encoded, had) = encode_text(&literal, encoding);
                        out.extend_from_slice(&encoded);
                        unmappable |= had;
                    }
                    EscItem::Bytes(raw) => out.extend_from_slice(&raw),
                }
            }
            (out, unmappable)
        }
    };
    bytes.extend_from_slice(ending.bytes());
    Ok(Payload {
        bytes,
        had_unmappable,
    })
}

/// 把定时发送间隔夹回合法区间。
pub fn clamp_repeat_interval(ms: u64) -> u64 {
    ms.clamp(MIN_REPEAT_INTERVAL_MS, MAX_REPEAT_INTERVAL_MS)
}

/// 光标是否在首行——决定 ↑ 能不能用来取历史。
///
/// `cursor` 是**字符**下标（与 egui 的 `CCursor::index` 一致，不是字节下标）。
pub fn on_first_line(text: &str, cursor: usize) -> bool {
    !text.chars().take(cursor).any(|c| c == '\n')
}

/// 光标是否在末行——决定 ↓ 能不能用来取历史。
pub fn on_last_line(text: &str, cursor: usize) -> bool {
    !text.chars().skip(cursor).any(|c| c == '\n')
}

/// 发送历史。
///
/// 只存"发出去过什么"，不存连接状态或时间戳——那些要么活不过进程边界，要么
/// 没有用途。上限固定为 [`MAX_SEND_HISTORY`]，因此**不把容量写进结构体**，
/// 免得存档里出现一个可以被手改成 0 或 10 万的字段。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendHistory {
    entries: Vec<String>,
}

impl SendHistory {
    /// 全部条目，从最旧到最新。
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 记一条。
    ///
    /// 两条规则：空串不记（用户按了发送但没输入，不该占一格历史）；**与最近一条
    /// 相同则不重复记**——定时发送每秒发同一句话，历史会被同一行塞满。
    pub fn record(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.entries.last().map(String::as_str) == Some(text) {
            return;
        }
        self.entries.push(text.to_string());
        if self.entries.len() > MAX_SEND_HISTORY {
            let excess = self.entries.len() - MAX_SEND_HISTORY;
            self.entries.drain(0..excess);
        }
    }

    /// 从最新往旧数第 `back` 条（`0` = 最近一条）。越界返回 `None`。
    pub fn from_back(&self, back: usize) -> Option<&str> {
        let index = self.entries.len().checked_sub(back + 1)?;
        self.entries.get(index).map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// 由内容推出按钮上的短标签（N2）。
///
/// 取**第一行**、去掉首尾空白，超过 `max_chars` 个字符就截断并加省略号。按**字符**
/// 而不是字节截断：中文一个字三字节，按字节切会切出半个字（这是本项目的老教训）。
///
/// 第一行为空（内容以换行开头、或整条只有空白）时给一个占位标签，否则按钮会是
/// 一块空白、连点都找不到地方。
pub fn preset_label(text: &str, max_chars: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "…".to_string();
    }
    let mut chars = first_line.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// 常用数据：用户挑出来、想一键重发的几条（N2）。
///
/// **只存内容，不存标签**——标签由 [`preset_label`] 从内容现推。存标签要多一个字段
/// 和一套改名界面，换来的只是"两条开头一样的指令能区分开"，而把完整内容挂在按钮的
/// 悬停提示上就够了。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPresets {
    entries: Vec<String>,
}

impl SendPresets {
    /// 全部条目，按加入顺序。
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.entries.len() >= MAX_SEND_PRESETS
    }

    pub fn contains(&self, text: &str) -> bool {
        self.entries.iter().any(|entry| entry == text)
    }

    /// 这段内容能不能存进去。
    ///
    /// 界面用它决定「存为常用」按钮是否可点。**判定只有这一处**——否则按钮的置灰
    /// 条件与 [`add`](Self::add) 的实际规则迟早对不上（按钮亮着、点了没反应）。
    ///
    /// 拒绝三种：空内容、与已有条目一字不差、列表已满。
    pub fn can_add(&self, text: &str) -> bool {
        !text.is_empty() && !self.is_full() && !self.contains(text)
    }

    /// 存一条，返回是否真的存进去了。
    ///
    /// 满了**不淘汰旧的**（与 [`SendHistory`] 相反）：这些是用户亲手挑的，静默删掉
    /// 第一条会让人以为程序丢了数据。满了就明确拒绝，由用户自己先删一条。
    pub fn add(&mut self, text: &str) -> bool {
        if !self.can_add(text) {
            return false;
        }
        self.entries.push(text.to_string());
        true
    }

    /// 删掉第 `index` 条；越界什么也不做。
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// 历史的浏览状态（↑ / ↓ 停在哪儿、回到编辑态时要还原什么）。
///
/// 与 [`SendHistory`] 分开：历史本身是**用户偏好、要落盘**，浏览位置是纯运行时
/// 状态、**不落盘**（下次启动不该停在半截历史里）。
#[derive(Debug, Default, Clone)]
pub struct HistoryNav {
    /// 当前停留的条目（`0` = 最近一条）；`None` = 没在浏览历史。
    cursor: Option<usize>,
    /// 开始浏览之前在输入框里的内容。↓ 越过最近一条时用它还原。
    draft: Option<String>,
}

impl HistoryNav {
    pub fn is_active(&self) -> bool {
        self.cursor.is_some()
    }

    /// 退出浏览态（用户自己改了输入框、或刚发出去一条）。
    pub fn reset(&mut self) {
        self.cursor = None;
        self.draft = None;
    }

    /// ↑：往更旧走一步。返回要放进输入框的文本；已经是最旧的一条时返回 `None`
    /// （停在原地，而不是绕回最新的）。
    pub fn older(&mut self, history: &SendHistory, current: &str) -> Option<String> {
        let next = match self.cursor {
            None => 0,
            Some(i) => i + 1,
        };
        let text = history.from_back(next)?.to_string();
        if self.cursor.is_none() {
            // 只在"从编辑态第一次进入历史"时记草稿，之后连续按 ↑ 不该覆盖它。
            self.draft = Some(current.to_string());
        }
        self.cursor = Some(next);
        Some(text)
    }

    /// ↓：往更新走一步；越过最近一条时回到开始浏览前的草稿。
    ///
    /// 没在浏览历史时返回 `None`——**不主动"记住"当前内容**，否则用户在首行按 ↓
    /// 会发现输入框莫名其妙被换掉了。
    pub fn newer(&mut self, history: &SendHistory) -> Option<String> {
        let cursor = self.cursor?;
        if cursor == 0 {
            self.cursor = None;
            return self.draft.take();
        }
        let next = cursor - 1;
        let text = history.from_back(next)?.to_string();
        self.cursor = Some(next);
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- HEX 解析 ----------

    #[test]
    fn hex_accepts_common_separators() {
        let expected = b"Hello".to_vec();
        for input in [
            "48656c6c6f",
            "48 65 6C 6C 6F",
            "48,65,6c,6c,6f",
            "0x48 0x65 0x6c 0x6c 0x6f",
            "0X48,0X65,0X6C,0X6C,0X6F",
            "48:65:6c:6c:6f",
            "48-65-6c-6c-6f",
            "0x48656c6c6f",
            "  48 65\n6c\t6c 6f  ",
        ] {
            assert_eq!(parse_hex(input).unwrap(), expected, "输入 `{input}`");
        }
    }

    #[test]
    fn hex_grouping_is_per_token_not_concatenated() {
        // `48 656c` 是三个字节，而不是把空格当噪声后拼成 48656c（四个字符）。
        assert_eq!(parse_hex("48 656c").unwrap(), vec![0x48, 0x65, 0x6C]);
    }

    #[test]
    fn hex_empty_input_is_empty_payload_not_an_error() {
        assert_eq!(parse_hex("").unwrap(), Vec::<u8>::new());
        assert_eq!(parse_hex("   ").unwrap(), Vec::<u8>::new());
        assert_eq!(parse_hex(", ,").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hex_rejects_odd_digit_count() {
        let err = parse_hex("4").unwrap_err();
        assert!(err.contains("奇数"), "错误信息要说清原因: {err}");
        assert!(parse_hex("48656c6c6").unwrap_err().contains("奇数"));
        // 奇数位出现在第二组时，报的是那一组
        assert!(parse_hex("48 656").unwrap_err().contains("656"));
    }

    #[test]
    fn hex_rejects_non_hex_characters() {
        let err = parse_hex("0xZZ").unwrap_err();
        assert!(err.contains('Z'), "错误信息要点出那个字符: {err}");
        assert!(parse_hex("48 6g").unwrap_err().contains('g'));
        assert!(parse_hex("hello").unwrap_err().contains('h'));
    }

    #[test]
    fn hex_rejects_bare_prefix() {
        assert!(parse_hex("0x").unwrap_err().contains("没有十六进制数字"));
    }

    // ---------- 负载组装 ----------

    /// 默认编码下组装负载。多数用例不关心编码，用它少写一个参数。
    fn payload(text: &str, format: SendFormat, ending: SendLineEnding) -> Result<Payload, String> {
        build_payload(text, format, ending, TextEncoding::Utf8)
    }

    #[test]
    fn line_endings_append_at_byte_level() {
        assert_eq!(
            payload("AB", SendFormat::Text, SendLineEnding::None)
                .unwrap()
                .bytes,
            b"AB"
        );
        assert_eq!(
            payload("AB", SendFormat::Text, SendLineEnding::Lf)
                .unwrap()
                .bytes,
            b"AB\n"
        );
        assert_eq!(
            payload("AB", SendFormat::Text, SendLineEnding::Crlf)
                .unwrap()
                .bytes,
            b"AB\r\n"
        );
    }

    /// HEX 模式下行尾必须是**字节**：`0x0D 0x0A`，而不是把 `"\r\n"` 当文本发出去。
    #[test]
    fn line_ending_in_hex_mode_is_bytes_not_characters() {
        let payload = payload("41", SendFormat::Hex, SendLineEnding::Crlf).unwrap();
        assert_eq!(payload.bytes, vec![0x41, 0x0D, 0x0A]);
    }

    /// 只发一个换行是合法用法（输入为空 + 行尾 LF）。
    #[test]
    fn empty_input_with_line_ending_still_sends() {
        assert_eq!(
            payload("", SendFormat::Text, SendLineEnding::Lf)
                .unwrap()
                .bytes,
            b"\n"
        );
        // 而输入为空、行尾为「无」时确实没有任何字节要发
        assert!(
            payload("", SendFormat::Text, SendLineEnding::None)
                .unwrap()
                .bytes
                .is_empty()
        );
    }

    /// 解析失败时**不能**产出任何字节——调用方据此决定"不发、也不清空输入框"。
    #[test]
    fn failed_parse_yields_no_payload() {
        assert!(payload("4", SendFormat::Hex, SendLineEnding::None).is_err());
        assert!(payload("4", SendFormat::Hex, SendLineEnding::Crlf).is_err());
    }

    #[test]
    fn text_payload_is_utf8_by_default() {
        assert_eq!(
            encode_text("你好", TextEncoding::Utf8).0,
            vec![0xE4, 0xBD, 0xA0, 0xE5, 0xA5, 0xBD]
        );
    }

    // ---------- 文本编码（F2） ----------

    #[test]
    fn gbk_encoding_produces_gbk_bytes() {
        let (bytes, unmappable) = encode_text("你好", TextEncoding::Gbk);
        assert_eq!(bytes, vec![0xC4, 0xE3, 0xBA, 0xC3]);
        assert!(!unmappable);
    }

    /// GBK 与 GB18030 的**解码器**相同，但**编码器**不同：GB18030 能表示 GBK
    /// 表示不了的字符（四字节序列）。所以同一段文本在两档下的字节可以不同。
    #[test]
    fn utf8_is_unaffected_by_choosing_gbk() {
        // ASCII 在 GBK 下与 UTF-8 完全一致——这是"切换编码不该让纯英文命令变样"的保证
        assert_eq!(encode_text("AT+RST", TextEncoding::Gbk).0, b"AT+RST");
        assert_eq!(encode_text("AT+RST", TextEncoding::Gb18030).0, b"AT+RST");
        assert_eq!(encode_text("", TextEncoding::Utf16Le).0, Vec::<u8>::new());
    }

    #[test]
    fn utf16le_encoding_adds_null_bytes() {
        assert_eq!(
            encode_text("AB", TextEncoding::Utf16Le).0,
            vec![0x41, 0x00, 0x42, 0x00]
        );
    }

    /// 表示不了的字符必须**如实报告**，不能静默替换掉。
    ///
    /// GBK 没有 U+1D11E（音乐符号）这样的码位，`encoding_rs` 会写入 HTML 数字字符
    /// 引用；这在串口场景下几乎肯定不是用户要的，所以要 `had_unmappable` 让上层提示。
    #[test]
    fn unmappable_characters_are_reported() {
        let (_bytes, had_unmappable) = encode_text("\u{1D11E}", TextEncoding::Gbk);
        assert!(had_unmappable, "生僻字符在 GBK 下表示不了，必须报告");
    }

    /// HEX 模式不经过编码：输入本身就是字节的十六进制写法。
    #[test]
    fn hex_mode_ignores_the_text_encoding() {
        let bytes = build_payload(
            "C4 E3",
            SendFormat::Hex,
            SendLineEnding::None,
            TextEncoding::Gbk,
        )
        .unwrap()
        .bytes;
        assert_eq!(
            bytes,
            vec![0xC4, 0xE3],
            "HEX 输入就是字节，不该再被编码一次"
        );
    }

    // ---------- 定时发送间隔 ----------

    #[test]
    fn repeat_interval_is_clamped() {
        assert_eq!(clamp_repeat_interval(0), MIN_REPEAT_INTERVAL_MS);
        assert_eq!(clamp_repeat_interval(1), MIN_REPEAT_INTERVAL_MS);
        assert_eq!(clamp_repeat_interval(500), 500);
        assert_eq!(clamp_repeat_interval(u64::MAX), MAX_REPEAT_INTERVAL_MS);
    }

    // ---------- 首行 / 末行判定 ----------

    #[test]
    fn first_and_last_line_detection() {
        let text = "abc\ndef\nghi";
        // 首行内（下标 0~3）
        assert!(on_first_line(text, 0));
        assert!(on_first_line(text, 3));
        // 跨过第一个换行后就不是首行了
        assert!(!on_first_line(text, 4));
        // 末行内（下标 8~11）
        assert!(on_last_line(text, 8));
        assert!(on_last_line(text, 11));
        // 末行之前还有内容
        assert!(!on_last_line(text, 3));
    }

    #[test]
    fn single_line_text_is_both_first_and_last() {
        assert!(on_first_line("hello", 5));
        assert!(on_last_line("hello", 5));
    }

    /// 中文是多字节，但判定按**字符**下标——若误用字节下标，这里会算错。
    #[test]
    fn line_detection_uses_character_indices() {
        let text = "你好\n世界";
        assert!(on_first_line(text, 2), "`你好` 是 2 个字符");
        assert!(!on_first_line(text, 3));
        assert!(on_last_line(text, 5));
    }

    // ---------- 发送历史 ----------

    #[test]
    fn history_records_and_orders_from_oldest_to_newest() {
        let mut h = SendHistory::default();
        h.record("a");
        h.record("b");
        assert_eq!(h.entries(), ["a", "b"]);
        assert_eq!(h.from_back(0), Some("b"));
        assert_eq!(h.from_back(1), Some("a"));
        assert_eq!(h.from_back(2), None);
    }

    #[test]
    fn history_skips_empty_and_consecutive_duplicates() {
        let mut h = SendHistory::default();
        h.record("");
        assert!(h.is_empty(), "空串不占历史");
        h.record("a");
        h.record("a");
        assert_eq!(h.len(), 1, "连发同一条只记一次");
        h.record("b");
        h.record("a");
        assert_eq!(h.len(), 3, "隔开了就要各记一条");
    }

    #[test]
    fn history_evicts_oldest_beyond_capacity() {
        let mut h = SendHistory::default();
        for i in 0..(MAX_SEND_HISTORY + 10) {
            h.record(&format!("m{i}"));
        }
        assert_eq!(h.len(), MAX_SEND_HISTORY);
        assert_eq!(h.from_back(0), Some("m59"));
        assert_eq!(h.entries().first().map(String::as_str), Some("m10"));
    }

    #[test]
    fn history_clear_empties_everything() {
        let mut h = SendHistory::default();
        h.record("a");
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.from_back(0), None);
    }

    // 历史的落盘往返不在这里测：真实路径是 `eframe::set_value` → RON → `get_value`
    // → `config::load`，由 `config::tests::preferences_survive_round_trip` 覆盖。
    // 在本模块里手写一遍序列化只会测到"我们以为的"格式。

    // ---------- 历史浏览 ----------

    fn history_of(items: &[&str]) -> SendHistory {
        let mut h = SendHistory::default();
        for item in items {
            h.record(item);
        }
        h
    }

    #[test]
    fn nav_walks_backwards_and_stops_at_oldest() {
        let h = history_of(&["a", "b", "c"]);
        let mut nav = HistoryNav::default();

        assert_eq!(nav.older(&h, "draft"), Some("c".to_string()));
        assert_eq!(nav.older(&h, "draft"), Some("b".to_string()));
        assert_eq!(nav.older(&h, "draft"), Some("a".to_string()));
        // 已经在最旧：停在原地，不绕回最新的
        assert_eq!(nav.older(&h, "draft"), None);
        assert_eq!(nav.older(&h, "draft"), None);
    }

    #[test]
    fn nav_restores_original_draft_when_going_past_newest() {
        let h = history_of(&["a", "b"]);
        let mut nav = HistoryNav::default();

        assert_eq!(nav.older(&h, "我正在写的内容"), Some("b".to_string()));
        // 连续 ↑ 时草稿不能被覆盖成历史条目
        assert_eq!(nav.older(&h, "b"), Some("a".to_string()));
        assert_eq!(nav.newer(&h), Some("b".to_string()));
        assert_eq!(nav.newer(&h), Some("我正在写的内容".to_string()));
        assert!(!nav.is_active(), "回到草稿后已退出浏览态");
    }

    /// 没在浏览历史时按 ↓ 必须什么都不做——否则用户想往下移光标却把内容换掉了。
    #[test]
    fn nav_does_nothing_on_down_when_not_browsing() {
        let h = history_of(&["a"]);
        let mut nav = HistoryNav::default();
        assert_eq!(nav.newer(&h), None);
        assert!(!nav.is_active());
    }

    #[test]
    fn nav_with_empty_history_stays_inactive() {
        let h = SendHistory::default();
        let mut nav = HistoryNav::default();
        assert_eq!(nav.older(&h, ""), None);
        assert!(!nav.is_active());
    }

    #[test]
    fn nav_reset_forgets_position_and_draft() {
        let h = history_of(&["a", "b"]);
        let mut nav = HistoryNav::default();
        nav.older(&h, "draft");
        nav.reset();
        assert!(!nav.is_active());

        // reset 后再按 ↑ 从最近一条重新开始，且草稿是**重新采集**的，不是上一轮剩下的
        assert_eq!(nav.older(&h, "新的草稿"), Some("b".to_string()));
        assert_eq!(nav.older(&h, "新的草稿"), Some("a".to_string()));
        assert_eq!(nav.newer(&h), Some("b".to_string()));
        assert_eq!(nav.newer(&h), Some("新的草稿".to_string()));
        assert_eq!(nav.newer(&h), None, "已回到编辑态，再按 ↓ 不该动");
    }

    // ---------- N3：转义序列 ----------

    /// 转义与字面文本**必须切成不同的片段**：`\n` 是原样字节，字面文本走编码。
    /// 合成一段"先拼字符串再统一编码"就丢掉了这个区分（`\xC4` 会被再编码一次）。
    #[test]
    fn escapes_split_into_literals_and_raw_bytes() {
        assert_eq!(
            parse_escapes("a\\r\\nb").unwrap(),
            vec![
                EscItem::Literal("a".to_string()),
                EscItem::Bytes(vec![0x0D]),
                EscItem::Bytes(vec![0x0A]),
                EscItem::Literal("b".to_string()),
            ]
        );
    }

    #[test]
    fn escapes_cover_the_documented_set() {
        assert_eq!(
            parse_escapes("\\r").unwrap(),
            vec![EscItem::Bytes(vec![0x0D])]
        );
        assert_eq!(
            parse_escapes("\\n").unwrap(),
            vec![EscItem::Bytes(vec![0x0A])]
        );
        assert_eq!(
            parse_escapes("\\t").unwrap(),
            vec![EscItem::Bytes(vec![0x09])]
        );
        assert_eq!(
            parse_escapes("\\0").unwrap(),
            vec![EscItem::Bytes(vec![0x00])]
        );
        assert_eq!(
            parse_escapes("\\\\").unwrap(),
            vec![EscItem::Bytes(vec![b'\\'])]
        );
        assert_eq!(
            parse_escapes("\\x41\\x0a").unwrap(),
            vec![EscItem::Bytes(vec![0x41]), EscItem::Bytes(vec![0x0A])]
        );
        // 没有转义时整段就是一个 Literal
        assert_eq!(
            parse_escapes("hello").unwrap(),
            vec![EscItem::Literal("hello".to_string())]
        );
        assert!(parse_escapes("").unwrap().is_empty());
    }

    /// 与 `parse_hex` 同一条规则：**写错了就报错，绝不静默跳过**（模块文档第 1 条）。
    #[test]
    fn escapes_reject_unknown_and_incomplete_forms() {
        assert!(parse_escapes("\\q").unwrap_err().contains('q'));
        assert!(parse_escapes("\\x4").unwrap_err().contains("两位"));
        assert!(parse_escapes("\\xZZ").unwrap_err().contains("十六进制"));
        assert!(parse_escapes("ab\\").unwrap_err().contains("孤立"));
    }

    /// 转义模式下**字节绕过编码**：`\xC4\xE3` 就是 `C4 E3` 两个字节，
    /// 不会被 GBK 再编码一遍（那会变成四个字节，而且完全不是用户要的东西）。
    #[test]
    fn escaped_bytes_bypass_the_encoding() {
        let payload = build_payload(
            "\\xC4\\xE3",
            SendFormat::Escape,
            SendLineEnding::None,
            TextEncoding::Gbk,
        )
        .expect("合法转义不该解析失败");

        assert_eq!(payload.bytes, vec![0xC4, 0xE3]);
    }

    /// 同一段输入里，字面文本照旧走所选编码，转义字节原样——两者可以混用。
    #[test]
    fn escape_mode_still_encodes_literal_text() {
        let payload = build_payload(
            "你\\n",
            SendFormat::Escape,
            SendLineEnding::None,
            TextEncoding::Gbk,
        )
        .expect("合法转义不该解析失败");

        let mut expected = encoding_rs::GBK.encode("你").0.into_owned();
        expected.push(0x0A);
        assert_eq!(payload.bytes, expected);
        assert_eq!(
            payload.bytes.len(),
            3,
            "GBK 的「你」是两个字节，加换行共三个"
        );
    }

    /// 行尾不管在哪一档都追加在**字节层**（与 HEX 档同一个道理）。
    #[test]
    fn escape_mode_appends_line_ending_at_byte_level() {
        let payload = payload("hi", SendFormat::Escape, SendLineEnding::Crlf).unwrap();
        assert_eq!(payload.bytes, b"hi\r\n");
    }

    // ---------- N2：常用数据 ----------

    #[test]
    fn preset_label_takes_the_first_line_and_truncates_by_chars() {
        assert_eq!(preset_label("AT+RST\nAT+GMR", 12), "AT+RST");
        assert_eq!(preset_label("  padded  ", 12), "padded");
        assert_eq!(preset_label("abcdefghijklmnop", 4), "abcd…");

        // 按**字符**截断：中文一个字三字节，按字节切会切出半个字
        assert_eq!(preset_label("中文五五六六", 4), "中文五五…");
        assert_eq!(
            preset_label("中文五五", 4),
            "中文五五",
            "刚好放得下就不加省略号"
        );

        // 首行是空白时给一个占位，否则按钮是一块空白、连点的地方都找不到
        assert_eq!(preset_label("\nrest", 12), "…");
        assert_eq!(preset_label("", 12), "…");
    }

    #[test]
    fn presets_add_remove_and_refuse_over_limit() {
        let mut presets = SendPresets::default();
        assert!(presets.is_empty());
        assert!(!presets.can_add(""));

        assert!(presets.add("a"));
        assert!(presets.contains("a"));
        assert!(!presets.add("a"), "重复项要拒绝");
        assert_eq!(presets.len(), 1);

        for i in 0..MAX_SEND_PRESETS {
            presets.add(&format!("p{i}"));
        }
        assert!(presets.is_full());
        assert!(!presets.add("overflow"), "满了不淘汰旧的，只拒绝");
        assert_eq!(presets.len(), MAX_SEND_PRESETS);
        assert_eq!(presets.entries()[0], "a", "第一条必须还在");

        presets.remove(0);
        assert_eq!(presets.entries()[0], "p0");
        presets.remove(999);
        assert_eq!(presets.len(), MAX_SEND_PRESETS - 1, "越界删除什么也不做");

        presets.clear();
        assert!(presets.is_empty());
    }
}
