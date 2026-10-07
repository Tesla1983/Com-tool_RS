//! 应用图标：用代码在编译期"画"出来，而不是加载外部图片资源。
//!
//! 为什么不是 `assets/icon.png`：
//!
//! - **不需要资源文件**。图标永远不会缺失、不会与二进制不同步，也没有
//!   "文件读失败怎么办"这条分支。
//! - **可以被 `cargo test` 覆盖**。尺寸、透明度、渐变、前景笔画是否真的画上
//!   去了，全部能在无 GUI 的环境里断言（见文件末尾的测试）。
//! - **零新增依赖**。`eframe` 在非 wasm 目标上本来就依赖 `image`（开启 `png`
//!   特性），并且在 Windows 上会把 [`IconData`] 重新编码成 PNG 再交给
//!   `CreateIconFromResourceEx` 设置窗口图标（见 `eframe::native::app_icon`），
//!   所以这里只需要提供 RGBA 像素，PNG 编解码由 `eframe` 自己完成。
//!
//! 图案设计：圆角方形底 + 一条数字脉冲波形。选脉冲而不是正弦，是因为串口
//! 调试工具的本质是"看高低电平"，方波缩到任务栏的 16 px 依然认得出来，正弦
//! 会糊成一团。
//!
//! 抗锯齿靠超采样：每个输出像素取 `SUPERSAMPLE²` 个子样本，统计覆盖率后再
//! 合成。这样边缘是柔和的，不需要引入任何光栅化库。

use eframe::egui::IconData;

/// 输出边长（像素）。
///
/// `egui` 要求图标宽度最好为 4 的倍数；256 同时也提供了足够的分辨率，供
/// Windows 缩放到 16/32/48/256 各档。
pub const ICON_SIZE: u32 = 256;

/// 每轴超采样倍数。4 表示每个输出像素取 4×4 = 16 个子样本。
const SUPERSAMPLE: u32 = 4;

/// 圆角半径占边长的比例。
const CORNER_RADIUS_RATIO: f32 = 0.22;

/// 波形笔画半宽占边长的比例（0.052 → 全宽约为边长的 10.4%）。
const STROKE_HALF_WIDTH_RATIO: f32 = 0.052;

/// 底色渐变的上端（#22314A，深靛蓝）。
const BACKGROUND_TOP: [f32; 3] = [0.133, 0.192, 0.290];

/// 底色渐变的下端（#0D6E7A，青蓝）。
const BACKGROUND_BOTTOM: [f32; 3] = [0.051, 0.431, 0.478];

/// 波形笔画颜色（#ECFEFF，近白的微青）。
const FOREGROUND: [f32; 3] = [0.925, 0.996, 1.000];

/// 波形折线的顶点，坐标按边长归一化到 `0.0..=1.0`。
///
/// 形状是一条完整的数字脉冲：低 → 高 → 低 → 高 → 低，即两个脉冲，并且首尾
/// 都在低电平，视觉上左右对称。竖直段与水平段交替、没有斜线，缩到 16 px 时
/// 仍然是清晰的方波。
const WAVE: [[f32; 2]; 10] = [
    [0.185, 0.68],
    [0.311, 0.68],
    [0.311, 0.32],
    [0.437, 0.32],
    [0.437, 0.68],
    [0.563, 0.68],
    [0.563, 0.32],
    [0.689, 0.32],
    [0.689, 0.68],
    [0.815, 0.68],
];

