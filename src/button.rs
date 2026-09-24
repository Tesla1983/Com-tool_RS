//! 全界面统一的按钮外观。
//!
//! 2026-09-24 用户要求「把 GUI 界面上的所有按钮都按照打开串口的样式重新写一遍」。
//! 「打开串口」那一版（`sidebar::draw_connect_button`）确立的样式语言是三条：
//!
//! 1. **自己画底**：`allocate_exact_size` 占一个位置 + `Painter` 填一个圆角实心
//!    矩形。不用 `egui::Button` 的默认外观——它的 `button_padding` 只有
//!    `vec2(4.0, 1.0)`，高度约等于一行字、宽度随文案长度、圆角 2、没有描边，
//!    看上去是一块"灰标签"而不是一个按钮；
//! 2. **尺寸显式给死**：高度由 [`ButtonSize`] 定，宽度贴合内容并夹在可用宽度内；
//! 3. **有悬停 / 按下反馈**：悬停亮一档、按下暗一档，方向与 egui 默认控件一致，
//!    免得和旁边的下拉框、单选钮手感打架。
//!
//! 「打开串口」自己的**状态点与右侧三角箭头不在这里**：那是"连接"这个动作的专属
//! 语义（按下去会建立 / 断开一条通道）。给普通按钮套上它反而是误导。
//!
//! # 配色分两档
//!
//! * [`ButtonTone::Primary`]＝**主操作**（发送 / 保存常用数据 / 开始记录），与
//!   「打开串口」同族的绿——它们说的都是"发起一件事"；
//! * [`ButtonTone::Neutral`]＝**普通操作**（刷新 / 清空 / 导出 / 选择目录 …），中性灰。
//!
//! **刻意不做红色档**：界面上一共有五处「清空 / 恢复默认」，全画成红色会在屏幕上
//! 同时出现五个红块，反而让真正该被看见的那一个（连接按钮的"断开"）不再突出。
//! 这是 2026-09-24 定稿时用户在"三档（另加破坏性红）"与"两档"之间选了后者的理由。
//!
//! # ⚠️ 宽度永远夹在 `ui.available_width()` 之内
//!
//! 这是用户这次点名的要求（"注意和其它组件的边距，不允许覆盖到其它无关的组件"）：
//! 宽度先按内容算，但**不许超过当前可用宽度**。`allocate_exact_size` 会把矩形计入
//! 布局（不是像裸 `Painter` 那样凭空画上去），所以只要不超宽，它就压不到同行的
//! 其它控件上。收窄到放不下文案时按比例缩字号，缩到 [`ButtonSize::text_min_size`]
//! 为止——**不截断**，截断后用户看不出这按钮是干什么的。

use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Response, Sense, Ui, vec2};

/// 按钮的圆角半径。
///
/// 与「打开串口」的 `CONNECT_BUTTON_CORNER` 同值。圆角是这套样式语言里最显眼的一处
/// 特征，两处一旦取不同值，界面上会立刻显出"两种按钮"。
const BUTTON_CORNER: u8 = 6;

/// 标准尺寸（[`ButtonSize::Normal`]）：独占一行时用。
const HEIGHT_NORMAL: f32 = 28.0;
const TEXT_SIZE_NORMAL: f32 = 15.0;
const TEXT_MIN_SIZE_NORMAL: f32 = 13.0;
const PADDING_X_NORMAL: f32 = 14.0;
const MIN_WIDTH_NORMAL: f32 = 72.0;

/// 紧凑尺寸（[`ButtonSize::Compact`]）：与下拉框 / 输入框挤在同一窄行时用。
///
/// 高度取 24 是为了**与旁边的下拉框齐平**（egui 的 `ComboBox` 按钮默认就是 24 上下）。
/// 若这里也走 28，那一行会被按钮顶高 4 点，下拉框上下多出一圈空隙。
const HEIGHT_COMPACT: f32 = 24.0;
const TEXT_SIZE_COMPACT: f32 = 14.0;
const TEXT_MIN_SIZE_COMPACT: f32 = 12.0;
const PADDING_X_COMPACT: f32 = 10.0;
const MIN_WIDTH_COMPACT: f32 = 56.0;

/// 按钮的**语义档**——决定底色。
///
/// 与 [`ButtonSize`] 正交：同一个档位既可以出现在标准尺寸上，也可以出现在紧凑尺寸上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonTone {
    /// 主操作：绿＝"按下去会发起一件事"。
    Primary,
    /// 普通操作：中性灰。界面上的大多数按钮都在这一档。
    Neutral,
}

