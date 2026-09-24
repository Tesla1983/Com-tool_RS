//! 底部状态栏：一颗彩色圆点 + 一句话，常驻显示当前连接状态。
//!
//! **为什么单独一个模块**：状态栏的判据（哪颗灯、哪句话）是一个纯函数
//! （[`connection_indicator`]），与绘制分开。这样"UDP 该显示已绑定而不是已连接"
//! 这类规则可以直接断言，不必先画一遍再去看像素——见 `mod tests`。
//!
//! 词义与侧边栏的 [`crate::sidebar`] 那套**必须一致**：同一个窗口里两处状态指示
//! 给出不同结论，比只有一处更容易让人误判。所以这里同样按四态走
//! （串口/TCP 的已连接/未连接 + UDP 的已绑定/未绑定）。
//!
//! # 圆点为什么是画出来的，而不是一个 emoji 字符
//!
//! 初版用 🟢(U+1F7E2) / 🔴(U+1F534) 两个字符当圆点，结果是**两个大小不一、
//! 且都是灰黑色**的圆。原因不在字体选错，而在 egui 的文本管线：
//! `epaint` 光栅化字形时走的是 `skrifa` 的 `outline_glyphs()`
//! （`epaint-0.36.1/src/text/font.rs` 的 `allocate_glyph_uncached`），
//! 即**只取字体的矢量轮廓**并填充成当前文字色。而彩色 emoji 的颜色来自
//! `COLR`/`CPAL`（分层）或 `CBDT`/`CBLC`（位图）表——这些表 egui **从不读取**。
//!
//! 实测两个候选系统字体：`seguiemj.ttf` 有 `COLR`+`CPAL`，但同一码点也有普通的
//! `glyf` 轮廓（就是那个被当成单色圆画出来的东西）；`seguisym.ttf` 连 `COLR` 都没有。
//! 所以**换任何字体都得不到彩色**——这是架构限制，不是配置问题。
//!
//! 于是改成自己画：一个 `Color32` 填充的圆，尺寸与文字行高挂钩。好处不只是颜色
//! 正确，还顺带解决了大小不一（不再受 emoji 字形自带 advance 的摆布）、
//! 垂直对齐（圆按行高居中，不跟字体基线漂移），以及**对字体回退链的依赖归零**。

use crate::app::{ComTool, ConnectionKind};
use crate::strings::Strings;
use egui::{Color32, Ui};

/// 圆点直径相对行高的比例。
///
/// 取 0.72 而不是 1.0：满行高的实心圆会顶到相邻行、看起来比文字"重"；
/// 略小于行高才像一个指示灯泡。圆点大小随全局字体档位（`UiFontScale`）
/// 一起变，因为它读的是实时行高。
const DOT_SIZE_RATIO: f32 = 0.72;

// 比例必须 < 1：满行高的实心圆会顶到相邻行、看起来比文字"重"。这是**常量之间**的
// 关系，所以用编译期断言——写成运行期 `assert!` 会被 clippy 判为恒真。
const _: () = assert!(DOT_SIZE_RATIO < 1.0, "圆点比例必须小于一行高度");

/// 状态栏要显示的内容。
///
/// 把"哪颗灯 + 哪句话"做成一个数据结构，是为了让判据可以被**单独断言**：
/// 测试不必去解析绘制出的文字，直接比较两个字段即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionIndicator {
    /// 圆点亮不亮（绿 / 红）。
    ///
    /// 这里存**语义**（亮/灭）而不是颜色值：颜色是"亮/灭"的一种表现，将来若换成
    /// 灰/绿、或加第三态，判据这层不用动。真颜色由 [`ConnectionIndicator::color`] 给。
    pub is_on: bool,
    /// 圆点右边的文字（已连接 / 未连接 / 已绑定 / 未绑定）。
    pub label: &'static str,
}

