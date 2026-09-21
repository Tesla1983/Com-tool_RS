//! 实时图表的数据层：把串口字节流解析成多通道数值序列。
//!
//! 分工：本模块只做「字节 → 数值样本」，一行文本如何变成多条曲线；把样本画成
//! 折线在 [`crate::panels::chart`]。数据层刻意不依赖 `egui`，因此可以纯粹用
//! 单元测试固定行为。
//!
//! **输入格式**：一行一组样本，字段用逗号分隔，换行（`\n` 或 `\r\n`）作为组结束符。
//!
//! ```text
//! 1.0,2.0,3.0
//! 1.1,2.1,3.1
//! ```
//!
//! 第 k 个字段永远对应第 k 条曲线，同一组的多个样本共享同一个 x（组序号），
//! 所以多条曲线在横轴上是对齐的。
//!
//! 几条刻意的行为约定：
//!
//! - **串口是字节流，数据块可能在一行中间断开**，因此未见到换行符之前的字节全部
//!   留在 `pending` 里等下一块补齐。这与 [`crate::receive::AnsiDecoder`] 跨包拼接
//!   UTF-8 出自同一个理由：分片是常态，不是异常。
//! - **非数值字段被忽略并计数**，不静默丢弃。设备发的不是本模块约定的格式时，
//!   用户需要在界面上看出「格式对不上」，而不是对着一片空白猜。
//! - **通道数由数据决定**并逐行增长到上限。某行的字段数少于已建立的通道数时，
//!   缺少的通道在该组不记样本，曲线自然断开——**不补零**：补零会画出从未发生过的
//!   数据，那种错误在曲线上看不出来。
//! - **x 是组序号，不是时间戳**。串口数据没有可靠的时间基准，而组序号足以让曲线
//!   按顺序展开；淘汰旧样本时 x 不回退，曲线表现为持续右移。
//! - **半行超长时整行丢弃**（见 [`MAX_PENDING_BYTES`]）。一旦某行超过上限，说明
//!   对端发的不是本模块约定的格式；该行**剩下的字节也一并丢掉，直到换行符**，
//!   而不是从中截一段出来当样本。支持的行格式只有「一行一组」这一种；**不支持**
//!   把无换行的连续逗号流按固定通道数循环分组——那种情况下通道语义取决于猜出来的
//!   通道数，猜错就会把数据画错位，不如明确不做。

use std::collections::VecDeque;

use crate::app::ComTool;

/// 半行残留的长度上限（字节）。
///
/// 只会在**没有换行符**时累积：正常数值流每行几十字节，4096 已远超任何合理的
/// 单行长度。超限说明对端发的不是本模块约定的「一行一组」格式（例如无换行的
/// 连续逗号流，或干脆是二进制日志），此时整行丢弃并计数，而不是让缓冲无限增长
/// —— 串口工具挂机跑一整天是常见用法，无上限的缓冲等于把内存交给对端决定。
pub const MAX_PENDING_BYTES: usize = 4096;
/// 曲线条数上限。
///
/// 存在的意义是挡住畸形数据：一行里几千个逗号会凭空创建出几千条曲线，绘制开销
/// 与内存都会失控。16 条对「同时观察几个传感器通道」远远够用，超出部分被忽略并
/// 计入 [`ChartData::ignored_fields`]，用户能在界面上看到。
pub const MAX_CHANNELS: usize = 16;

/// 样本上限设置项的取值范围。
pub const MIN_SAMPLES: usize = 10;
pub const MAX_SAMPLES: usize = 100_000;

/// 多通道数值流的解析结果与环形缓冲。
///
/// `Default` 是「还没收到任何数据」：没有通道、没有样本、也没有残留。
#[derive(Default)]
pub struct ChartData {
    /// 跨数据块的半行残留。只在没有见到换行符时才有内容。
    pending: String,
    /// 当前这一行是否已判定为超长垃圾。为真时，直到遇到换行符为止的数据全部
    /// 丢弃——否则被截断的那一行会以「尾部一个数字」的形式变成一个假样本。
    discarding_line: bool,
    /// 下一个样本组的 x（组序号）。单调递增，淘汰旧样本时不回退。
    next_x: f64,
    /// 每条曲线的样本点，`[x, y]`。下标即通道号。
    channels: Vec<VecDeque<[f64; 2]>>,
    /// 被忽略的非数值字段数（含超出通道上限的部分）。
    ignored_fields: u64,
    /// 因半行超长而被丢弃的段数。
    discarded_segments: u64,
}