/// 按钮的**尺寸档**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonSize {
    /// 高 28、字号 15：给独占一行的按钮。
    Normal,
    /// 高 24、字号 14：给与下拉框同行的窄位按钮（例如「刷新」）。
    Compact,
}

impl ButtonSize {
    /// 按钮高度（逻辑像素）。
    pub(crate) fn height(self) -> f32 {
        match self {
            ButtonSize::Normal => HEIGHT_NORMAL,
            ButtonSize::Compact => HEIGHT_COMPACT,
        }
    }

    /// 文案的基准字号。
    pub(crate) fn text_size(self) -> f32 {
        match self {
            ButtonSize::Normal => TEXT_SIZE_NORMAL,
            ButtonSize::Compact => TEXT_SIZE_COMPACT,
        }
    }

    /// 文案缩到的最小字号（德语 / 俄语那类长译文放不下时用）。
    ///
    /// 下限的取法是"**不低于正文**"：egui 0.36.1 的 `TextStyle::Body` 实测是 13.0，
    /// 所以标准档缩到狠处也就和正文齐平；紧凑档基准本来就只有 14，再往下压到 12
    /// 仍能读清。两条都不取更小的值——按钮上的字缩到看不清，比换行更糟。
    pub(crate) fn text_min_size(self) -> f32 {
        match self {
            ButtonSize::Normal => TEXT_MIN_SIZE_NORMAL,
            ButtonSize::Compact => TEXT_MIN_SIZE_COMPACT,
        }
    }

    /// 文案左右各留出的空白。
    fn padding_x(self) -> f32 {
        match self {
            ButtonSize::Normal => PADDING_X_NORMAL,
            ButtonSize::Compact => PADDING_X_COMPACT,
        }
    }

    /// 宽度下限。给"发送"这种只有两个字的文案兜底——纯按内容算出来会比
    /// [`ButtonSize::height`] 还窄，看起来像标签而不是按钮。
    fn min_width(self) -> f32 {
        match self {
            ButtonSize::Normal => MIN_WIDTH_NORMAL,
            ButtonSize::Compact => MIN_WIDTH_COMPACT,
        }
    }
}

/// 按钮底色：常态 / 悬停 / 按下三档里被选中的那一档。
///
/// 按下比常态**更暗**、悬停比常态**更亮**（深浅两套主题同向），与「打开串口」以及
/// egui 默认控件的行为一致（深色主题下 `inactive` `gray(60)` → `hovered` `gray(70)`
/// → `active` `gray(55)`）。禁用时走另一套更接近面板底色的值，见下。
///
/// 禁用态**不是**把常态乘一个透明度系数：那样在浅色主题下会得到一块"更浅的灰"，
/// 反而比周围更扎眼。这里给的是两个固定值，与各自主题的面板底色只差一点点。
pub(crate) fn button_fill(
    tone: ButtonTone,
    dark: bool,
    hovered: bool,
    pressed: bool,
    enabled: bool,
) -> Color32 {
    if !enabled {
        return if dark {
            Color32::from_rgb(0x2E, 0x2E, 0x2E)
        } else {
            Color32::from_rgb(0xED, 0xED, 0xED)
        };
    }

    let (base, hover, press) = match (tone, dark) {
        // 绿的三个值取自「打开串口」那一档，两处必须一致：它们表达的是同一个意思
        // （"按下去会连上 / 会发起"），各写一个近似绿迟早漂移成两种绿。
        (ButtonTone::Primary, false) => (
            Color32::from_rgb(0x17, 0x79, 0x4A),
            Color32::from_rgb(0x1E, 0x8F, 0x58),
            Color32::from_rgb(0x0F, 0x5C, 0x38),
        ),
        (ButtonTone::Primary, true) => (
            Color32::from_rgb(0x1A, 0x7F, 0x4B),
            Color32::from_rgb(0x21, 0x90, 0x5A),
            Color32::from_rgb(0x14, 0x66, 0x3C),
        ),
        // 中性灰要**明显亮于面板**才看得出是个按钮：深色面板是 `gray(27)`（#1B1B1B），
        // 所以常态取 #3A3A3A；浅色面板是 #F8F8F8，常态取 #DCDCDC 才有对比。
        (ButtonTone::Neutral, false) => (
            Color32::from_rgb(0xDC, 0xDC, 0xDC),
            Color32::from_rgb(0xE8, 0xE8, 0xE8),
            Color32::from_rgb(0xC8, 0xC8, 0xC8),
        ),
        (ButtonTone::Neutral, true) => (
            Color32::from_rgb(0x3A, 0x3A, 0x3A),
            Color32::from_rgb(0x47, 0x47, 0x47),
            Color32::from_rgb(0x30, 0x30, 0x30),
        ),
    };

    if pressed {
        press
    } else if hovered {
        hover
    } else {
        base
    }
}

