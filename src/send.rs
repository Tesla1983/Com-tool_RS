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

use serde::{Deserialize, Serialize};

/// 发送历史的容量上限（条）。超出后淘汰最旧的一条。
pub const MAX_SEND_HISTORY: usize = 50;

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

/// 文本负载的编码入口。
///
/// 目前固定 UTF-8。`FUTURE_ROADMAP.md` 的 F2（编码选择）会把这里换成按用户所选
/// 的编码编码——**这是 F2 唯一需要改的发送侧位置**，所以它单独成为一个函数。
pub fn encode_text(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// 组装本次要写进通道的字节：先解析负载，再在**字节层**追加行尾。
pub fn build_payload(
    text: &str,
    format: SendFormat,
    ending: SendLineEnding,
) -> Result<Vec<u8>, String> {
    let mut bytes = match format {
        SendFormat::Text => encode_text(text),
        SendFormat::Hex => parse_hex(text)?,
    };
    bytes.extend_from_slice(ending.bytes());
    Ok(bytes)
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

    #[test]
    fn line_endings_append_at_byte_level() {
        assert_eq!(
            build_payload("AB", SendFormat::Text, SendLineEnding::None).unwrap(),
            b"AB"
        );
        assert_eq!(
            build_payload("AB", SendFormat::Text, SendLineEnding::Lf).unwrap(),
            b"AB\n"
        );
        assert_eq!(
            build_payload("AB", SendFormat::Text, SendLineEnding::Crlf).unwrap(),
            b"AB\r\n"
        );
    }

    /// HEX 模式下行尾必须是**字节**：`0x0D 0x0A`，而不是把 `"\r\n"` 当文本发出去。
    #[test]
    fn line_ending_in_hex_mode_is_bytes_not_characters() {
        let payload = build_payload("41", SendFormat::Hex, SendLineEnding::Crlf).unwrap();
        assert_eq!(payload, vec![0x41, 0x0D, 0x0A]);
    }

    /// 只发一个换行是合法用法（输入为空 + 行尾 LF）。
    #[test]
    fn empty_input_with_line_ending_still_sends() {
        assert_eq!(
            build_payload("", SendFormat::Text, SendLineEnding::Lf).unwrap(),
            b"\n"
        );
        // 而输入为空、行尾为「无」时确实没有任何字节要发
        assert!(
            build_payload("", SendFormat::Text, SendLineEnding::None)
                .unwrap()
                .is_empty()
        );
    }

    /// 解析失败时**不能**产出任何字节——调用方据此决定"不发、也不清空输入框"。
    #[test]
    fn failed_parse_yields_no_payload() {
        assert!(build_payload("4", SendFormat::Hex, SendLineEnding::None).is_err());
        assert!(build_payload("4", SendFormat::Hex, SendLineEnding::Crlf).is_err());
    }

    #[test]
    fn text_payload_is_utf8() {
        assert_eq!(
            encode_text("你好"),
            vec![0xE4, 0xBD, 0xA0, 0xE5, 0xA5, 0xBD]
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
}