/// 生成应用图标。
///
/// 纯计算：不触碰文件系统、不依赖 `egui::Context`，因此可以在单元测试里直接
/// 调用；同样的输入永远得到同样的输出。
pub fn generate() -> IconData {
    let size = ICON_SIZE as f32;
    let radius = size * CORNER_RADIUS_RATIO;
    let half_width = size * STROKE_HALF_WIDTH_RATIO;
    let half_width_sq = half_width * half_width;
    let samples_per_pixel = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    let sub_step = 1.0 / SUPERSAMPLE as f32;

    let wave: Vec<[f32; 2]> = WAVE
        .iter()
        .map(|point| [point[0] * size, point[1] * size])
        .collect();
    let wave_bounds = bounding_box(&wave, half_width);

    let mut rgba = Vec::with_capacity((ICON_SIZE as usize) * (ICON_SIZE as usize) * 4);

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let mut background_hits = 0.0_f32;
            let mut foreground_hits = 0.0_f32;
            let mut background_sum = [0.0_f32; 3];

            for sub_y in 0..SUPERSAMPLE {
                for sub_x in 0..SUPERSAMPLE {
                    // 子样本取方格中心，避免所有样本落在像素边界上导致覆盖率
                    // 系统性偏斜。
                    let px = x as f32 + (sub_x as f32 + 0.5) * sub_step;
                    let py = y as f32 + (sub_y as f32 + 0.5) * sub_step;

                    if !inside_rounded_rect(px, py, size, radius) {
                        continue;
                    }

                    background_hits += 1.0;
                    let color = background_color(py / size);
                    background_sum[0] += color[0];
                    background_sum[1] += color[1];
                    background_sum[2] += color[2];

                    if inside_wave(px, py, &wave, half_width_sq, wave_bounds) {
                        foreground_hits += 1.0;
                    }
                }
            }

            if background_hits == 0.0 {
                // 形状之外：全透明。RGB 保持 0，不给下游留"透明的黑"。
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }

            // 底色是竖直渐变，同一像素内各子样本的颜色略有差异，先按命中数取
            // 均值。这里除以 background_hits 而不是子样本总数，是为了不把覆盖
            // 率重复乘进颜色里（覆盖率最后只进 alpha 通道）。
            let background_avg = [
                background_sum[0] / background_hits,
                background_sum[1] / background_hits,
                background_sum[2] / background_hits,
            ];

            // 笔画完全落在圆角矩形内部（由测试 `wave_stays_inside_the_shape`
            // 固定住这个前提），所以前景命中数不会超过背景命中数，这里的比值
            // 天然落在 0..=1，可以直接当"背景之上覆盖了多少前景"用。
            let foreground_ratio = foreground_hits / background_hits;

            for (channel, background) in background_avg.iter().enumerate() {
                rgba.push(unit_to_byte(lerp(
                    *background,
                    FOREGROUND[channel],
                    foreground_ratio,
                )));
            }

            // alpha = 底色覆盖率。边缘一圈由此得到半透明像素；形状外已提前
            // 返回，所以这里只会落在 1..=255。
            rgba.push(unit_to_byte(background_hits / samples_per_pixel));
        }
    }

    debug_assert_eq!(
        rgba.len(),
        (ICON_SIZE as usize) * (ICON_SIZE as usize) * 4,
        "图标缓冲长度必须与宽高一致"
    );

    IconData {
        rgba,
        width: ICON_SIZE,
        height: ICON_SIZE,
    }
}

/// 折线的包围盒，四个方向各外扩 `padding`。
fn bounding_box(points: &[[f32; 2]], padding: f32) -> (f32, f32, f32, f32) {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for point in points {
        min_x = min_x.min(point[0]);
        min_y = min_y.min(point[1]);
        max_x = max_x.max(point[0]);
        max_y = max_y.max(point[1]);
    }
    (
        min_x - padding,
        min_y - padding,
        max_x + padding,
        max_y + padding,
    )
}

/// 圆角矩形的内部判定（左上角为原点，边长 `size`）。
///
/// 做法是把点折到形状中心所在的象限，然后分两种情形：落在"十字"区域内时只有
/// 直边约束，落在角上时用圆的标准方程判定。
fn inside_rounded_rect(px: f32, py: f32, size: f32, radius: f32) -> bool {
    let half = size / 2.0;
    let dx = (px - half).abs();
    let dy = (py - half).abs();
    let straight_span = half - radius;

    if dx > half || dy > half {
        return false;
    }
    if dx <= straight_span || dy <= straight_span {
        return true;
    }

    let corner_x = dx - straight_span;
    let corner_y = dy - straight_span;
    corner_x * corner_x + corner_y * corner_y <= radius * radius
}

/// 底色（竖直方向的两段线性渐变）。
fn background_color(t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        lerp(BACKGROUND_TOP[0], BACKGROUND_BOTTOM[0], t),
        lerp(BACKGROUND_TOP[1], BACKGROUND_BOTTOM[1], t),
        lerp(BACKGROUND_TOP[2], BACKGROUND_BOTTOM[2], t),
    ]
}