/// 按钮上的墨色（文案）。
///
/// 绿底一律白字（两种主题都是深绿，白字对比度都够）；中性灰底在深色主题下配浅字、
/// 浅色主题下配深字。禁用态改用贴近面板的灰，让人一眼看出"这里现在点不动"。
pub(crate) fn button_text_color(tone: ButtonTone, dark: bool, enabled: bool) -> Color32 {
    if !enabled {
        return if dark {
            Color32::from_rgb(0x6E, 0x6E, 0x6E)
        } else {
            Color32::from_rgb(0xA0, 0xA0, 0xA0)
        };
    }

    match tone {
        ButtonTone::Primary => Color32::WHITE,
        ButtonTone::Neutral => {
            if dark {
                Color32::from_rgb(0xE6, 0xE6, 0xE6)
            } else {
                Color32::from_rgb(0x1B, 0x1B, 0x1B)
            }
        }
    }
}

/// 画一个统一的按钮，返回它的响应。
///
/// 这是本模块**唯一的绘制入口**，其余三个便捷函数都转发到这里。抽成自由函数（而不是
/// `Widget` 实现）是为了能被无头测试直接驱动：`Widget::ui` 只收 `&mut Ui`，测试照样
/// 造得出来，但 `ui.add(...)` 那条路上的 `Ui` 配置（`available_width` 等）得自己摆，
/// 直接收 `&mut Ui` 更省事。
///
/// 内部顺序刻意是"先量后画"：
///
/// 1. 量文案宽度（`layout_no_wrap`）；
/// 2. 由内容宽与上下限算出按钮宽度，并夹进 `ui.available_width()`；
/// 3. 宽度不够时按比例缩字号；
/// 4. 最后才 `allocate_exact_size` 占位并绘制。
///
/// 第 2 步的夹取必须在 `allocate_exact_size` **之前**——占位之后再改宽是无意义的，
/// 布局已经按旧宽度推进了光标。
pub(crate) fn button(ui: &mut Ui, label: &str, tone: ButtonTone, size: ButtonSize) -> Response {
    // 禁用状态由外层 `add_enabled_ui` / `add_enabled` 下发到 `Ui` 上，这里读出来即可。
    // 读它有两个用处：画禁用配色，以及把 `Sense` 降成 `hover`——后者保证禁用时
    // `response.clicked()` 恒为假，不会出现"按钮灰着但点得动"。
    let enabled = ui.is_enabled();
    let dark = ui.visuals().dark_mode;
    let ink = button_text_color(tone, dark, enabled);
    let base_text_size = size.text_size();
    let padding = size.padding_x();

    // 1) 先量文案：按钮宽度要贴合内容，不能占满整行。
    let natural = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(base_text_size), ink)
        .size()
        .x;

    // 2) 宽度 = 内容 + 左右留白，上限**只能是可用宽度**。
    //
    // 上限不取 `max(available, 下限)`：那样在极窄的容器里会算出超过可用宽度的值，
    // 正好违反用户这次点名的要求（"不允许覆盖到其它无关的组件"）。宁可按钮被挤窄，
    // 也不允许它越界——窄了还能看，越界会压住旁边的东西。
    let wanted = (natural + padding * 2.0).max(size.min_width());
    let available = ui.available_width().max(0.0);
    let width = wanted.min(available);

    // 3) 宽度被迫收窄时按比例缩字号，但不低于下限（**不截断**：截断后看不出这按钮
    //    是干什么的，而缩小的字至少还读得出来）。
    let room = (width - padding * 2.0).max(0.0);
    let text_size = if natural > room && natural > 0.0 {
        (base_text_size * room / natural).max(size.text_min_size())
    } else {
        base_text_size
    };

    let (rect, response) = ui.allocate_exact_size(
        vec2(width, size.height()),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };

    let fill = button_fill(
        tone,
        dark,
        response.hovered(),
        response.is_pointer_button_down_on(),
        enabled,
    );

    // 文字用**带裁剪**的 painter 画：万一容器窄到"缩到最小字号仍放不下"，也只会让字
    // 被裁在按钮内，而不会跑出去压住旁边的控件。正常宽度下裁剪不起作用（文字本来就
    // 只有一半宽）。这是"不允许覆盖到其它无关的组件"这条要求的最后一道防线。
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, CornerRadius::same(BUTTON_CORNER), fill);
    if !label.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(text_size),
            ink,
        );
    }

    response
}

