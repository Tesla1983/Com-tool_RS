//! 实时图表面板：把 [`crate::chart`] 解析出的多通道样本画成折线。
//!
//! 用 `egui_plot` 而不是手绘坐标轴（参考实现是手绘的、且只有单通道）：缩放、拖拽、
//! 自动缩放、图例、刻度都由 crate 提供，而这些能力对"看清一条实时曲线"是必需的。
//! 数据的解析规则与格式约定见 [`crate::chart`] 的模块文档——本模块只负责画。
//!
//! 与接收视图的关系：图表吃的是**原始字节**，与当前是文本 / HEX / 字节查看哪一种
//! 显示模式无关。这不是随手决定的——三种显示模式是对同一份数据的不同解释，而图表
//! 是并列的第四种；用户在 HEX 模式下盯着字节看时，没有理由让曲线停掉。

use crate::app::ComTool;
use crate::strings::fill;
use eframe::egui;
use egui::Ui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::VecDeque;

/// 绘图区高度（像素）。
const PLOT_HEIGHT: f32 = 220.0;

/// 图表的持久化 ID。同一界面里不能与其它控件重名，否则状态会互相干扰。
const PLOT_ID: &str = "realtime_chart";

/// 单帧、单通道最多绘制的点数。
///
/// 样本上限是用户可以调到 10 万的设置项，而 `egui_plot` 每帧都要把点转成它自己
/// 的结构——10 万点 × 16 通道会让重绘本身成为瓶颈。超过这个值时按等间隔抽样：
/// 曲线形状仍然可读，界面不会因为一个设置项而卡死。
const MAX_DRAWN_POINTS: usize = 4_000;

/// 曲线配色，按通道号循环取用。
///
/// 用固定的高辨识度色值而不是主题色：曲线颜色是**数据编码**（哪条线是哪个通道），
/// 必须与主题无关地保持一致；这些色值在浅色与深色背景上都有足够对比度。
const CHANNEL_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0xE5, 0x48, 0x4D), // 红
    egui::Color32::from_rgb(0x3E, 0x8D, 0xF7), // 蓝
    egui::Color32::from_rgb(0x2F, 0xA8, 0x4F), // 绿
    egui::Color32::from_rgb(0xF5, 0xA5, 0x24), // 橙
    egui::Color32::from_rgb(0x93, 0x53, 0xD3), // 紫
    egui::Color32::from_rgb(0x12, 0xA5, 0xB8), // 青
    egui::Color32::from_rgb(0xE9, 0x3D, 0x82), // 粉
    egui::Color32::from_rgb(0x9B, 0xBF, 0x1D), // 黄绿
];

/// 曲线颜色按通道号循环取用。
fn channel_color(index: usize) -> egui::Color32 {
    CHANNEL_COLORS[index % CHANNEL_COLORS.len()]
}

/// 把一条曲线的样本整理成待绘制的点序列（必要时等间隔抽样）。
///
/// 抽样时**必须保留最后一个点**：那是刚收到的数据，也正是用户盯着图表要看的东西；
/// 只按步长取点会把曲线末端切掉，看起来像"数据停了"。
fn points_for_drawing(points: &VecDeque<[f64; 2]>) -> Vec<[f64; 2]> {
    let len = points.len();
    if len <= MAX_DRAWN_POINTS {
        return points.iter().copied().collect();
    }

    let step = len.div_ceil(MAX_DRAWN_POINTS);
    let mut drawn: Vec<[f64; 2]> = points.iter().step_by(step).copied().collect();
    if let (Some(&tail), Some(&last_drawn)) = (points.back(), drawn.last())
        && tail != last_drawn
    {
        drawn.push(tail);
    }
    drawn
}