impl ChartData {
    /// 送入一批刚收到的字节。
    ///
    /// `max_samples` 是**每条曲线**保留的样本上限，调用方负责夹到
    /// [`MIN_SAMPLES`]`..=`[`MAX_SAMPLES`]；这里再夹一次，避免调用方漏掉时
    /// 出现「上限为 0 ⇒ 每来一个样本就被淘汰」这种看着像不工作的状态。
    pub fn push_bytes(&mut self, bytes: &[u8], max_samples: usize) {
        if bytes.is_empty() {
            return;
        }
        let max_samples = max_samples.clamp(MIN_SAMPLES, MAX_SAMPLES);

        // 非法 UTF-8 变成 U+FFFD：数值解析会拒绝它并计数，不会 panic。
        for ch in String::from_utf8_lossy(bytes).chars() {
            match ch {
                // `\r` 也当组结束符：既兼容 CRLF，也兼容只有 CR 的老式行尾。
                // CRLF 会因此产生一个空组，而空组会被 `flush_group` 直接忽略。
                '\n' | '\r' => {
                    self.discarding_line = false;
                    self.flush_group(max_samples);
                }
                _ => {
                    if self.discarding_line {
                        continue;
                    }
                    self.pending.push(ch);
                    if self.pending.len() > MAX_PENDING_BYTES {
                        // 一行不该这么长：对端格式与约定不符。整行作废，
                        // 剩下的字节也要丢，直到换行符重新给出对齐点。
                        self.pending.clear();
                        self.discarding_line = true;
                        self.discarded_segments += 1;
                    }
                }
            }
        }
    }

    /// 把 `pending` 里累积的一行解析成一组样本。
    fn flush_group(&mut self, max_samples: usize) {
        if self.pending.is_empty() {
            return;
        }
        // 先取出再解析：下面要可变借用 `self` 的其它字段。
        let line = std::mem::take(&mut self.pending);
        let line = line.trim();
        if line.is_empty() {
            return;
        }

        // 每行一次小分配，与逐字段 parse 同阶，不值得为它引入可复用的暂存缓冲。
        let mut values: Vec<f64> = Vec::new();
        for field in line.split(',') {
            let field = field.trim();
            if field.is_empty() {
                // 连续的逗号：视作没写这个字段，而不是一个 0。
                continue;
            }
            match field.parse::<f64>() {
                // 拒绝 inf / NaN：它们会让自动缩放的纵轴范围退化成无意义的值。
                Ok(v) if v.is_finite() => values.push(v),
                _ => self.ignored_fields += 1,
            }
        }
        if values.is_empty() {
            // 整行都不是数值（例如设备混进来的纯文本日志行）：不算一组样本，
            // x 也不推进，于是图表不会被非数值内容拉出空洞。
            return;
        }

        if values.len() > self.channels.len() {
            let grow_to = values.len().min(MAX_CHANNELS);
            self.channels.resize_with(grow_to, VecDeque::new);
        }
        let kept = values.len().min(self.channels.len());
        if values.len() > kept {
            self.ignored_fields += (values.len() - kept) as u64;
        }

        let x = self.next_x;
        self.next_x += 1.0;

        for (i, value) in values.iter().take(kept).enumerate() {
            let points = &mut self.channels[i];
            points.push_back([x, *value]);
            // 按样本淘汰最旧的点：FIFO 与接收日志的 `max_log_chars` 同一思路，
            // 只是这里以「点」为单位。
            while points.len() > max_samples {
                points.pop_front();
            }
        }
    }

    /// 回到初始状态：丢弃样本、半行残留与统计，x 归零。
    pub fn clear(&mut self) {
        self.pending.clear();
        self.discarding_line = false;
        self.next_x = 0.0;
        self.channels.clear();
        self.ignored_fields = 0;
        self.discarded_segments = 0;
    }

