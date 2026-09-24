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
///
/// **这是固定值，不是"期望值"**：图表块的真实高度由外层滚动块
/// （[`crate::receive::capped_scroll_area`]）决定——`ScrollArea::max_height` 是**至多**，
/// 空间不够时它自己夹住并给出滚动条，绘图区本身始终保持这个高度。
///
/// 2026-09-24 这一天它演进过三次，三条都留着（每次都是用户目视后提出的）：
///
/// 1. **写死 220 当实际高度**：`Plot::height()` 会把 `min_size.y` 与 `height` 一起写成
///    这个值并取 `at_least` ⇒ 矩形**恒为 220**，可用空间再小也不缩。中央区只剩 120 点时
///    越界 131 点，而中央区没有滚动条，被裁掉的部分正好把夹在图表**下面**的发送区顶出
///    可视区（实测中央区高 640 点时图表底已越界 53 点）。
/// 2. **按可用高度在 `[90, 220]` 之间夹**（曾有个 `plot_height(available)`）：越界没了，
///    但矮窗口下绘图区被压矮 ⇒ **纵横比失真**，曲线的趋势看起来比实际平缓——对"看趋势"
///    这件事来说，这比"要滚动"更糟。
/// 3. **交还给滚动块**（用户要求"复用日志滚动区"）：绘图区固定 220、等比例，放不下就
///    滚动；越界仍由滚动块兜住，"压扁"的问题一并消失。
const PLOT_HEIGHT_MAX: f32 = 220.0;

/// 图表的持久化 ID。同一界面里不能与其它控件重名，否则状态会互相干扰。
const PLOT_ID: &str = "realtime_chart";

/// 图表滚动块的持久化 ID。
///
/// 与 [`PLOT_ID`] 必须不同：`ScrollArea` 的滚动位置与 `Plot` 的视图范围是两份独立
/// 状态，共用一个 id 会让它们互相踩。
const CHART_SCROLL_ID: &str = "realtime_chart_scroll";

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

        // —— 页眉标题条 ——
        //
        // 与侧边栏顶部那条页眉同一套写法（`ui.heading` + 其下的分隔线），让"这一块
        // 是什么"在第一眼就成立。右侧那一排仍是原样：自动缩放、清空、样本摘要。
        ui.horizontal(|ui| {
            ui.heading(s.chart_panel_label);
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
        ui.separator();

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

        // 闭包要用的东西先全部取出来（数据、开关、高度），闭包便只捕获这些值，
        // 不与 `self` 的其它借用打架。
        let is_empty = self.chart.is_empty();
        let auto_bounds = self.chart_auto_bounds;
        let series: Vec<Vec<[f64; 2]>> = if is_empty {
            Vec::new()
        } else {
            self.chart
                .channels()
                .iter()
                .map(points_for_drawing)
                .collect()
        };

        // 绘图区高度**固定**为 [`PLOT_HEIGHT_MAX`]，不再按可用高度缩。
        //
        // 按可用高度缩是"复用日志滚动区"之前的做法：它能让图表在矮窗口下不被裁，但
        // 代价是可视区变矮 ⇒ 纵横比失真，趋势看起来比实际平缓（见 [`PLOT_HEIGHT_MAX`]）。
        //
        // 现在高度交给滚动块：`ScrollArea::max_height` 是**至多**，空间不够时它自己夹住
        // 并给出滚动条，绘图区仍是完整的 220 高、等比例，滚动即可看全。这一条也顺手
        // 替掉了"按可用高度算高度"那套——`Plot::height()` 同时写 `min_size.y` 与
        // `height` 并取 `at_least`，矩形恒为该值，曾经把它下面的发送区顶出可视区。
        crate::receive::capped_scroll_area(
            crate::receive::ScrollAxis::Vertical,
            CHART_SCROLL_ID,
            PLOT_HEIGHT_MAX,
        )
        .show(ui, |ui| {
            if is_empty {
                ui.weak(s.waiting_data);
                return;
            }
            // 图例常开：多通道时"哪条线是哪个通道"只能靠颜色对照，没有图例就得猜。
            let mut plot = Plot::new(PLOT_ID)
                .height(PLOT_HEIGHT_MAX)
                .legend(Legend::default());
            if auto_bounds {
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
        });

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
    /// | 画布高 | 图表块占用 | 绘图区 | 说明 |
    /// |---|---|---|---|
    /// | 480 | 251 | 886 × 206 | 空间够，与改动前逐点一致 |
    /// | 180 | 180 | 886 × 206 | 块被滚动区夹住，**绘图区不变**，改为可滚动 |
    ///
    /// 第三列在矮画布下**不再跟着变小**——这正是"复用日志滚动区"要的效果：被夹住的
    /// 是容器，不是内容。
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

    /// 矮画布下：图表块被**夹住**（不越界），但绘图区**保持完整**——超出部分交给滚动块。
    ///
    /// 这条正是"复用日志滚动区"与"按可用高度缩"的分界：
    ///
    /// - 只断"不越界"，会放走"顺手把高度算小"——那种做法下曲线被压扁，断言照样绿；
    /// - 只断"绘图区完整"，会放走越界——改动前的写死 220 正是"绘图区完整 + 越界 131 点"。
    ///
    /// 所以两条一起断：**块不越界** + **绘图区仍是 220**。
    #[test]
    fn squeezed_chart_clips_its_block_but_keeps_the_plot_area_whole() {
        const ROOM: f32 = 150.0;

        let painted = paint(ROOM, 4, 60);

        assert!(
            painted.consumed.max.y <= ROOM + 0.5,
            "画布只有 {ROOM} 点高，图表块却占到了 {:.1} 点（越界 {:.1} 点）——\
             滚动块没有把高度夹住",
            painted.consumed.max.y,
            painted.consumed.max.y - ROOM
        );
        assert!(
            painted.data_area.height() >= 200.0,
            "绘图区被压到只剩 {:.1} 点高（固定值是 220）——说明又退回\"按可用高度缩\"了，\
             矮窗口下曲线的纵横比会失真",
            painted.data_area.height()
        );
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

    /// 画布矮到需要滚动时，**曲线照样完整落在绘图区里**：夹高度不能变成"画一片空白"。
    #[test]
    fn squeezed_chart_still_draws_the_curves_inside_the_plot_area() {
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