impl ComTool {
    /// 绘制实时图表面板（`show_chart_panel` 为 false 时直接返回）。
    ///
    /// 方法名用 `render_` 而不是 `show_`：字段 [`ComTool::show_chart_panel`] 已经占了
    /// 「要不要显示」这个名字，两者同名会让调用处难分"读状态"还是"画界面"。
    pub(crate) fn render_chart_panel(&mut self, ui: &mut Ui) {
        if !self.show_chart_panel {
            return;
        }

        // 清空是状态变更，不能在绘制闭包里直接做：先记录意图，画完再派发
        // （与 sidebar / settings 的 action 同一条约定）。
        let mut clear_requested = false;
        let s = self.language.strings();

        ui.horizontal(|ui| {
            ui.label(s.chart_panel_label);
            ui.checkbox(&mut self.chart_auto_bounds, s.auto_bounds);
            if crate::button::standard(ui, s.clear_chart).clicked() {
                clear_requested = true;
            }
            ui.weak(fill(
                s.chart_summary,
                &[
                    &self.chart.channel_count(),
                    &self.chart.sample_count(),
                    &self.chart_max_samples,
                ],
            ));
        });

        // 解析异常时给出可诊断的提示。用户配置的格式对不上时，最需要看到的
        // 不是一片空白，而是"有东西被忽略了、忽略了多少"。
        let ignored = self.chart.ignored_fields();
        let discarded = self.chart.discarded_segments();
        if ignored > 0 || discarded > 0 {
            let mut detail: Vec<String> = Vec::new();
            if ignored > 0 {
                detail.push(fill(s.ignored_fields, &[&ignored]));
            }
            if discarded > 0 {
                detail.push(fill(s.discarded_segments, &[&discarded]));
            }
            ui.weak(fill(s.chart_format_hint, &[&detail.join(" · ")]));
        }

        if self.chart.is_empty() {
            ui.weak(s.waiting_data);
        } else {
            // 先把要画的数据整理好再进闭包：`Plot::show` 的闭包捕获 `series`，
            // 不借用 `self`，于是这段代码不会与 `self` 的其它可变借用打架。
            let series: Vec<Vec<[f64; 2]>> = self
                .chart
                .channels()
                .iter()
                .map(points_for_drawing)
                .collect();

            // 图例常开：多通道时"哪条线是哪个通道"只能靠颜色对照，没有图例就得猜。
            let mut plot = Plot::new(PLOT_ID)
                .height(PLOT_HEIGHT)
                .legend(Legend::default());
            if self.chart_auto_bounds {
                // 关掉时不设置，`egui_plot` 便会保留用户拖动 / 缩放后的视图。
                plot = plot.auto_bounds(true);
            }
            plot.show(ui, |plot_ui| {
                for (index, points) in series.into_iter().enumerate() {
                    if points.is_empty() {
                        continue;
                    }
                    plot_ui.line(
                        Line::new(format!("CH{}", index + 1), PlotPoints::from(points))
                            .color(channel_color(index))
                            .width(1.5),
                    );
                }
            });
        }

        if clear_requested {
            self.clear_chart();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deque(points: &[[f64; 2]]) -> VecDeque<[f64; 2]> {
        points.iter().copied().collect()
    }

    #[test]
    fn short_series_is_passed_through_unchanged() {
        let points = deque(&[[0.0, 1.0], [1.0, 2.0]]);
        assert_eq!(points_for_drawing(&points), vec![[0.0, 1.0], [1.0, 2.0]]);
    }

    #[test]
    fn empty_series_stays_empty() {
        assert!(points_for_drawing(&VecDeque::new()).is_empty());
    }

    /// 超过绘制上限时抽样，但**末端必须保留**：那是最新数据。
    #[test]
    fn long_series_is_sampled_but_the_tail_is_kept() {
        let points: VecDeque<[f64; 2]> = (0..MAX_DRAWN_POINTS * 3)
            .map(|i| [i as f64, i as f64])
            .collect();

        let drawn = points_for_drawing(&points);

        assert!(
            drawn.len() <= MAX_DRAWN_POINTS + 1,
            "抽样后不该还接近原始长度，实际 {}",
            drawn.len()
        );
        assert_eq!(
            drawn.last().copied(),
            points.back().copied(),
            "最后一个点是最新数据，必须在曲线上"
        );
    }

    /// 抽样后的点仍必须是原序列的子序列（按序、不造点）。
    #[test]
    fn sampled_points_are_a_subsequence_of_the_original() {
        let points: VecDeque<[f64; 2]> = (0..MAX_DRAWN_POINTS * 2 + 7)
            .map(|i| [i as f64, -(i as f64)])
            .collect();
        let drawn = points_for_drawing(&points);

        let mut cursor = points.iter();
        for point in &drawn {
            assert!(
                cursor.any(|candidate| candidate == point),
                "抽样点 {point:?} 不在原序列里或顺序被打乱"
            );
        }
    }

    #[test]
    fn channel_colors_cycle_and_stay_in_palette() {
        assert_eq!(channel_color(0), CHANNEL_COLORS[0]);
        assert_eq!(channel_color(7), CHANNEL_COLORS[7]);
        assert_eq!(
            channel_color(CHANNEL_COLORS.len()),
            CHANNEL_COLORS[0],
            "第 9 条曲线应回到第一个颜色"
        );
    }
}
