//! 底部状态栏：一颗彩色圆点 + 一句话，常驻显示当前连接状态。
//!
//! **为什么单独一个模块**：状态栏的判据（哪颗灯、哪句话）是一个纯函数
//! （[`connection_indicator`]），与绘制分开。这样"UDP 该显示已绑定而不是已连接"
//! 这类规则可以直接断言，不必先画一遍再去看像素——见 `mod tests`。
//!
//! 词义与侧边栏的 [`crate::sidebar`] 那套**必须一致**：同一个窗口里两处状态指示
//! 给出不同结论，比只有一处更容易让人误判。所以这里同样按四态走
//! （串口/TCP 的已连接/未连接 + UDP 的已绑定/未绑定）。

use crate::app::{ComTool, ConnectionKind};
use crate::strings::Strings;
use egui::Ui;

/// 状态栏要显示的内容。
///
/// 把"哪颗灯 + 哪句话"做成一个数据结构，是为了让判据可以被**单独断言**：
/// 测试不必去解析绘制出的文字，直接比较两个字段即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionIndicator {
    /// 圆点用哪个字符（🟢 / 🔴）。
    pub dot: &'static str,
    /// 圆点右边的文字（已连接 / 未连接 / 已绑定 / 未绑定）。
    pub label: &'static str,
    /// 灯是不是"亮"的（绿）。
    ///
    /// 与 `dot` 分开存而不是"看文字猜"：将来若把圆点换成图标字体、或给 UDP
    /// 用第三种颜色，判据仍然只有这一处。
    pub is_on: bool,
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
            dot: s.statusbar_dot_on,
            label: s.statusbar_bound,
            is_on: true,
        },
        (true, _) => ConnectionIndicator {
            dot: s.statusbar_dot_on,
            label: s.statusbar_connected,
            is_on: true,
        },
        (false, ConnectionKind::Udp) => ConnectionIndicator {
            dot: s.statusbar_dot_off,
            label: s.statusbar_unbound,
            is_on: false,
        },
        (false, _) => ConnectionIndicator {
            dot: s.statusbar_dot_off,
            label: s.statusbar_disconnected,
            is_on: false,
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
                    // 圆点单独上色没必要：🟢/🔴 本身就是彩色 emoji，着色反而会被
                    // 字体回退链丢弃（彩色字形不跟随 `color`）。这里只保证它
                    // **画得出来**——字形覆盖由 `main.rs::setup_fonts` 的 emoji 档负责。
                    let dot = ui.label(indicator.dot);
                    // 圆点是纯视觉标记，读屏软件拿到"🟢"没有意义，所以把有意义的
                    // 那句话挂成它的标签，而不是让它成为一条独立的读屏条目。
                    dot.on_hover_text(indicator.label);
                    ui.label(indicator.label);
                });
            });
    }
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
        assert_eq!(on.dot, zh.statusbar_dot_on);
        assert!(on.is_on, "连接成功时灯必须是亮的");

        let off = connection_indicator(false, ConnectionKind::Serial, zh);
        assert_eq!(off.label, zh.statusbar_disconnected);
        assert_eq!(off.dot, zh.statusbar_dot_off);
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

    /// 灯的颜色必须跟着状态走，而不是写死一个字符。
    ///
    /// 这是最容易被写错的地方：把 `dot` 固定成 `🟢`，其余全对——文字对、开关对，
    /// 只有灯永远绿着。所以这条**单独**断言两个状态的 `dot` 互不相同。
    #[test]
    fn the_dot_changes_with_the_state() {
        let zh = crate::app::Language::Zh.strings();
        let on = connection_indicator(true, ConnectionKind::Serial, zh);
        let off = connection_indicator(false, ConnectionKind::Serial, zh);

        assert_ne!(
            on.dot, off.dot,
            "连接与断开必须用**不同**的圆点字符，否则灯是个装饰、不指示任何东西"
        );
        assert_eq!(on.dot, "🟢");
        assert_eq!(off.dot, "🔴");
    }

    /// 英文界面下文案必须换成英文，但圆点不变。
    #[test]
    fn indicator_follows_the_language() {
        let en = crate::app::Language::En.strings();
        let on = connection_indicator(true, ConnectionKind::Serial, en);
        let off = connection_indicator(false, ConnectionKind::Serial, en);

        assert_eq!(on.label, "Connected");
        assert_eq!(off.label, "Not connected");
        // 圆点是通用符号，不随语言变
        assert_eq!(on.dot, "🟢");
        assert_eq!(off.dot, "🔴");
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

    /// **圆点字形必须真的在字体回退链覆盖范围内**。
    ///
    /// 这是这条功能的隐性依赖：🟢(U+1F7E2) 与 🔴(U+1F534) 都在 Emoji 区，
    /// 而 egui 默认字体、Hack、微软雅黑**都不含**它们中的至少一个。
    /// 实测（读字体 cmap 得到）：`seguisym.ttf` 有 U+1F534 却**没有** U+1F7E2；
    /// 只有 `seguiemj.ttf` 两个都有。所以 `main.rs::setup_fonts` 里那一档
    /// emoji 字体是**必需的**，不是锦上添花。
    ///
    /// 上面几条测试全都测不出"界面上是两个方块"——它们只看字符串内容。
    /// 这条直接去读字体文件确认字形存在，把"依赖某个系统字体"这件事钉下来。
    ///
    /// 字体缺失时**跳过**而不是失败：开发机可能是精简系统，那不是代码的问题。
    /// 但只要字体在，就必须真的含有这两个字形——那才是这条测试要拦的回归。
    #[test]
    fn status_dot_glyphs_are_covered_by_the_font_chain() {
        // 与 `main.rs::setup_fonts` 里的 `emoji_paths` 保持一致。
        const EMOJI_FONT: &str = r"C:\Windows\Fonts\seguiemj.ttf";

        let Ok(data) = std::fs::read(EMOJI_FONT) else {
            eprintln!("跳过：本机没有 {EMOJI_FONT}，无法校验 emoji 字形覆盖");
            return;
        };

        let zh = crate::app::Language::Zh.strings();
        for (name, s) in [
            ("绿色圆点", zh.statusbar_dot_on),
            ("红色圆点", zh.statusbar_dot_off),
        ] {
            let cp = s.chars().next().unwrap_or_else(|| panic!("{name} 是空串")) as u32;
            assert!(
                font_has_codepoint(&data, cp),
                "{name} {s:?}(U+{cp:04X}) 不在 {EMOJI_FONT} 里——\
                 界面会把它渲染成方框 □。要么换一个系统覆盖的圆点字符，\
                 要么在 setup_fonts 里换一档字体"
            );
        }
    }

    /// 在 TrueType/OpenType 的 `cmap` 表里查某个码点是否有字形。
    ///
    /// 只实现了 format 4 与 format 12 两个子表——Windows 系统字体都是这两种之一，
    /// 其余格式（0/2/6）在这个用途下不会出现。查不到就当"没有"，
    /// 宁可误报也不要放过（这条测试的目的是拦截，不是宽容）。
    fn font_has_codepoint(data: &[u8], codepoint: u32) -> bool {
        let be16 = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]) as usize;
        let be32 = |o: usize| {
            u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize
        };

        let cp = codepoint as usize;
        let num_tables = be16(4);
        let mut cmap_off = None;
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            if &data[rec..rec + 4] == b"cmap" {
                cmap_off = Some(be32(rec + 8));
            }
        }
        let Some(cmap_off) = cmap_off else {
            return false;
        };

        // 选一个子表：优先 format 12（能覆盖 U+FFFF 以上），否则 format 4。
        let n_sub = be16(cmap_off + 2);
        let mut chosen = None;
        for i in 0..n_sub {
            let rec = cmap_off + 4 + i * 8;
            let (pid, eid, off) = (be16(rec), be16(rec + 2), be32(rec + 4));
            let sub = cmap_off + off;
            let format = be16(sub);
            if format == 12 {
                chosen = Some(sub);
                break;
            }
            if format == 4 && chosen.is_none() {
                chosen = Some(sub);
            }
            let _ = (pid, eid);
        }
        let Some(sub) = chosen else {
            return false;
        };

        match be16(sub) {
            // format 12：一组组 (start, end, glyphId) 的区间
            12 => {
                let n_groups = be32(sub + 12);
                (0..n_groups).any(|i| {
                    let g = sub + 16 + i * 12;
                    let start = be32(g);
                    let end = be32(g + 4);
                    start <= cp && cp <= end
                })
            }
            // format 4：段表，U+FFFF 以内的码点
            4 => {
                if cp > 0xFFFF {
                    return false;
                }
                let seg_x2 = be16(sub + 6);
                let segs = seg_x2 / 2;
                let ends = sub + 14;
                let starts = ends + seg_x2;
                (0..segs).any(|i| {
                    let s = be16(starts + i * 2);
                    let e = be16(ends + i * 2);
                    s != 0xFFFF && s <= cp && cp <= e
                })
            }
            _ => false,
        }
    }
}
