//! 底部状态栏：一颗彩色圆点 + 一句话常驻显示连接状态，紧跟收发字节数；
//! 整组内容在栏内**水平居中、垂直居中**。
//!
//! **为什么单独一个模块**：状态栏的判据（哪颗灯、哪句话）是一个纯函数
//! （[`connection_indicator`]），与绘制分开。这样"UDP 该显示已绑定而不是已连接"
//! 这类规则可以直接断言，不必先画一遍再去看像素——见 `mod tests`。
//!
//! 词义与侧边栏的 [`crate::sidebar`] 那套**必须一致**：同一个窗口里两处状态指示
//! 给出不同结论，比只有一处更容易让人误判。所以这里同样按四态走
//! （串口/TCP 的已连接/未连接 + UDP 的已绑定/未绑定）。
//!
//! # 收发字节数为什么在这里
//!
//! 「接收统计」原先挂在右侧边栏，但它是一**读数**不是**控件**：用户扫一眼就知道
//! 收了多少、发了多少，不该占侧边栏一块竖向空间、也不该需要展开折叠区才看得见。
//! 底部状态栏本来就是"常驻读数"该待的地方，所以把**两个字节计数**挪了过来
//! （与连接状态同处一排，整组居中显示）。
//!
//! 口径不同的两行**不在这里**：「显示模式」「时间戳前缀」描述的是当前设置，
//! 不是累计量，随「串口信息」一起展示更自然——见 `sidebar.rs` 的 `serial_info` 区。
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
use crate::strings::{Strings, fill};
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

/// 状态栏内容四周的**垂直**留白（上下各一份），单位是 egui 点。
///
/// 为什么要留：内容贴着面板上下边缘时，文字视觉上会与上方中央区的分隔线、
/// 以及窗口底边"粘"在一起，看着像被挤在缝里。留出这段空白后，整条内容在栏内
/// 垂直居中，上下都透气，视觉上就与两条边框脱开了。
///
/// 取 10 而不是更大：太小（<6）留白看不出来、等于没做；太大（>16）状态栏会显得
/// 空旷、抢中央区的纵向空间。10 点约等于三分之二行高，在四档字体下都还合身。
const BAR_PADDING_Y: i8 = 10;

/// 状态栏内容左右的留白。
///
/// 左右各留一点，是为了让居中的内容与窗口左右边缘保持同等距离；取小值即可，
/// 因为内容本身已在栏内水平居中，不会贴边。
const BAR_PADDING_X: i8 = 12;