/// 点是否落在波形的笔画上。
///
/// `bounds` 是笔画的包围盒。绝大多数像素落在盒外，先做一次廉价矩形判定就能
/// 跳掉下面 9 次线段距离计算——生成一次图标要跑一百万个子样本，这个早退把
/// debug 构建下的启动开销砍掉约三分之二。
fn inside_wave(
    px: f32,
    py: f32,
    wave: &[[f32; 2]],
    half_width_sq: f32,
    bounds: (f32, f32, f32, f32),
) -> bool {
    let (min_x, min_y, max_x, max_y) = bounds;
    if px < min_x || px > max_x || py < min_y || py > max_y {
        return false;
    }

    wave.windows(2)
        .any(|segment| distance_sq_to_segment(px, py, segment[0], segment[1]) <= half_width_sq)
}

/// 点到线段的最短距离的平方（不开方，省一次 `sqrt`）。
fn distance_sq_to_segment(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ab_x, ab_y) = (b[0] - a[0], b[1] - a[1]);
    let (ap_x, ap_y) = (px - a[0], py - a[1]);
    let length_sq = ab_x * ab_x + ab_y * ab_y;

    // 退化成一点时直接取到该点的距离。这里不能只靠下面的 `clamp` 兜底：
    // `length_sq` 为 0 时相除会得到 NaN，而 `f32::clamp` 遇到 NaN 原样返回。
    let t = if length_sq <= f32::EPSILON {
        0.0
    } else {
        ((ap_x * ab_x + ap_y * ab_y) / length_sq).clamp(0.0, 1.0)
    };

    let (dx, dy) = (ap_x - ab_x * t, ap_y - ab_y * t);
    dx * dx + dy * dy
}