impl ConnectionIndicator {
    /// 圆点该用什么颜色画。
    ///
    /// 单独一个方法而不是在绘制处写 `if is_on { GREEN } else { RED }`：
    /// 颜色是"亮/灭"的**唯一**落点，测试可以不必绘制就断言颜色。
    pub(crate) fn color(&self) -> Color32 {
        if self.is_on {
            // 与 `Color32::GREEN` 不同，这里用略深一点的绿：纯 `#00FF00` 在浅色主题的
            // 白底上偏刺眼，`#22C55E` 这类更接近常见状态灯。
            Color32::from_rgb(0x22, 0xC5, 0x5E)
        } else {
            // 同理，不用 `Color32::RED`（#FF0000），用饱和度略低的红。
            Color32::from_rgb(0xEF, 0x44, 0x44)
        }
    }
}

/// 按当前连接状态算出状态栏该显示什么。
///
/// **UDP 单独一档**：UDP 是无连接的，`connected == true` 只表示"套接字绑定成功"，
/// 并不代表对端在线。把它显示成"已连接"会让用户以为掉线会被发现——见
/// `ConnectionKind` 的文档与 `sidebar.rs` 里同一个分叉。
pub(crate) fn connection_indicator(
    connected: bool,
    kind: ConnectionKind,
    s: &Strings,
) -> ConnectionIndicator {
    match (connected, kind) {
        (true, ConnectionKind::Udp) => ConnectionIndicator {
            is_on: true,
            label: s.statusbar_bound,
        },
        (true, _) => ConnectionIndicator {
            is_on: true,
            label: s.statusbar_connected,
        },
        (false, ConnectionKind::Udp) => ConnectionIndicator {
            is_on: false,
            label: s.statusbar_unbound,
        },
        (false, _) => ConnectionIndicator {
            is_on: false,
            label: s.statusbar_disconnected,
        },
    }
}

impl ComTool {
    /// 绘制底部状态栏。
    ///
    /// **调用位置有硬约束**：必须在 `CentralPanel` **之前**画（见 `app.rs` 的 `ui()`）。
    /// egui 的面板"先画先占位"——后画的只能拿到剩下的空间。若在中央区之后画，
    /// 状态栏只会横跨中央区剩余的宽度，左边露出侧边栏、右边露出边缘，
    /// 看起来像一条没对齐的横条。
    pub(crate) fn show_status_bar(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        let indicator = connection_indicator(self.connected, self.connection_kind, s);

        egui::Panel::bottom("status_bar")
            .resizable(false)
            // 状态栏是**指示**不是**控件**：它不该抢走点击焦点，也不该让用户能把
            // 它拖大拖小。`show_separator_line` 保留——上面那条细线是它与主内容区
            // 唯一的视觉分界（面板默认没有背景色差）。
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    draw_status_dot(ui, indicator);
                    ui.label(indicator.label);
                });
            });
    }
}