    /// 各通道的样本点，下标即通道号。通道是在收到数据时才建立的，因此为空
    /// 表示还没有任何可用数据。
    pub fn channels(&self) -> &[VecDeque<[f64; 2]>] {
        &self.channels
    }

    /// 已建立的通道数。
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// 是否已经有任何样本可画。
    pub fn is_empty(&self) -> bool {
        self.channels.iter().all(|points| points.is_empty())
    }

    /// 全部通道的样本点总数。
    pub fn sample_count(&self) -> usize {
        self.channels.iter().map(VecDeque::len).sum()
    }

    /// 被忽略的非数值字段累计数。界面用它提示「格式可能对不上」。
    pub fn ignored_fields(&self) -> u64 {
        self.ignored_fields
    }

    /// 因半行超长被丢弃的段数。
    pub fn discarded_segments(&self) -> u64 {
        self.discarded_segments
    }
}

impl ComTool {
    /// 清空图表：丢弃全部样本、半行残留与统计，组序号归零。
    ///
    /// 包一层而不是让界面直接调 [`ChartData::clear`]：这样"清空图表还会复位什么"
    /// 有一个明确的位置，将来若要连带复位视图缩放也不会漏掉某条调用路径。
    pub fn clear_chart(&mut self) {
        self.chart.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 便捷入口：按默认上限解析一批字节，并断言没有被丢弃的段。
    fn feed(data: &str) -> ChartData {
        let mut chart = ChartData::default();
        chart.push_bytes(data.as_bytes(), 100);
        assert_eq!(chart.discarded_segments(), 0, "测试数据不应触发超长丢弃");
        chart
    }

    fn ys(chart: &ChartData, channel: usize) -> Vec<f64> {
        chart.channels()[channel]
            .iter()
            .map(|point| point[1])
            .collect()
    }

    #[test]
    fn single_channel_lines() {
        let chart = feed("1\n2.5\n-3\n");
        assert_eq!(chart.channel_count(), 1);
        assert_eq!(ys(&chart, 0), vec![1.0, 2.5, -3.0]);
        assert_eq!(chart.sample_count(), 3);
    }

    /// 一行里的第 k 个字段对应第 k 条曲线，且同一组共享同一个 x——
    /// 这是「多通道在横轴上对齐」的全部依据。
    #[test]
    fn comma_separated_fields_become_channels_sharing_one_x() {
        let chart = feed("1,2,3,4\n");
        assert_eq!(chart.channel_count(), 4, "验收要求至少 4 通道");
        for channel in 0..4 {
            assert_eq!(ys(&chart, channel), vec![(channel + 1) as f64]);
        }
        let xs: Vec<f64> = chart.channels()[0].iter().map(|p| p[0]).collect();
        assert_eq!(xs, vec![0.0]);
    }

    /// 串口分片是常态：一行可能被切在两个数据块中间。
    #[test]
    fn line_split_across_chunks_is_joined() {
        let mut chart = ChartData::default();
        chart.push_bytes(b"1,2", 100);
        assert_eq!(chart.channel_count(), 0, "没有换行符前不该产出样本");

        chart.push_bytes(b".5,3", 100);
        assert_eq!(chart.channel_count(), 0);

        chart.push_bytes(b"\n", 100);
        assert_eq!(chart.channel_count(), 3);
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(ys(&chart, 1), vec![2.5], "跨块的数字要拼成一个值");
        assert_eq!(ys(&chart, 2), vec![3.0]);
    }

    /// 一个数据块里可能有多行，也可能在某行中间结束。
    #[test]
    fn chunk_with_several_lines_and_a_partial_tail() {
        let mut chart = ChartData::default();
        chart.push_bytes(b"1\n2\n3,4", 100);
        assert_eq!(chart.channel_count(), 1, "尾部半行还不算样本");
        assert_eq!(ys(&chart, 0), vec![1.0, 2.0]);

        chart.push_bytes(b"\n", 100);
        assert_eq!(chart.channel_count(), 2, "补齐后通道数增长到 2");
        assert_eq!(ys(&chart, 0), vec![1.0, 2.0, 3.0]);
        assert_eq!(ys(&chart, 1), vec![4.0]);
    }

    #[test]
    fn crlf_and_lone_cr_both_end_a_group() {
        assert_eq!(ys(&feed("1,2\r\n"), 0), vec![1.0]);
        assert_eq!(feed("1,2\r\n").channel_count(), 2);
        // 只有 CR 的老式行尾同样成立
        assert_eq!(ys(&feed("1\r2\r"), 0), vec![1.0, 2.0]);
    }

    #[test]
    fn blank_lines_and_whitespace_are_ignored() {
        let chart = feed("\n\n   \n1\n\n");
        assert_eq!(chart.channel_count(), 1);
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(chart.sample_count(), 1);
    }

    #[test]
    fn fields_are_trimmed() {
        let chart = feed(" 1 , 2 \n");
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(ys(&chart, 1), vec![2.0]);
    }

    /// 非数值字段要计数，而不是静默跳过——用户得能看出格式不对。
    #[test]
    fn non_numeric_fields_are_counted_not_silently_dropped() {
        let chart = feed("abc,1\n");
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(chart.ignored_fields(), 1);
    }

    /// 整行都不是数值时不算一组样本，x 不推进。
    #[test]
    fn all_text_line_produces_no_sample() {
        let mut chart = ChartData::default();
        chart.push_bytes(b"hello world\n1\n", 100);
        assert_eq!(chart.channel_count(), 1);
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(
            chart.channels()[0][0][0],
            0.0,
            "纯文本行不该占用组序号，否则曲线上会出现一段空洞"
        );
        assert_eq!(
            chart.ignored_fields(),
            1,
            "按逗号切分只有「hello world」一个字段，它整体不是一个数字"
        );
    }

    #[test]
    fn empty_fields_are_skipped_without_becoming_zero() {
        let chart = feed("1,,2\n");
        assert_eq!(chart.channel_count(), 2, "空字段不占通道");
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(ys(&chart, 1), vec![2.0]);
    }

    #[test]
    fn special_float_forms_are_accepted_but_inf_nan_are_not() {
        let chart = feed("1e3,-2.5E-2,+4\n");
        assert_eq!(ys(&chart, 0), vec![1000.0]);
        assert_eq!(ys(&chart, 1), vec![-0.025]);
        assert_eq!(ys(&chart, 2), vec![4.0]);

        // inf / NaN 能 parse 成功，但会让自动缩放的纵轴范围失去意义，一律拒绝
        let chart = feed("inf,nan,NaN\n");
        assert!(chart.is_empty());
        assert_eq!(chart.ignored_fields(), 3);
    }

    /// 通道只增不减：字段变多时新建通道，早先的通道保留自己的历史。
    #[test]
    fn channel_count_grows_with_the_widest_line() {
        let chart = feed("1\n1,2,3\n1,2\n");
        assert_eq!(chart.channel_count(), 3);
        assert_eq!(ys(&chart, 0), vec![1.0, 1.0, 1.0]);
        assert_eq!(ys(&chart, 1), vec![2.0, 2.0]);
        assert_eq!(ys(&chart, 2), vec![3.0]);
    }

    /// 字段变少时，缺少的通道在该组不记样本（曲线断开），而不是补 0。
    #[test]
    fn missing_channels_get_no_sample_instead_of_zero() {
        let chart = feed("1,2,3\n9\n10\n");
        assert_eq!(chart.channel_count(), 3);
        assert_eq!(ys(&chart, 0), vec![1.0, 9.0, 10.0]);
        assert_eq!(ys(&chart, 1), vec![2.0]);
        assert_eq!(ys(&chart, 2), vec![3.0]);

        // 三组的 x 连续推进，缺样本的那两组在其它通道上存在
        let xs: Vec<f64> = chart.channels()[0].iter().map(|p| p[0]).collect();
        assert_eq!(xs, vec![0.0, 1.0, 2.0]);
    }

    /// 一行里的字段数超过通道上限时，超出部分被忽略并计数。
    #[test]
    fn channels_are_capped() {
        let fields: Vec<String> = (0..MAX_CHANNELS + 3).map(|i| i.to_string()).collect();
        let chart = feed(&(fields.join(",") + "\n"));
        assert_eq!(chart.channel_count(), MAX_CHANNELS);
        assert_eq!(chart.ignored_fields(), 3);
    }

    #[test]
    fn sample_limit_evicts_the_oldest_points() {
        let mut chart = ChartData::default();
        for i in 0..50 {
            chart.push_bytes(format!("{i}\n").as_bytes(), 10);
        }
        assert_eq!(ys(&chart, 0).len(), 10);
        assert_eq!(ys(&chart, 0), (40..50).map(f64::from).collect::<Vec<_>>());
        // x 不回退：曲线整体右移，而不是从头再来
        assert_eq!(chart.channels()[0][0][0], 40.0);
    }

    /// 上限再小也不会小于 `MIN_SAMPLES`，避免出现「一个点都留不住」的假故障。
    #[test]
    fn tiny_sample_limit_is_clamped() {
        let mut chart = ChartData::default();
        for i in 0..MIN_SAMPLES + 5 {
            chart.push_bytes(format!("{i}\n").as_bytes(), 0);
        }
        assert_eq!(ys(&chart, 0).len(), MIN_SAMPLES);
    }

    /// 半行超长是对端格式不符的信号，必须丢弃并计数，不能无限累积内存。
    #[test]
    fn overlong_unterminated_line_is_discarded_and_counted() {
        let mut chart = ChartData::default();
        let garbage = vec![b'7'; MAX_PENDING_BYTES * 3];

        chart.push_bytes(&garbage, 100);
        assert!(chart.is_empty(), "没有换行符就没有样本");
        assert_eq!(
            chart.discarded_segments(),
            1,
            "这是「丢弃了多少行」，同一行内的重复超限不该重复计数"
        );

        // 丢弃一直持续到换行符：这一行剩下的字节也算作垃圾，
        // 否则被腰斩的那一行会以「尾部一个数字」的形式变成一个假样本。
        chart.push_bytes(b"1,2\n", 100);
        assert!(chart.is_empty(), "换行之前的数据仍属于被丢弃的那一行");

        chart.push_bytes(b"3,4\n", 100);
        assert_eq!(ys(&chart, 0), vec![3.0]);
        assert_eq!(ys(&chart, 1), vec![4.0]);

        // 换行之后是新的一行：再次超长就应再记一次
        chart.push_bytes(&garbage, 100);
        chart.push_bytes(b"\n", 100);
        assert_eq!(chart.discarded_segments(), 2);
    }

    #[test]
    fn invalid_utf8_does_not_panic_and_is_counted() {
        let mut chart = ChartData::default();
        chart.push_bytes(&[0xFF, 0xFE, b'\n'], 100);
        assert!(chart.is_empty());
        assert!(chart.ignored_fields() > 0);
    }

    #[test]
    fn clear_resets_samples_residue_and_statistics() {
        let mut chart = ChartData::default();
        chart.push_bytes(b"abc,1\n", 100);
        chart.push_bytes(b"5", 100); // 半行残留
        assert!(!chart.is_empty());

        chart.clear();

        assert!(chart.is_empty());
        assert_eq!(chart.channel_count(), 0);
        assert_eq!(chart.sample_count(), 0);
        assert_eq!(chart.ignored_fields(), 0);
        assert_eq!(chart.discarded_segments(), 0);

        // 残留必须一起清掉：否则残留会和清空后的新数据拼成一行假样本
        chart.push_bytes(b"\n1\n", 100);
        assert_eq!(chart.channel_count(), 1);
        assert_eq!(ys(&chart, 0), vec![1.0]);
        assert_eq!(chart.channels()[0][0][0], 0.0, "x 应重新从 0 开始");
    }

    #[test]
    fn empty_input_is_a_noop() {
        let mut chart = ChartData::default();
        chart.push_bytes(b"", 100);
        assert!(chart.is_empty());
        assert_eq!(chart.discarded_segments(), 0);
    }
}