/// 线性插值。
fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// 把 `0.0..=1.0` 的分量转成 `0..=255` 的字节。
fn unit_to_byte(value: f32) -> u8 {
    // 先夹再舍入：超采样统计出的覆盖率理论上不会越界，但浮点误差可能给出
    // 1.0000001 这种值，不夹会回绕成 0。
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取某个像素的 RGBA。
    fn pixel(icon: &IconData, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * icon.width + x) * 4) as usize;
        [
            icon.rgba[offset],
            icon.rgba[offset + 1],
            icon.rgba[offset + 2],
            icon.rgba[offset + 3],
        ]
    }

    /// 统计某个纵向区间内**完全不透明**像素的平均 RGB。
    ///
    /// 只取 alpha == 255 的像素，免得把抗锯齿产生的过渡色算进来。
    fn average_opaque_color(icon: &IconData, y_from: u32, y_to: u32) -> [f32; 3] {
        let mut sum = [0.0_f32; 3];
        let mut count = 0.0_f32;
        for y in y_from..y_to {
            for x in 0..icon.width {
                let [r, g, b, a] = pixel(icon, x, y);
                if a == 255 {
                    sum[0] += r as f32;
                    sum[1] += g as f32;
                    sum[2] += b as f32;
                    count += 1.0;
                }
            }
        }
        assert!(count > 0.0, "区间 {y_from}..{y_to} 内没有任何不透明像素");
        [sum[0] / count, sum[1] / count, sum[2] / count]
    }

    #[test]
    fn icon_is_square_and_multiple_of_four() {
        let icon = generate();
        assert_eq!(icon.width, ICON_SIZE);
        assert_eq!(icon.height, ICON_SIZE);
        assert_eq!(icon.width % 4, 0, "egui 要求宽度为 4 的倍数");
        assert_eq!(
            icon.rgba.len(),
            (ICON_SIZE as usize) * (ICON_SIZE as usize) * 4,
            "RGBA 缓冲长度必须与宽高一致，否则 eframe 会拒绝这个图标"
        );
        assert!(!icon.is_empty());
    }

    /// 四角必须在圆角之外，也就是全透明——这正是"圆角矩形"区别于"矩形"之处。
    #[test]
    fn four_corners_are_transparent() {
        let icon = generate();
        for (x, y) in [
            (0, 0),
            (ICON_SIZE - 1, 0),
            (0, ICON_SIZE - 1),
            (ICON_SIZE - 1, ICON_SIZE - 1),
        ] {
            assert_eq!(pixel(&icon, x, y)[3], 0, "角点 ({x},{y}) 应当透明");
        }
    }

    /// 正中心必须完全实心：它离所有边界都足够远。
    #[test]
    fn center_is_fully_opaque() {
        let icon = generate();
        assert_eq!(
            pixel(&icon, ICON_SIZE / 2, ICON_SIZE / 2)[3],
            255,
            "中心像素应当完全不透明"
        );
    }

    /// 边缘要有半透明像素，否则说明超采样没生效（退化成硬边）。
    #[test]
    fn edges_are_antialiased() {
        let icon = generate();
        let partial = icon
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0 && px[3] < 255)
            .count();
        assert!(
            partial > 100,
            "抗锯齿边缘只有 {partial} 个半透明像素，超采样可能没生效"
        );
        assert!(
            (partial as u32) < ICON_SIZE * ICON_SIZE / 2,
            "半透明像素多达 {partial} 个，形状判定可能写错了"
        );
    }

    /// 前景波形必须真的画上去了：存在足够多的近白像素。
    #[test]
    fn wave_is_drawn() {
        let icon = generate();
        let bright = icon
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] > 200 && px[1] > 200 && px[2] > 200)
            .count();
        // 笔画中心线总长约 530 px、全宽约 26.6 px，覆盖率粗算在两成上下；
        // 这里只要求落在一个宽松但能区分"没画"与"糊满"的区间里。
        let lower = ((ICON_SIZE * ICON_SIZE) as usize) / 40;
        let upper = ((ICON_SIZE * ICON_SIZE) as usize) / 3;
        assert!(
            (lower..upper).contains(&bright),
            "近白像素 {bright} 个，不在预期的 {lower}..{upper} 区间内"
        );
    }

    /// 底色是竖直渐变：下方比上方更青（蓝绿更高、红更低）。
    ///
    /// 只取顶部与底部各 15% 的高度。波形笔画的纵向范围约在 27%~73%，伸不到
    /// 这两条带子里，所以带内的不透明像素必然是纯底色——这条断言与波形具体
    /// 形状无关，改波形不会让它失效。
    #[test]
    fn background_gradient_runs_top_to_bottom() {
        let icon = generate();
        let band = ICON_SIZE * 15 / 100;
        let top = average_opaque_color(&icon, 0, band);
        let bottom = average_opaque_color(&icon, ICON_SIZE - band, ICON_SIZE);

        assert!(
            top[0] > bottom[0],
            "上端红色分量 {:.1} 应当高于下端 {:.1}",
            top[0],
            bottom[0]
        );
        assert!(
            bottom[1] > top[1],
            "下端绿色分量 {:.1} 应当高于上端 {:.1}",
            bottom[1],
            top[1]
        );
        assert!(
            bottom[2] > top[2],
            "下端蓝色分量 {:.1} 应当高于上端 {:.1}",
            bottom[2],
            top[2]
        );
    }

    /// 波形必须整个落在圆角矩形内部。
    ///
    /// 生成算法用"前景命中数 ÷ 背景命中数"当作前景占比，这个比值成立的前提
    /// 就是笔画不越界。这里用逐点采样把该前提变成可执行断言：将来若调整波形
    /// 坐标或圆角半径导致越界，这条测试会先失败，而不是悄悄生成一张笔画被
    /// 啃掉一块的图标。
    #[test]
    fn wave_stays_inside_the_shape() {
        let size = ICON_SIZE as f32;
        let radius = size * CORNER_RADIUS_RATIO;
        let half_width = size * STROKE_HALF_WIDTH_RATIO;

        // 沿笔画中心线密集取样，再向四周各偏一个笔画半宽。
        for segment in WAVE.windows(2) {
            let a = [segment[0][0] * size, segment[0][1] * size];
            let b = [segment[1][0] * size, segment[1][1] * size];
            for step in 0..=32 {
                let t = step as f32 / 32.0;
                let cx = lerp(a[0], b[0], t);
                let cy = lerp(a[1], b[1], t);
                for (dx, dy) in [
                    (0.0, 0.0),
                    (half_width, 0.0),
                    (-half_width, 0.0),
                    (0.0, half_width),
                    (0.0, -half_width),
                ] {
                    assert!(
                        inside_rounded_rect(cx + dx, cy + dy, size, radius),
                        "笔画上的点 ({:.1},{:.1}) 越出了圆角矩形",
                        cx + dx,
                        cy + dy
                    );
                }
            }
        }
    }

    /// 同样的输入必须给出同样的输出（没有依赖随机数或系统状态）。
    #[test]
    fn generation_is_deterministic() {
        assert_eq!(generate(), generate());
    }

    /// 透明像素的 RGB 也要是 0，避免下游把"透明的黑"当黑色渲染。
    #[test]
    fn transparent_pixels_have_zero_rgb() {
        let icon = generate();
        for (index, px) in icon.rgba.as_chunks::<4>().0.iter().enumerate() {
            if px[3] == 0 {
                assert_eq!(
                    &px[..3],
                    &[0, 0, 0],
                    "第 {index} 个像素全透明却带着非零 RGB"
                );
            }
        }
    }

    /// 开发时用来"看一眼"图标的 ASCII 预览，默认不跑。
    ///
    /// 图标是纯视觉产物：上面的断言能保证"形状画出来了、边缘是柔和的"，但保证
    /// 不了"看起来认得出来"。调整 [`WAVE`] 顶点或配色之后，执行
    ///
    /// ```text
    /// cargo test app_icon::tests::ascii_preview -- --ignored --nocapture
    /// ```
    ///
    /// 在终端里核对圆角与波形。字符越靠后越亮，空格表示完全透明。
    #[test]
    #[ignore = "开发时手工执行：打印 ASCII 预览，不参与常规测试"]
    fn ascii_preview() {
        /// 预览宽度（字符数）；高度按正方形换算。
        const COLUMNS: usize = 72;
        /// 由暗到亮的字符梯度。
        const RAMP: &[u8] = b" .:-=+*#%@";

        let icon = generate();
        let step = (ICON_SIZE as usize) / COLUMNS;
        assert!(step > 0, "预览宽度不能超过图标边长");

        let mut preview = String::new();
        for y in (0..ICON_SIZE as usize).step_by(step) {
            for x in (0..ICON_SIZE as usize).step_by(step) {
                let offset = (y * icon.width as usize + x) * 4;
                let [r, g, b, a] = [
                    icon.rgba[offset],
                    icon.rgba[offset + 1],
                    icon.rgba[offset + 2],
                    icon.rgba[offset + 3],
                ];
                // 感知亮度 × 不透明度：透明处直接落成空格，半透明边缘自然变暗。
                let luminance = if a == 0 {
                    0.0
                } else {
                    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
                        * (a as f32 / 255.0)
                };
                let last = RAMP.len() - 1;
                let index = ((luminance * last as f32) + 0.5) as usize;
                preview.push(RAMP[index.min(last)] as char);
            }
            preview.push('\n');
        }

        println!("{preview}");
    }

    #[test]
    fn distance_to_segment_handles_degenerate_segment() {
        // 退化成一个点：距离就是到该点的欧氏距离的平方。
        let d = distance_sq_to_segment(3.0, 4.0, [0.0, 0.0], [0.0, 0.0]);
        assert!((d - 25.0).abs() < 1e-3, "期望 25，得到 {d}");

        // 投影落在端点之外时取到端点的距离。
        let d = distance_sq_to_segment(-3.0, 0.0, [0.0, 0.0], [1.0, 0.0]);
        assert!((d - 9.0).abs() < 1e-3, "期望 9，得到 {d}");

        // 投影落在段内时取垂距。
        let d = distance_sq_to_segment(0.5, 2.0, [0.0, 0.0], [1.0, 0.0]);
        assert!((d - 4.0).abs() < 1e-3, "期望 4，得到 {d}");
    }

    #[test]
    fn unit_to_byte_saturates_instead_of_wrapping() {
        assert_eq!(unit_to_byte(0.0), 0);
        assert_eq!(unit_to_byte(1.0), 255);
        assert_eq!(unit_to_byte(1.000_000_1), 255, "超过 1 不能回绕成 0");
        assert_eq!(unit_to_byte(-0.5), 0);
        assert_eq!(unit_to_byte(0.5), 128);
    }
}