// 垂直留白必须为正：为 0 时"加高"这件事就落空了，且不会有任何编译期报错。
const _: () = assert!(BAR_PADDING_Y > 0, "状态栏垂直留白必须为正，否则加高无效");

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
    /// 圆点右边的文字（已连接 / 未连接 / 已绑定 / 未绑定 / 正在打开）。
    pub label: &'static str,
    /// 「已经发起打开、通道还没就绪」。
    ///
    /// 单独一个字段而不是把颜色写死成第三种：**亮度**与**文案**是两个维度——
    /// "正在打开"的灯在语义上是"还没亮"，而它的文案又与"未连接"完全不同。
    /// 把两者挤进一个 `label` 字段的写法，会让"恰好三种颜色"变成隐含约定，
    /// 多一档就崩。
    pub is_pending: bool,
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
        } else if self.is_pending {
            // 「正在打开」用琥珀色：既不是"通"（绿）也不是"不通"（红）。用红会让
            // 用户以为已经失败；用绿等于谎报已经连上（这正是要修的缺陷）。琥珀是
            // 交通灯里"准备/等待"的那一档，语义上最贴切。
            Color32::from_rgb(0xF5, 0xA6, 0x23)
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
///
/// **`opening` 这一档只有串口会有**（判定见 `ComTool::opening`）。它必须与
/// "未连接"分开显示：串口打开失败时，界面上先出现的是"正在打开"、随后才是
/// "未连接"+ 一条失败说明；两者合并会让用户以为"刚才根本没试过"。
pub(crate) fn connection_indicator(
    connected: bool,
    opening: bool,
    kind: ConnectionKind,
    s: &Strings,
) -> ConnectionIndicator {
    if connected && opening {
        // 与侧边栏的 `status_opening` 同源（那个带"状态:"前缀，这个是短词）。
        return ConnectionIndicator {
            is_on: false,
            label: s.statusbar_opening,
            is_pending: true,
        };
    }
    match (connected, kind) {
        (true, ConnectionKind::Udp) => ConnectionIndicator {
            is_on: true,
            label: s.statusbar_bound,
            is_pending: false,
        },
        (true, _) => ConnectionIndicator {
            is_on: true,
            label: s.statusbar_connected,
            is_pending: false,
        },
        (false, ConnectionKind::Udp) => ConnectionIndicator {
            is_on: false,
            label: s.statusbar_unbound,
            is_pending: false,
        },
        (false, _) => ConnectionIndicator {
            is_on: false,
            label: s.statusbar_disconnected,
            is_pending: false,
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
    ///
    /// # 版式：整行水平居中、栏内垂直居中
    ///
    /// 内容是"圆点 + 连接状态 + 收发字节数"三件，作为**一整组**在栏内**水平居中**，
    /// 而不是左状态右读数各贴一边——后者在宽窗口下会把两件信息甩到两个极端，
    /// 中间空出一大片，看起来像没画完。三件之间用分隔符隔开、整组居中，读起来更紧凑。
    ///
    /// 栏内**垂直**居中靠 [`BAR_PADDING_Y`] 的上下等量留白实现；同时关掉面板自带的
    /// 分隔线（`show_separator_line(false)`）——那条线是"边框感"的主要来源之一，
    /// 关掉后内容与上方中央区之间只靠留白分隔，视觉上就与边框脱开了。窗口底边那条
    /// 外框仍由操作系统/egui 保留，无法也不应由这里去掉。
    pub(crate) fn show_status_bar(&mut self, ui: &mut Ui) {
        let s = self.language.strings();
        let indicator =
            connection_indicator(self.connected, self.opening(), self.connection_kind, s);

        // 本帧要显示的读数快照。与侧边栏一样，在绘制前统一取值，避免同一行里
        // 几个数字来自不同时刻（虽然这里读的都是即时值，但把口径写在一处更好维护）。
        //
        // 两个计数的口径**不一样**，悬停里点明（N4）：接收那个是"当前留在缓冲区
        // 里的字节数"（会被上限裁掉），发送那个是"自本次连接以来累计送出去的
        // 字节数"（只增不减）。不写清楚的话，用户会拿两个数字相加去对账。
        let rx_bytes = self.receive_text.len();
        let tx_bytes = self.tx_bytes;

        egui::Panel::bottom("status_bar")
            .resizable(false)
            // 关掉面板自带的顶部分隔线：它是"贴着上面那条边框"的观感来源之一。
            // 去掉后，栏内内容与中央区之间只剩留白，视觉上更"飘"、更像一条独立信息带。
            .show_separator_line(false)
            // 背景用**标准的面板底色**，只把内边距换成我们自己的（纵向留白把状态栏"垫高"、
            // 让内容在栏内垂直居中；横向留白让居中内容与窗口左右边缘等距）。
            //
            // ⚠️ **这里原先写的是 `Frame::NONE`，那是个真缺陷**（2026-09-24 用户目视发现
            // "浅色主题下状态栏没变色"）：`Frame::NONE` 的 `fill` 是 `Color32::TRANSPARENT`，
            // 于是状态栏**自己不画背景**，透出来的是 `eframe::App::clear_color` 的默认值——
            // 一个**写死的**深灰 `rgba(12,12,12,180)`，它的参数名就叫 `_visuals`，
            // **完全不看主题**（`eframe-0.36.1/src/epi.rs`）。深色主题下 (12,12,12) 与
            // 面板底色 (27,27,27) 几乎一样，所以一直没暴露；浅色主题下面板底色是
            // (248,248,248)，这条带子却还是 (12,12,12)，而文字色同时变成 gray(80)
            // ——深字压在近黑底上，整条状态栏看起来"没内容、也没变色"。
            //
            // 教训：**面板背景是自己的事，不要指望 clear_color**。用
            // `Frame::side_top_panel` 而不是只补一个 `.fill(...)`：它正是 egui 面板
            // 不给 frame 时会用的那一份（`Panel::resolve_frame`），底色的唯一来源在
            // egui 那边，将来它改了我们也跟着改。
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(BAR_PADDING_X, BAR_PADDING_Y)),
            )
            .show(ui, |ui| {
                // 整组内容水平居中。
                //
                // **为什么不用 egui 的声明式居中**（实测踩过四次）：`with_layout`
                // 配 `with_main_align(Center)`、`vertical_centered`、
                // `allocate_ui_with_layout` 配 `top_down(Center)`、`horizontal_centered`
                // ——这四种写法在无头渲染下产出的**文字落笔位置全在 x=0**（贴左）。
                // 原因：这些 API 只重排"区域矩形"，而形状一旦 emit 就不会回移；
                // 且整行默认撑满可用宽度，"居中一条满宽的行"本就没有可居之处。
                // 与其依赖这层不确定的行为，不如**把居中算清楚**：
                // 先量出这一行实际要多宽，再用 `add_space` 补左侧偏移。确定性、
                // 可断言、与 egui 内部实现解耦。
                let avail = ui.available_width();
                let row_width = self.status_bar_row_width(ui, indicator, rx_bytes, tx_bytes);
                let pad = ((avail - row_width) / 2.0).max(0.0);

                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    self.status_bar_row(ui, indicator, rx_bytes, tx_bytes);
                });
            });
    }

    /// 按当前内容渲染状态栏那一行（圆点 + 状态词 + 分隔符 + 两个字节读数）。
    ///
    /// 抽成方法是因为它要**被画两遍**：一次用于量宽度（[`Self::status_bar_row_width`]），
    /// 一次用于真正落笔。两遍必须画出**完全相同**的内容，否则量出的宽度与实画不符、
    /// 居中就会偏——所以内容只能在这一个地方定义。
    fn status_bar_row(
        &self,
        ui: &mut Ui,
        indicator: ConnectionIndicator,
        rx_bytes: usize,
        tx_bytes: u64,
    ) {
        let s = self.language.strings();
        draw_status_dot(ui, indicator);
        ui.label(indicator.label);

        // 读数与状态之间加一条竖直分隔，避免"未连接 收: 0 字节"
        // 连成一句读过去像在说"未连接收了 0 字节"的因果。
        ui.separator();
        ui.label(fill(s.rx_bytes_label, &[&rx_bytes]))
            .on_hover_text(s.rx_bytes_hover);
        ui.label(fill(s.tx_bytes_label, &[&tx_bytes]))
            .on_hover_text(s.tx_bytes_hover);

        // 待发队列读数（2026-09-27，`HANDOVER.md` §5「写路径瞬态错误静默重试」）。
        //
        // **只在真有积压时出现**：0 的时候显示"待发 0 字节"只是噪音（绝大多数时间
        // 都是 0，状态栏宽度本来就紧）；而它要回答的问题恰恰是"有东西没出去"。
        //
        // **停滞时换警示色**：只给一个数字，用户得盯着它才知道没动；颜色一变就是
        // "现在不对劲"。判据与配色都由 `ComTool` 给，这里只负责画。
        if self.pending_bytes > 0 {
            let text = fill(s.pending_bytes_label, &[&self.pending_bytes]);
            let mut rich = egui::RichText::new(text);
            if self.pending_is_stalled() {
                rich = rich.color(ui.visuals().warn_fg_color);
            }
            ui.label(rich).on_hover_text(s.pending_bytes_hover);
        }
    }

    /// 量出状态栏那一行需要多宽（用于水平居中）。
    ///
    /// 做法：把同一行画进一个**不可见**的探测 `Ui`（`UiBuilder::invisible(true)`），
    /// 读它的 `min_rect.width()`。
    ///
    /// **为什么必须 `invisible`**：探测这一遍也会走完整的布局与绘制流程。若让它
    /// 正常可见，就会**真的往输出里再落一遍**圆点与文字——界面上是重影、测试里
    /// 则表现为"画出了两个圆点""这组文字位置还在 x=0"，把居中判定彻底带偏
    /// （实测踩过：圆点数从 1 变 2、中心从 500 偏回 136）。`invisible` 只抑制
    /// **绘制**，布局照常进行，`min_rect` 因此仍然准确。
    ///
    /// 为什么不硬编码一个宽度：内容随语言变（"未连接"↔"Not connected"）、
    /// 随数值变（9 位字节数 vs 0）、还随全局字体档位变——写死必然在某些组合下偏。
    fn status_bar_row_width(
        &self,
        ui: &mut Ui,
        indicator: ConnectionIndicator,
        rx_bytes: usize,
        tx_bytes: u64,
    ) -> f32 {
        // 探测 Ui 要拿到**足够宽**的 max_rect（否则行会换行、量到的宽度偏小），
        // 且必须用**左到右**布局——否则每个控件各占一行，量出来的是"最宽那个控件"
        // 而不是整行之和。它 `invisible`，只走布局不落笔，不会在界面上留痕。
        let mut probe = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width().max(1.0), 0.0),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .invisible(),
        );
        self.status_bar_row(&mut probe, indicator, rx_bytes, tx_bytes);
        probe.min_rect().width()
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

    /// 五态各自显示什么，逐条钉住。
    ///
    /// 断言的是**标签文字**（来自文案表）而不是"画出了什么"——文案表本身就带着
    /// 中英两列，所以这一条顺带覆盖了英文界面。
    #[test]
    fn indicator_covers_all_five_states() {
        let zh = crate::app::Language::Zh.strings();

        // 串口 / TCP：连上就是"已连接"，断开就是"未连接"
        let on = connection_indicator(true, false, ConnectionKind::Serial, zh);
        assert_eq!(on.label, zh.statusbar_connected);
        assert!(on.is_on, "连接成功时灯必须是亮的");

        let off = connection_indicator(false, false, ConnectionKind::Serial, zh);
        assert_eq!(off.label, zh.statusbar_disconnected);
        assert!(!off.is_on, "断开时灯必须是灭的");

        // UDP 单独一档：绑定成功≠对端在线，措辞必须避开"已连接"
        let udp_on = connection_indicator(true, false, ConnectionKind::Udp, zh);
        assert_eq!(
            udp_on.label, zh.statusbar_bound,
            "UDP 绑定成功不能显示成「已连接」——那会让用户以为对端在线、掉线会被发现"
        );
        assert_ne!(
            udp_on.label, zh.statusbar_connected,
            "UDP 那一档必须与串口/TCP 的措辞不同，否则这个分叉等于没做"
        );

        let udp_off = connection_indicator(false, false, ConnectionKind::Udp, zh);
        assert_eq!(udp_off.label, zh.statusbar_unbound);
        assert_ne!(udp_off.label, zh.statusbar_disconnected);

        // 第三档「正在打开」：`connected` 已经是真、通道还没就绪。
        let pending = connection_indicator(true, true, ConnectionKind::Serial, zh);
        assert_eq!(pending.label, zh.statusbar_opening);
        assert!(
            !pending.is_on,
            "「正在打开」不能把灯点亮——点亮就是在说「已经连上」，而那正是要修的假阳性"
        );
        assert_ne!(
            pending.label, zh.statusbar_connected,
            "「正在打开」必须与「已连接」措辞不同，否则用户仍然以为已经连上、点了发送才发现发不出去"
        );
        assert_ne!(
            pending.label, zh.statusbar_disconnected,
            "「正在打开」不能与「未连接」同词：串口打开失败时界面会先出现「正在打开」、\
             随后才是「未连接」+ 失败说明；合并成一档会让用户以为刚才根本没试过"
        );
    }

    /// 灯的颜色必须跟着状态走，而不是写死一种。
    ///
    /// 这是最容易被写错的地方：把 `color()` 固定成绿色，其余全对——文字对、
    /// 开关对，只有灯永远绿着。所以这条**单独**断言三个状态的颜色互不相同，
    /// 并钉住方向（不是随便三种颜色都算过）。
    #[test]
    fn the_dot_changes_colour_with_the_state() {
        let zh = crate::app::Language::Zh.strings();
        let on = connection_indicator(true, false, ConnectionKind::Serial, zh);
        let off = connection_indicator(false, false, ConnectionKind::Serial, zh);
        let pending = connection_indicator(true, true, ConnectionKind::Serial, zh);

        assert_ne!(
            on.color(),
            off.color(),
            "连接与断开必须用**不同**的颜色，否则灯是个装饰、不指示任何东西"
        );
        assert_ne!(
            pending.color(),
            on.color(),
            "「正在打开」必须与「已连接」用不同颜色——同色等于谎报已经连上"
        );
        assert_ne!(
            pending.color(),
            off.color(),
            "「正在打开」必须与「未连接」用不同颜色——同色会让用户以为已经失败"
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
        // 琥珀：红与绿都高、蓝低。这样"顺手复用红"或"顺手复用绿"都会被拦下。
        let amber = pending.color();
        assert!(
            amber.r() > amber.b() && amber.g() > amber.b() && amber.r() != amber.g(),
            "「正在打开」的圆点必须是琥珀色（红绿都高、蓝低，且红绿不等），当前 {amber:?}"
        );
    }

    /// 英文界面下文案必须换成英文，但颜色不随语言变。
    #[test]
    fn indicator_follows_the_language() {
        let en = crate::app::Language::En.strings();
        let on = connection_indicator(true, false, ConnectionKind::Serial, en);
        let off = connection_indicator(false, false, ConnectionKind::Serial, en);
        let pending = connection_indicator(true, true, ConnectionKind::Serial, en);

        assert_eq!(on.label, "Connected");
        assert_eq!(off.label, "Not connected");
        assert_eq!(pending.label, "Opening");

        // 颜色是通用视觉信号，不该随界面语言变
        let zh_on = connection_indicator(true, false, ConnectionKind::Serial, zh_strings());
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
            let bar = connection_indicator(connected, false, ConnectionKind::Udp, zh);
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

    /// **「正在打开」这一档必须在侧边栏与状态栏同时存在且同源**。
    ///
    /// 与上面那条 UDP 的守卫是同一个套路，防的也是同一类错：只给一边加档位、
    /// 另一边照旧说"已连接"。判定用**两侧都必须有独立文案**这条硬事实：
    /// 状态栏那个是短词、侧边栏那个带"状态:"前缀，两者不可能共用同一条。
    ///
    /// 之所以单独一条而不是并进 UDP 那条：`opening` 只与串口有关，而 UDP 那条
    /// 的循环只跑 UDP，两者覆盖的场景不相交。
    #[test]
    fn status_bar_agrees_with_the_sidebar_on_opening() {
        let zh = crate::app::Language::Zh.strings();

        let bar = connection_indicator(true, true, ConnectionKind::Serial, zh);
        assert_eq!(bar.label, zh.statusbar_opening);

        assert_ne!(
            zh.statusbar_opening, zh.status_opening,
            "状态栏用短词、侧边栏带「状态:」前缀——两条现在一样了，说明有人把其中一处\
             改成了另一处的值，两套措辞的分工就没了"
        );
        assert!(
            zh.status_opening.contains("正在打开") && zh.statusbar_opening.contains("正在打开"),
            "两侧的「正在打开」措辞必须都含这四个字，否则两处指示的语义会漂成两种说法"
        );
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

    /// **接收/发送字节数必须在状态栏里画出来，且是本帧的真实值。**
    ///
    /// 这条钉住本次重构（把「接收统计」从侧边栏搬到底部状态栏）的**落点**：
    /// 光看"状态栏能画"是不够的——读写数是新加的，必须确认它既画了、又用的是
    /// `receive_text.len()` / `tx_bytes` 而不是写死的 0。
    ///
    /// 断言用 `fill()` 拼出**期望的完整字符串**再比对，因此"画了标签但值恒为 0"
    /// 或"两个数字搞反"都会被拦下——只断言"包含 `接收字节数`"是空转的（标签
    /// 本来就会画出来）。
    #[test]
    fn status_bar_paints_live_byte_counters() {
        let mut app = ComTool::default();
        // 造两个**互不相同**的值：若实现把两个标签接反了，断言会红。
        app.receive_text.push_str("hello"); // 5 字节
        app.tx_bytes = 4096;

        let s = crate::app::Language::Zh.strings();
        let want_rx = crate::strings::fill(s.rx_bytes_label, &[&app.receive_text.len()]);
        let want_tx = crate::strings::fill(s.tx_bytes_label, &[&app.tx_bytes]);

        let painted = painted_labels(&mut app);
        assert!(
            painted.contains(&want_rx),
            "状态栏没画出接收字节数 {want_rx:?}（实际：{painted:?}）——\
             重构后这一读数应当常驻状态栏"
        );
        assert!(
            painted.contains(&want_tx),
            "状态栏没画出发送字节数 {want_tx:?}（实际：{painted:?}）"
        );
    }

    /// 两个读数必须**随数据变化**，不能是写死的常量。
    ///
    /// 上一条用默认 0 也可能碰巧通过（如果标签里本来就有个 0）；这条换两个不同的
    /// 值各画一次，要求两次画出的文字**不相同**——写死常量的实现必然失败。
    #[test]
    fn status_bar_byte_counters_change_with_the_data() {
        let s = crate::app::Language::Zh.strings();

        let mut a = ComTool::default();
        a.receive_text.push_str("aaaa");
        a.tx_bytes = 1;

        let mut b = ComTool::default();
        b.receive_text.push_str("bbbbbbbbbb");
        b.tx_bytes = 999_999;

        let pa = painted_labels(&mut a);
        let pb = painted_labels(&mut b);

        let want_a = crate::strings::fill(s.rx_bytes_label, &[&4usize]);
        let want_b = crate::strings::fill(s.rx_bytes_label, &[&10usize]);
        assert!(
            pa.contains(&want_a),
            "第一帧没画出 {want_a:?}（实际：{pa:?}）"
        );
        assert!(
            pb.contains(&want_b),
            "第二帧没画出 {want_b:?}（实际：{pb:?}）"
        );
        assert_ne!(want_a, want_b, "两次的期望值就不该相同——测试数据没造好");
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

    /// 取出一次绘制里所有落笔的文字**及其包围盒**：(文本, 左上角, 尺寸)。
    ///
    /// 与 [`painted_labels`] 的区别：那个只要文本内容，这个要几何位置，
    /// 用来断言"居中"这类**版面**性质——文字内容对但整组贴在左边，内容断言照样过。
    fn painted_label_boxes(
        app: &mut ComTool,
        screen: egui::Rect,
    ) -> Vec<(String, egui::Pos2, egui::Vec2)> {
        let ctx = egui::Context::default();
        // 屏幕尺寸要通过 `RawInput.screen_rect` 给——`run_ui` 的第一个参数是输入，
        // 不是矩形。不给的话容器尺寸是默认值，"居中"就无从判定。
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.show_status_bar(ui);
        });

        let mut boxes = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(t) = &clipped.shape {
                boxes.push((t.galley.text().to_owned(), t.pos, t.galley.size()));
            }
        }
        out.textures_delta.clear();
        boxes
    }

    /// **状态栏必须比一行文字高**——这是"加高"的判据。
    ///
    /// 关键在于：不能断言"面板存在"或"文字画出来了"（那些与高度无关，把留白删掉
    /// 也照样过）。要断言的是**内容簇的垂直范围之外还有多少空隙**。做法是给一个固定
    /// 高度的屏幕，把状态栏画在底部，然后看文字簇的**底边**离屏幕底边有多远——
    /// 这个距离就等于下留白（`BAR_PADDING_Y`），留白删掉它会塌到 0。
    #[test]
    fn status_bar_is_taller_than_one_line_of_text() {
        let mut app = ComTool::default();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
        let boxes = painted_label_boxes(&mut app, screen);

        let bottom_of_text = boxes
            .iter()
            .map(|(_, pos, size)| pos.y + size.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(bottom_of_text.is_finite(), "状态栏没画出任何文字");

        // 文字底边到屏幕底边的空隙 = 下留白（面板贴着窗口底边绘制）。
        let gap_below = screen.max.y - bottom_of_text;
        assert!(
            gap_below >= f32::from(BAR_PADDING_Y),
            "状态栏下留白只有 {gap_below:.1}，应当至少有 BAR_PADDING_Y={BAR_PADDING_Y}。\
             若把 Frame 的 inner_margin 去掉，这里会塌到 0——本测试就是钉住它别被删。"
        );
    }

    /// **整组内容必须水平居中**——这是"居中"的判据。
    ///
    /// 判据是**位置**而不是内容：断言"文字里有收/发字节数"在右对齐实现下同样成立
    /// （旧版就是那样），所以那种断言与该改动无关。这里量的是**整组文字的几何中心**
    /// 与容器中心之差，只有真的改成居中才会落在这条线附近；右对齐会让它明显偏右。
    #[test]
    fn status_bar_content_is_horizontally_centred() {
        let mut app = ComTool::default();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
        let boxes = painted_label_boxes(&mut app, screen);

        let left = boxes
            .iter()
            .map(|(_, pos, _)| pos.x)
            .fold(f32::INFINITY, f32::min);
        let right = boxes
            .iter()
            .map(|(_, pos, size)| pos.x + size.x)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            left.is_finite() && right.is_finite(),
            "状态栏没画出任何文字"
        );

        let group_centre = (left + right) / 2.0;
        let screen_centre = screen.center().x;
        let offset = (group_centre - screen_centre).abs();

        // 容差给 12 点，理由有两条：
        //
        // ① **测的是文字不是整组**。本测试只取 `Shape::Text` 的包围盒，而这一行
        //    最左边是个**圆点**（几何图形，不是文字）。圆点在自己的矩形里居中，
        //    其矩形比圆本身宽，于是"文字的几何中心"天然比"含圆点的整组中心"
        //    偏右几个点——这是测量口径造成的固定偏差，不是居中没做对。
        // ② `ui.separator()` 在两端占位不完全对称，还有一两点的零头。
        //
        // 两者相加实测约 9.4 点。而**错误实现**（贴左 / 贴右 / 右对齐）的偏移
        // 都是**数百点**（半个容器宽），12 点这条线把它们全部拦下绰绰有余。
        // 关键：容差必须**远小于**错误实现会产生的偏移——这正是它有效的前提。
        assert!(
            offset <= 12.0,
            "状态栏整组内容中心 x={group_centre:.1}，容器中心 x={screen_centre:.1}，\
             偏移 {offset:.1} 超过容差 12——看起来没有水平居中（右对齐/左对齐会明显偏离）"
        );
    }

    /// 取出一次绘制里所有矩形填充：(矩形, 填充色)。
    fn painted_rects(
        app: &mut ComTool,
        theme: egui::ThemePreference,
    ) -> Vec<(egui::Rect, Color32)> {
        let ctx = egui::Context::default();
        // 主题要先在 context 上设好：`show_status_bar` 本身不负责下发主题（那是
        // `ComTool::ui` 里 `apply_theme` 的活），所以测试得自己把 context 摆成目标主题。
        ctx.set_theme(theme);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.show_status_bar(ui);
        });

        let mut rects = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Rect(rect) = &clipped.shape {
                rects.push((rect.rect, rect.fill));
            }
        }
        out.textures_delta.clear();
        rects
    }

    /// 取出状态栏**面板自身**那块背景的填充色。
    ///
    /// 判据落在"面板背景"这个矩形上：状态栏是贴底的整幅面板，它的背景矩形一定横跨
    /// 整个宽度、且下边缘就是屏幕底边。这两条同时满足的矩形只可能有一个。
    fn status_bar_background_fill(theme: egui::ThemePreference) -> Color32 {
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
        let mut app = ComTool::default();
        let rects = painted_rects(&mut app, theme);

        let matches: Vec<_> = rects
            .into_iter()
            .filter(|(rect, _)| {
                rect.width() >= screen.width() - 1.0 && rect.max.y >= screen.max.y - 1.0
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "期望恰好一个「横跨整幅宽度、贴住屏幕底边」的矩形（状态栏面板背景），实际 {matches:?}"
        );
        matches[0].1
    }

    /// **状态栏的底色必须跟着主题走。**
    ///
    /// 用户目视发现的缺陷：切到浅色主题后，整条状态栏**没变色**，仍是深色带子。
    ///
    /// 根因不在配色表，而在**状态栏原先根本没画背景**：面板用的是 `Frame::NONE`，
    /// 它的 `fill` 是 `Color32::TRANSPARENT`，于是透出来的是
    /// `eframe::App::clear_color` 的默认值——一个**写死的**深灰
    /// `rgba(12,12,12,180)`，参数名就叫 `_visuals`，**完全不看主题**
    /// （`eframe-0.36.1/src/epi.rs`）。深色主题下 (12,12,12) 与面板底色 (27,27,27)
    /// 几乎一样，所以这个缺陷一直没暴露；浅色主题下面板底色变成 (248,248,248)，
    /// 而这条带子还是 (12,12,12)——同时浅色主题的文字色是 gray(80)，深字压在近黑的
    /// 底上，于是整条状态栏看起来像"没有内容、也没变色"。
    ///
    /// 断言直接比"底色 == 该主题的面板底色"：这既排除了透明（`TRANSPARENT` 不等于
    /// 任何一个面板底色），也排除了"换个写死的浅灰"——那样两套主题会得到同一个值。
    /// 同时它钉住了"与中央区连成一片"这个外观要求：状态栏不该是一条异色的横带。
    #[test]
    fn status_bar_background_follows_the_theme() {
        let light = status_bar_background_fill(egui::ThemePreference::Light);
        let dark = status_bar_background_fill(egui::ThemePreference::Dark);

        assert_ne!(
            light,
            Color32::TRANSPARENT,
            "浅色主题下状态栏没有画自己的背景（`Frame::NONE` 的 fill 是透明的），\
             透出来的会是 eframe 那个写死的深灰 clear_color，于是浅色下不变色"
        );
        assert_eq!(
            light,
            egui::Visuals::light().panel_fill,
            "状态栏底色必须是浅色主题的面板底色，否则会与中央区形成一条异色横带"
        );
        assert_eq!(
            dark,
            egui::Visuals::dark().panel_fill,
            "状态栏底色必须是深色主题的面板底色"
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
    /// 画出的**文字用的是什么颜色**。
    ///
    /// ⚠️ 必须**两处都看**，只看 `fallback_color` 会漏掉本用例要验的那一半（实测踩过）：
    /// * `ui.label(...)` 这类默认着色的文字，颜色落在 `TextShape::fallback_color` 上；
    /// * 而 `RichText::new(x).color(c)`（待发读数停滞时就是这个）走的是 galley 里
    ///   section 的 `color`，`fallback_color` **原样不变**——于是"换了色"在只读
    ///   `fallback_color` 的断言里看不出来（实测两者都是 `#8C8C8CFF`，测试假绿）。
    ///
    /// 取 section 色时遇到 `PLACEHOLDER` 才回退到 `fallback_color`，与 epaint 的
    /// 填充规则一致。
    fn painted_text_color(text_shape: &egui::epaint::TextShape) -> egui::Color32 {
        text_shape
            .galley
            .job
            .sections
            .first()
            .map(|section| section.format.color)
            .filter(|color| *color != egui::Color32::PLACEHOLDER)
            .unwrap_or(text_shape.fallback_color)
    }

    /// 画出状态栏，返回落笔的 `(文字, 颜色)` 与**当帧的警示色**。
    ///
    /// 警示色从**同一帧的 `ui.visuals()`** 取，而不是写死一个色值——写死等于把主题的
    /// 实现细节钉进断言，换个主题就假红，而且那也说明不了"警示色是什么"。
    fn painted_labels_with_warn(
        app: &mut ComTool,
    ) -> (Vec<(String, egui::Color32)>, egui::Color32) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 200.0),
            )),
            ..Default::default()
        };
        let mut warn = None;
        let mut out = ctx.run_ui(input, |ui| {
            warn = Some(ui.visuals().warn_fg_color);
            app.show_status_bar(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push((
                    text_shape.galley.text().to_owned(),
                    painted_text_color(text_shape),
                ));
            }
        }
        // 不清会在 drop 时 panic（假红）
        out.textures_delta.clear();
        (texts, warn.expect("绘制过程中应当拿得到 visuals"))
    }

    /// ⭐ **状态栏真的把待发读数画了出来，并在停滞时换色**（2026-09-27）。
    ///
    /// 走的是**真实链路**：替身通道摆出积压 → `poll_pending_bytes()` → `show_status_bar()`。
    /// 三条判据各对应一件用户能感知的事：
    /// ① 没积压时**不出现**——常驻一个"待发 0 字节"只是噪音，且它要回答的问题恰恰是
    ///    "有东西没出去"；
    /// ② 有积压时出现，且**数字是本帧真实值**（不是画了标签却恒为 0）；
    /// ③ 停滞时**颜色与未停滞时不同**——只给数字不变色，用户得盯着它才看得出没动，
    ///    那等于没提示。
    ///
    /// 空转验证：把 `if self.pending_bytes > 0` 改成 `false` ⇒ ② 立刻红；把
    /// `pending_is_stalled()` 那一段去掉 ⇒ ③ 红；让 `poll_pending_bytes` 不读通道
    /// （恒 0）⇒ ② 红。
    #[test]
    fn status_bar_shows_pending_bytes_and_turns_warn_when_stalled() {
        use crate::communicationtrait::fake::FakeChannel;

        let zh = crate::app::Language::Zh.strings();
        let (channel, handle) = FakeChannel::new();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            ..ComTool::default()
        };
        app.open_connection();
        assert!(app.channel_is_ready(), "前提：替身通道应当已经就绪");

        // ① 队列是空的：那条读数不该出现
        let (labels, _) = painted_labels_with_warn(&mut app);
        let zero = crate::strings::fill(zh.pending_bytes_label, &[&0usize]);
        assert!(
            !labels.iter().any(|(t, _)| *t == zero),
            "队列是空的却画出了待发读数（那是常驻噪音）。实际：{labels:?}"
        );

        // ② 有积压：读数出现，而且用的是本帧真实值
        handle.set_pending(4096);
        app.poll_pending_bytes();
        let want = crate::strings::fill(zh.pending_bytes_label, &[&4096usize]);
        let (labels, warn) = painted_labels_with_warn(&mut app);
        let calm_color = labels
            .iter()
            .find(|(t, _)| *t == want)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("状态栏没画待发读数 {want:?}。实际：{labels:?}"));
        assert_ne!(
            calm_color, warn,
            "积压刚出现就被画成警示色了 —— 还没到「卡住」就不该报警"
        );

        // ③ 停滞：同一个读数换成警示色
        app.note_pending_bytes(
            4096,
            std::time::Instant::now() + crate::app::PENDING_STALL_AFTER,
        );
        let (labels, warn) = painted_labels_with_warn(&mut app);
        let stalled_color = labels
            .iter()
            .find(|(t, _)| *t == want)
            .map(|(_, c)| *c)
            .expect("停滞时读数应当还在，不该消失");
        assert_ne!(
            stalled_color, calm_color,
            "待发读数已停滞，颜色却没变 —— 用户看不出「卡住了」"
        );
        assert_eq!(stalled_color, warn, "停滞时应当用主题的警示色");
    }
}
