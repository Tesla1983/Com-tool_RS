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

/// 绘图区高度的**上限**（像素）：空间充裕时的期望高度。
const PLOT_HEIGHT_MAX: f32 = 220.0;

/// 绘图区高度的**下限**（像素）。
///
/// 可用空间比它还小时只能照它画。再往下缩只会得到一条看不见的线：窗口已经小到放不下
/// 中央区里的东西了，此时保留"至少看得见一条趋势"比继续让步更有意义。
const PLOT_HEIGHT_MIN: f32 = 90.0;

/// 由**当前可用高度**算出绘图区高度。
///
/// 2026-09-24 修（用户问"图表的窗口高度这么小，能够正常显示数据吗"）：
/// 此前这里是写死的 [`PLOT_HEIGHT_MAX`]，实测有两个后果——
///
/// 1. **可用空间不足时它不会缩**：中央区只剩 120 点时，图表块仍然占用 251 点，
///    越界 131 点。中央区没有滚动条，超出的部分会被面板裁掉，于是夹在图表**下面**
///    的发送区整块落到可视区之外（实测：中央区高 640 点时图表底已经越界 53 点）。
/// 2. **窗口拉高也不会变大**：写死的值既不会缩也不会涨。
///
/// 现在改成在 `[下限, 上限]` 之间跟着可用高度走：**空间充足时与从前逐点完全一致**
/// （仍是 220），只在放不下时才让出高度。
fn plot_height(available: f32) -> f32 {
    if available.is_finite() {
        available.clamp(PLOT_HEIGHT_MIN, PLOT_HEIGHT_MAX)
    } else {
        // 拿不到有限值（不该发生）时退回旧行为，不要算出 NaN 让布局整体失效。
        PLOT_HEIGHT_MAX
    }
}

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
            //
            // 高度在**这一行之后**再问可用空间：标题行与格式提示行已经画完，此时
            // `available_height()` 剩下的就是"还能给绘图区多少"——比预先减一个常数准。
            let mut plot = Plot::new(PLOT_ID)
                .height(plot_height(ui.available_height()))
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

    /// 一次无头绘制取回的素材。
    struct Painted {
        /// 这次绘制占用的矩形（从画布原点算起）。
        consumed: egui::Rect,
        /// 绘图区：真正画曲线的矩形，已扣掉坐标轴。
        data_area: egui::Rect,
        /// 每条曲线的顶点。
        curves: Vec<Vec<egui::Pos2>>,
    }

    impl Painted {
        /// 全部曲线的包围盒。
        fn curve_bbox(&self) -> egui::Rect {
            let mut bbox = egui::Rect::NOTHING;
            for curve in &self.curves {
                for point in curve {
                    bbox.extend_with(*point);
                }
            }
            bbox
        }
    }

    /// 在给定可用高度下画一次图表，取回素材。
    ///
    /// 画布的**高度就是"中央区还能给图表多少"**——`Context::run_ui` 造的根 `Ui` 铺满
    /// 整幅画面，把画布收窄等价于把图表放进一个没有余量的容器。
    ///
    /// 2026-09-24 实测（4 通道 × 60 点，宽 900）：
    ///
    /// | 画布高 | 图表块占用 | 绘图区 | 曲线包围盒 y |
    /// |---|---|---|---|
    /// | 480 | 251 | 886 × 206 | 40.4 – 227.6 |
    /// | 180 | 180 | 886 × 135 | 跟随绘图区 |
    fn paint(available_height: f32, channels: usize, samples: usize) -> Painted {
        let mut app = ComTool {
            show_chart_panel: true,
            ..ComTool::default()
        };
        for i in 0..samples {
            let line: String = (0..channels)
                .map(|channel| format!("{}", 50 + ((i * 7 + channel * 13) % 40)))
                .collect::<Vec<_>>()
                .join(",")
                + "\n";
            app.chart.push_bytes(line.as_bytes(), app.chart_max_samples);
        }

        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::Dark);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, available_height),
            )),
            ..Default::default()
        };

        let mut consumed = egui::Rect::NOTHING;
        let mut extreme = egui::Color32::PLACEHOLDER;
        let mut out = ctx.run_ui(input, |ui| {
            extreme = ui.visuals().extreme_bg_color;
            app.render_chart_panel(ui);
            consumed = ui.min_rect();
        });

        let mut data_area = None;
        let mut curves = Vec::new();
        for clipped in &out.shapes {
            match &clipped.shape {
                // 绘图区：`paint_background` 用 `extreme_bg_color` 填充并带一圈描边。
                // 图例的背景框同色，但它被 `background_alpha` 乘过透明度，所以"填充色
                // 恰好等于 `extreme_bg_color`"这一条就足以把它排除在外。
                egui::epaint::Shape::Rect(rect)
                    if rect.fill == extreme && rect.stroke.width > 0.0 =>
                {
                    data_area = Some(rect.rect);
                }
                // 曲线：`Line` 画成一条多点 `Path`；网格线与坐标轴的点数少得多，
                // 用点数把两者分开。
                egui::epaint::Shape::Path(path) if path.points.len() >= 10 => {
                    curves.push(path.points.clone());
                }
                _ => {}
            }
        }
        // 不清理这个 delta 会在 drop 时 panic——看着像抓到缺陷，其实是清理没做。
        out.textures_delta.clear();

        Painted {
            consumed,
            data_area: data_area
                .unwrap_or_else(|| panic!("没找到绘图区矩形——图表没画出来，或底色/描边改了")),
            curves,
        }
    }

    /// 空间充足时绘图区高度**与修改前逐点一致**（220）。
    ///
    /// 阈值写死字面量、不引用 [`PLOT_HEIGHT_MAX`]——引用了就变成"常量改一改、断言跟着改"
    /// 的自证（本项目在连接按钮的字号上正是这么栽过两次）。
    #[test]
    fn plot_height_keeps_its_documented_value_when_there_is_room() {
        for available in [251.0, 400.0, 1000.0] {
            assert_eq!(
                plot_height(available),
                220.0,
                "可用 {available} 点时高度变了——空间够时不该有变化"
            );
        }
        assert_eq!(
            plot_height(f32::INFINITY),
            220.0,
            "拿不到有限值时应退回 220，而不是算出 NaN 让整块布局失效"
        );
    }

    /// 放不下时按可用高度缩，但不低于下限。
    #[test]
    fn plot_height_shrinks_to_fit_and_stops_at_the_floor() {
        assert_eq!(plot_height(150.0), 150.0, "放得下就该照可用高度来");
        assert_eq!(plot_height(90.0), 90.0);
        for tiny in [50.0, 0.0, -30.0] {
            assert_eq!(plot_height(tiny), 90.0, "可用 {tiny} 点时跌破了下限 90");
        }
    }

    /// ⚠️ 图表**永不占用超过给它的空间**。
    ///
    /// 用户 2026-09-24 问"图表的窗口高度这么小，能正常显示数据吗"时暴露的正是这一条的
    /// 反面：此前高度写死 220，中央区只剩 180 点时图表块仍占用 251 点，越界的 71 点被
    /// 面板裁掉，而夹在图表**下面**的发送区整块落到可视区之外（中央区没有滚动条，
    /// 用户够不着）。
    #[test]
    fn chart_never_claims_more_than_the_room_it_has() {
        const ROOM: f32 = 180.0;

        let painted = paint(ROOM, 4, 60);

        assert!(
            painted.consumed.max.y <= ROOM + 0.5,
            "画布只有 {ROOM} 点高，图表却占到了 {:.1} 点——越界 {:.1} 点，\
             下面的发送区会被挤出可视区",
            painted.consumed.max.y,
            painted.consumed.max.y - ROOM
        );
        assert!(
            painted.consumed.max.y > ROOM - 1.0,
            "只用到 {:.1} 点、留了 {:.1} 点空白——高度算小了",
            painted.consumed.max.y,
            ROOM - painted.consumed.max.y
        );
    }

    /// 缩过之后**曲线照样落在绘图区里**：缩高度不能变成"画一片空白"。
    #[test]
    fn shrunk_chart_still_draws_the_curves_inside_the_plot_area() {
        let painted = paint(180.0, 4, 60);

        assert_eq!(painted.curves.len(), 4, "四条通道应各有一条曲线");
        for curve in &painted.curves {
            assert!(
                curve.len() >= 59,
                "曲线只剩 {} 个点，这个数据量下不该发生抽样",
                curve.len()
            );
            for point in curve {
                assert!(
                    painted.data_area.contains(*point),
                    "曲线点 {point:?} 跑到绘图区 {:?} 外面了",
                    painted.data_area
                );
            }
        }
        assert!(
            painted.curve_bbox().height() >= painted.data_area.height() * 0.7,
            "曲线只占绘图区高度的 {:.0}%——自动缩放没生效，看不出数据的变化",
            painted.curve_bbox().height() / painted.data_area.height() * 100.0
        );
    }

    /// 容器够高时，留给数据的面积与修改前一模一样（绘图区 886 × 206）。
    #[test]
    fn roomy_container_gets_the_same_plot_area_as_before() {
        let painted = paint(480.0, 4, 60);

        assert!(
            painted.data_area.height() >= 200.0,
            "容器够高时绘图区只剩 {:.1} 点高（改前是 206）",
            painted.data_area.height()
        );
        assert!(
            painted.data_area.width() >= 800.0,
            "绘图区宽度只剩 {:.1} 点",
            painted.data_area.width()
        );
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