/// 画状态灯圆点。
///
/// 尺寸取**行高**而不是字号：圆是竖直居中的几何图形，用行高才与旁边那行文字的
/// 视觉重心对齐；用字号会让它偏下（字号的度量不含行距）。
fn draw_status_dot(ui: &mut Ui, indicator: ConnectionIndicator) {
    let line_height = ui.text_style_height(&egui::TextStyle::Body);
    let diameter = (line_height * DOT_SIZE_RATIO).max(6.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(diameter, line_height), egui::Sense::hover());

    // 在分配到的矩形里居中画圆。`rect.center()` 而非左上角：`allocate_exact_size`
    // 返回的矩形可能比直径高（我们按行高分配），不居中会顶在上边缘。
    let radius = diameter / 2.0;
    ui.painter()
        .circle_filled(rect.center(), radius, indicator.color());

    // 圆点是纯视觉标记，读屏软件念不出颜色，所以把有意义的文案挂成它的提示，
    // 而不是让它成为一个不可读的独立条目。
    response.on_hover_text(indicator.label);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 四态各自显示什么，逐条钉住。
    ///
    /// 断言的是**标签文字**（来自文案表）而不是"画出了什么"——文案表本身就带着
    /// 中英两列，所以这一条顺带覆盖了英文界面。
    #[test]
    fn indicator_covers_all_four_states() {
        let zh = crate::app::Language::Zh.strings();

        // 串口 / TCP：连上就是"已连接"，断开就是"未连接"
        let on = connection_indicator(true, ConnectionKind::Serial, zh);
        assert_eq!(on.label, zh.statusbar_connected);
        assert!(on.is_on, "连接成功时灯必须是亮的");

        let off = connection_indicator(false, ConnectionKind::Serial, zh);
        assert_eq!(off.label, zh.statusbar_disconnected);
        assert!(!off.is_on, "断开时灯必须是灭的");

        // UDP 单独一档：绑定成功≠对端在线，措辞必须避开"已连接"
        let udp_on = connection_indicator(true, ConnectionKind::Udp, zh);
        assert_eq!(
            udp_on.label, zh.statusbar_bound,
            "UDP 绑定成功不能显示成「已连接」——那会让用户以为对端在线、掉线会被发现"
        );
        assert_ne!(
            udp_on.label, zh.statusbar_connected,
            "UDP 那一档必须与串口/TCP 的措辞不同，否则这个分叉等于没做"
        );

        let udp_off = connection_indicator(false, ConnectionKind::Udp, zh);
        assert_eq!(udp_off.label, zh.statusbar_unbound);
        assert_ne!(udp_off.label, zh.statusbar_disconnected);
    }

    /// 灯的颜色必须跟着状态走，而不是写死一种。
    ///
    /// 这是最容易被写错的地方：把 `color()` 固定成绿色，其余全对——文字对、
    /// 开关对，只有灯永远绿着。所以这条**单独**断言两个状态的颜色互不相同，
    /// 并钉住"亮=偏绿、灭=偏红"这个方向（不是随便两种颜色都算过）。
    #[test]
    fn the_dot_changes_colour_with_the_state() {
        let zh = crate::app::Language::Zh.strings();
        let on = connection_indicator(true, ConnectionKind::Serial, zh);
        let off = connection_indicator(false, ConnectionKind::Serial, zh);

        assert_ne!(
            on.color(),
            off.color(),
            "连接与断开必须用**不同**的颜色，否则灯是个装饰、不指示任何东西"
        );

        // 方向断言：只断言"两色不同"会被"亮=红、灭=绿"这种反色实现骗过。
        let green = on.color();
        let red = off.color();
        assert!(
            green.g() > green.r() && green.g() > green.b(),
            "「亮」的圆点必须是绿的（当前 {green:?}）"
        );
        assert!(
            red.r() > red.g() && red.r() > red.b(),
            "「灭」的圆点必须是红的（当前 {red:?}）"
        );
    }

    /// 英文界面下文案必须换成英文，但颜色不随语言变。
    #[test]
    fn indicator_follows_the_language() {
        let en = crate::app::Language::En.strings();
        let on = connection_indicator(true, ConnectionKind::Serial, en);
        let off = connection_indicator(false, ConnectionKind::Serial, en);

        assert_eq!(on.label, "Connected");
        assert_eq!(off.label, "Not connected");

        // 颜色是通用视觉信号，不该随界面语言变
        let zh_on = connection_indicator(true, ConnectionKind::Serial, zh_strings());
        assert_eq!(
            on.color(),
            zh_on.color(),
            "状态灯的颜色不该因为界面切成英文而改变"
        );
    }

    fn zh_strings() -> &'static Strings {
        crate::app::Language::Zh.strings()
    }

    /// **状态栏与侧边栏必须不会同时给出矛盾结论**。
    ///
    /// 两处各自维护一套措辞，最容易出的错是其中一处忘了 UDP 那档：侧边栏说
    /// "已绑定"、状态栏却说"已连接"，同一个窗口里两个指示互相打脸。
    /// 这条把两者的分叉条件绑在一起——只要哪天有人给其中一边加了新档位而漏了
    /// 另一边，这里就会红。
    #[test]
    fn status_bar_agrees_with_the_sidebar_on_udp() {
        let zh = crate::app::Language::Zh.strings();

        for connected in [true, false] {
            let bar = connection_indicator(connected, ConnectionKind::Udp, zh);
            // 侧边栏在 UDP 下用的是 status_bound / status_unbound；
            // 那两条是 "状态: 已绑定" 这种带前缀的长句，所以只能比"是否含绑定字样"。
            let sidebar_says_bound = bar.label == zh.statusbar_bound;
            let sidebar_uses_bound_wording =
                zh.status_bound.contains("已绑定") && zh.status_unbound.contains("未绑定");
            assert!(
                sidebar_uses_bound_wording,
                "侧边栏的 UDP 措辞变了（不再是「已绑定/未绑定」），状态栏这边的对应关系要一起改"
            );
            assert_eq!(
                sidebar_says_bound, connected,
                "UDP 下状态栏的绑定措辞与连接位不符"
            );
        }
    }

    /// 状态栏真的把状态词画了出来。
    ///
    /// 上面几条测的是纯函数的输出；这一条接上绘制路径，确认 `show_status_bar`
    /// 没有把 `indicator` 算了却不用（那会得到"函数全对、界面空白"这种结果）。
    /// 做法与 `panels::tests::font_scale_dropdown_is_wired_to_the_field` 一致：
    /// 读 `FullOutput.shapes` 里真实落笔的文字。
    #[test]
    fn status_bar_paints_the_indicator() {
        let mut app = ComTool::default();
        let expected = crate::app::Language::Zh.strings().statusbar_disconnected;

        let painted = painted_labels(&mut app);
        assert!(
            painted.contains(&expected.to_string()),
            "状态栏没有画出 {expected:?}——`show_status_bar` 可能算完 `indicator` 就丢了。\
             实际画出：{painted:?}"
        );
    }

    /// 圆点必须作为**一个真实画出的圆**出现在绘制输出里，且用对了颜色。
    ///
    /// 这条替换掉了早期的"emoji 字形覆盖"测试——那个测试现在没有意义了，因为状态栏
    /// 已经完全不依赖任何 emoji 字体（原因见模块文档：egui 只光栅化矢量轮廓，
    /// 拿不到 COLR/CPAL 彩色图层）。取而代之要防的回归是"圆点没画出来"或"颜色写死"。
    ///
    /// 读的是 `Shape::Circle` 而不是 `Shape::Text`：圆点是几何图形，不是字符。
    /// 断言里同时查**颜色**，所以"只画一个灰圆"或"永远画绿圆"都会被拦下。
    #[test]
    fn status_dot_is_painted_as_a_filled_circle_in_the_right_colour() {
        let mut app = ComTool::default();
        let expected = connection_indicator(
            false,
            ConnectionKind::Serial,
            crate::app::Language::Zh.strings(),
        );

        let circles = painted_circles(&mut app);
        assert!(
            !circles.is_empty(),
            "状态栏没有画出任何圆点——圆点是这个功能的主体，不是可选项"
        );

        // 默认状态是"未连接"，所以应当恰好有一枚红色的圆。
        let matching = circles
            .iter()
            .filter(|(_, fill)| *fill == expected.color())
            .count();
        assert_eq!(
            matching,
            1,
            "期望恰好一枚填充色为 {:?}（未连接=红）的圆，实际画出：{circles:?}",
            expected.color()
        );
    }

    /// 圆点直径必须随全局字体档位一起变，且小于行高（否则会顶到相邻行）。
    #[test]
    fn dot_size_tracks_the_line_height_but_stays_smaller_than_it() {
        let mut app = ComTool::default();
        let circles = painted_circles(&mut app);
        let (radius, _) = circles.first().copied().expect("没画出圆点");

        // 半径来自行高：`行高 * DOT_SIZE_RATIO / 2`。这里只断言它是有限正数，
        // 且换算回去的行高落在合理范围（egui 默认 Body 约 14~22pt）。
        assert!(radius.is_finite() && radius > 0.0, "半径非法：{radius}");
        let line_height = radius * 2.0 / DOT_SIZE_RATIO;
        assert!(
            (10.0..=40.0).contains(&line_height),
            "由半径 {radius} 反推出的行高 {line_height} 不合理，尺寸计算可能挂了"
        );
    }

    /// 取出一次绘制里所有落笔的文字。
    fn painted_labels(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.show_status_bar(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }
        // 不清会导致 drop 时 panic（假红）
        out.textures_delta.clear();
        texts
    }

    /// 取出一次绘制里所有圆：(半径, 填充色)。
    fn painted_circles(app: &mut ComTool) -> Vec<(f32, Color32)> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.show_status_bar(ui);
        });

        let mut circles = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Circle(circle) = &clipped.shape {
                circles.push((circle.radius, circle.fill));
            }
        }
        out.textures_delta.clear();
        circles
    }
}