/// 标准尺寸的**普通**按钮（中性灰）——界面上最常见的那个。
pub(crate) fn standard(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, ButtonTone::Neutral, ButtonSize::Normal)
}

/// 标准尺寸的**主操作**按钮（绿）。
pub(crate) fn primary(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, ButtonTone::Primary, ButtonSize::Normal)
}

/// 紧凑尺寸的普通按钮（中性灰）——给与下拉框 / 输入框同行的窄位用。
pub(crate) fn compact(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, ButtonTone::Neutral, ButtonSize::Compact)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次绘制取回的素材。
    struct Painted {
        /// 所有填充矩形：`(矩形, 填充色)`。
        rects: Vec<(egui::Rect, Color32)>,
        /// 所有文字：`(文本, 字号)`。
        texts: Vec<(String, f32)>,
    }

    impl Painted {
        /// 按**高度**挑出按钮背景矩形。
        ///
        /// 不"取第一个"：目标是"这次绘制里那个高 N 的实心块"，按高度认比按顺序认更
        /// 贴近断言的本意（将来谁给测试画布加一层背景也不会误伤）。
        fn button(&self, height: f32) -> egui::Rect {
            self.rects
                .iter()
                .map(|(rect, _)| *rect)
                .find(|rect| (rect.height() - height).abs() < 0.5)
                .unwrap_or_else(|| {
                    panic!(
                        "没找到高 {height} 的按钮背景矩形——按钮没画自己的底，\
                         或者高度不是约定值。实际矩形：{:?}",
                        self.rects.iter().map(|(rect, _)| *rect).collect::<Vec<_>>()
                    )
                })
        }

        /// 按钮里那段文字的字号。
        fn font_size(&self) -> f32 {
            self.texts.first().expect("按钮里没画文案").1
        }
    }

    /// 在给定宽度与主题下画一次，取回素材。
    ///
    /// `screen_rect` 的宽度就是这次绘制的**可用宽度**：`Context::run_ui` 造的根 `Ui`
    /// 铺满整幅画面且不带 frame，所以把画布收窄等价于把按钮放进一个窄容器——正是
    /// 要验的场景（侧边栏拖到最窄、一行里按钮前面还排着标签与下拉框）。
    fn paint(width: f32, dark: bool, draw: impl FnMut(&mut Ui)) -> Painted {
        let ctx = egui::Context::default();
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, 400.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, draw);

        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
                egui::epaint::Shape::Text(text) => texts.push((
                    text.galley.text().to_owned(),
                    text.galley
                        .job
                        .sections
                        .first()
                        .map_or(0.0, |section| section.format.font_id.size),
                )),
                _ => {}
            }
        }
        // 不清理这个 delta 会在 drop 时 panic——看着像抓到缺陷，其实是清理没做。
        out.textures_delta.clear();

        Painted { rects, texts }
    }

    /// 三通道绝对差之和，用来比较"谁更接近面板底色"。
    fn distance(a: Color32, b: Color32) -> u32 {
        u32::from(a.r().abs_diff(b.r()))
            + u32::from(a.g().abs_diff(b.g()))
            + u32::from(a.b().abs_diff(b.b()))
    }

    /// 悬停比常态**亮**、按下比常态**暗**——两档配色 × 两套主题都要成立。
    ///
    /// 方向断言比较的是三通道之和，不是"两个颜色不一样"：后者在把亮暗对调之后依然
    /// 成立，等于没测。
    #[test]
    fn hover_is_brighter_and_press_is_darker() {
        fn brightness(color: Color32) -> u32 {
            u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b())
        }

        for dark in [false, true] {
            for tone in [ButtonTone::Primary, ButtonTone::Neutral] {
                let base = button_fill(tone, dark, false, false, true);
                let hover = button_fill(tone, dark, true, false, true);
                let press = button_fill(tone, dark, false, true, true);

                assert!(
                    brightness(hover) > brightness(base),
                    "dark={dark} / {tone:?}：悬停没有比常态更亮（{base:?} → {hover:?}）"
                );
                assert!(
                    brightness(press) < brightness(base),
                    "dark={dark} / {tone:?}：按下没有比常态更暗（{base:?} → {press:?}）"
                );
                // 指针按着不放时 `hovered()` 也为真，此时必须以按下为准。
                assert_eq!(
                    button_fill(tone, dark, true, true, true),
                    press,
                    "dark={dark} / {tone:?}：按下 + 悬停时应以按下为准"
                );
            }
        }
    }

    /// 主操作档必须**偏绿**，普通档必须是**中性**的（三通道接近）。
    ///
    /// 两条都是方向/形状断言，不是"两色不同"——把红绿对调、或者把中性灰偷偷染成
    /// 淡蓝，"不同"依然成立，但这两条会红。
    #[test]
    fn primary_is_green_and_neutral_is_grey() {
        for dark in [false, true] {
            let primary = button_fill(ButtonTone::Primary, dark, false, false, true);
            assert!(
                primary.g() > primary.r() && primary.g() > primary.b(),
                "dark={dark} 时主操作档不偏绿：{primary:?}"
            );

            let neutral = button_fill(ButtonTone::Neutral, dark, false, false, true);
            let max = neutral.r().max(neutral.g()).max(neutral.b());
            let min = neutral.r().min(neutral.g()).min(neutral.b());
            assert!(
                max - min <= 2,
                "dark={dark} 时普通档不是中性灰（通道差 {max}−{min}）：{neutral:?}"
            );
        }
    }

    /// 禁用态要**明显不同于**常态，而且要**更接近面板底色**。
    ///
    /// 只断言"和常态不一样"是不够的：把禁用底色写成任意另一个亮色也能过，而那样
    /// 在界面上会比常态**更**扎眼——正好反了。所以判据是"到面板底色的距离更小"。
    #[test]
    fn disabled_fill_is_flatter_than_enabled() {
        for dark in [false, true] {
            // 两套主题各自的面板底色，取自 egui 默认 visuals。
            let panel = if dark {
                Color32::from_rgb(0x1B, 0x1B, 0x1B)
            } else {
                Color32::from_rgb(0xF8, 0xF8, 0xF8)
            };

            for tone in [ButtonTone::Primary, ButtonTone::Neutral] {
                let on = button_fill(tone, dark, false, false, true);
                let off = button_fill(tone, dark, false, false, false);

                assert_ne!(on, off, "dark={dark} / {tone:?}：禁用态与常态同色");
                assert!(
                    distance(off, panel) < distance(on, panel),
                    "dark={dark} / {tone:?}：禁用态没有更贴近面板底色（看不出禁用）：\
                     常态 {on:?} 距面板 {}，禁用 {off:?} 距面板 {}",
                    distance(on, panel),
                    distance(off, panel)
                );
            }
        }
    }

    /// 两档高度写死：标准 **28**、紧凑 **24**。
    ///
    /// 判据用**字面量**、不引用 `ButtonSize::height()`——引用了就变成"实现改一改、
    /// 断言跟着变松"的自证。本项目在连接按钮的字号那条上正是这么栽过两次。
    #[test]
    fn sizes_are_the_documented_heights() {
        let normal = paint(320.0, true, |ui| {
            standard(ui, "清空日志");
        });
        let rect = normal.button(28.0);
        assert!(
            (rect.height() - 28.0).abs() < 0.5,
            "标准档高度是 {}，应当是 28",
            rect.height()
        );

        let packed = paint(320.0, true, |ui| {
            compact(ui, "刷新");
        });
        let rect = packed.button(24.0);
        assert!(
            (rect.height() - 24.0).abs() < 0.5,
            "紧凑档高度是 {}，应当是 24",
            rect.height()
        );
    }

    /// 两档字号写死：标准 **15**、紧凑 **14**，且**都大于正文档**。
    ///
    /// 「大于正文」这半边是接着上一轮那条反馈来的（用户说过"这几个字太小了"）。
    /// 但阈值必须写死：egui 0.36.1 的 `TextStyle::Body` 实测只有 13.0，"比正文大"
    /// 是个很松的条件，单靠它放不住"又缩回小字"。
    #[test]
    fn label_sizes_are_fixed_and_above_the_body_default() {
        let mut body_size = 0.0_f32;
        let normal = paint(320.0, true, |ui| {
            body_size = egui::TextStyle::Body.resolve(ui.style()).size;
            standard(ui, "清空日志");
        });

        assert_eq!(
            normal.texts.len(),
            1,
            "按钮里应当恰好有一段文字：{:?}",
            normal.texts
        );
        let size = normal.font_size();
        assert!((size - 15.0).abs() < 0.01, "标准档字号是 {size}，应当是 15");
        assert!(
            size > body_size,
            "标准档字号 {size} 没有大于正文的 {body_size}"
        );

        let packed = paint(320.0, true, |ui| {
            compact(ui, "刷新");
        });
        let size = packed.font_size();
        assert!((size - 14.0).abs() < 0.01, "紧凑档字号是 {size}，应当是 14");
    }

    /// 宽度**贴合文案**：长文案更宽，而且谁都不吃满整行。
    ///
    /// 这一条接着用户上一轮的反馈（"按钮不需要那么长贴到右边框"）。只断言"不超上限"
    /// 会漏掉"所有按钮都做成同一个固定宽度"——那种实现下两个按钮一样宽，这条会红。
    #[test]
    fn width_follows_the_label_and_never_fills_the_row() {
        const CANVAS: f32 = 280.0;

        let short = paint(CANVAS, true, |ui| {
            standard(ui, "发送");
        });
        let long = paint(CANVAS, true, |ui| {
            standard(ui, "清空接收区日志");
        });

        let short_w = short.button(28.0).width();
        let long_w = long.button(28.0).width();

        assert!(
            long_w > short_w,
            "长文案的按钮没有更宽（{short_w} vs {long_w}）——宽度没跟着文案走"
        );
        assert!(
            long_w < CANVAS - 1.0,
            "长文案的按钮吃满了整行（{long_w} / {CANVAS}）"
        );
        assert!(
            short_w < CANVAS - 1.0,
            "短文案的按钮吃满了整行（{short_w} / {CANVAS}）"
        );
    }

    /// ⚠️ **宽度永远不超可用宽度**——用户这次点名的要求（"不允许覆盖到其它无关的
    /// 组件"）在"容器比按钮还窄"这一情形下的落地。
    ///
    /// 容器只有 90 宽而要放一句七个字的文案：按钮只能缩到 90 以内，字号跟着缩，
    /// 但**不低于下限**（不截断、也不把字缩到看不见）。
    #[test]
    fn width_never_exceeds_the_available_space() {
        const NARROW: f32 = 90.0;

        let painted = paint(NARROW, true, |ui| {
            standard(ui, "清空接收区日志");
        });
        let rect = painted.button(28.0);

        assert!(
            rect.width() <= NARROW + 0.5,
            "按钮宽 {} 超出了可用宽度 {NARROW}——这正是会压到同行其它控件的那种越界",
            rect.width()
        );

        let size = painted.font_size();
        assert!(
            size < 15.0 - 0.01,
            "宽度被挤窄之后字号没跟着缩（仍是 {size}）——文字会溢出按钮"
        );
        assert!(size >= 13.0 - 0.01, "字号被缩到了 {size}，低于下限 13");
    }

    /// ⚠️ **同一行里两个按钮不重叠，而且留出间距**。
    ///
    /// 这是"不允许覆盖到其它无关的组件"在"两个按钮挨着"这一情形下的落地。除了不
    /// 重叠，还要**留缝**：两块实心色贴在一起会被看成一块，用户认不出是两个按钮。
    #[test]
    fn two_buttons_side_by_side_never_overlap() {
        let painted = paint(280.0, true, |ui| {
            ui.horizontal(|ui| {
                standard(ui, "清空日志");
                standard(ui, "导出日志");
            });
        });

        let mut rects: Vec<egui::Rect> = painted
            .rects
            .iter()
            .map(|(rect, _)| *rect)
            .filter(|rect| (rect.height() - 28.0).abs() < 0.5)
            .collect();
        assert_eq!(
            rects.len(),
            2,
            "应当恰好有两个高 28 的按钮背景：{:?}",
            painted.rects
        );
        rects.sort_by(|a, b| a.min.x.partial_cmp(&b.min.x).expect("坐标不该是 NaN"));

        assert!(
            rects[0].max.x <= rects[1].min.x,
            "两个按钮重叠了：{:?} 与 {:?}",
            rects[0],
            rects[1]
        );
        assert!(
            rects[1].min.x - rects[0].max.x >= 4.0,
            "两个按钮只隔了 {} 点——贴在一起会被看成一个色块",
            rects[1].min.x - rects[0].max.x
        );
    }
}
