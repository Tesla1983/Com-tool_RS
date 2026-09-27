//! 右侧边栏：串口信息回显（含可用端口列表）、连接参数设置与日志操作入口。
//!
//! **不负责读数**：接收/发送字节数是底部状态栏的常驻读数（见 [`crate::status_bar`]），
//! 这里只画"当前设置"类的描述（端口、波特率、显示模式、时间戳前缀…）。
//!
//! 绘制函数只负责画控件并把「用户按了哪个按钮」记录下来，真正的状态变更与
//! 副作用（关闭串口、清空日志、弹出导出对话框）全部交回调用方执行。这样
//! 派发逻辑集中在 `app.rs` 一处，侧边栏不会在渲染过程中偷偷改状态。

use crate::app::{ComTool, ConnectionKind};
use crate::serial_impl::{BAUD_CUSTOM_SEED, BAUD_RATES, CUSTOM_BAUD_MAX, CUSTOM_BAUD_MIN};
use crate::strings::{Strings, fill, timestamp_prefix_label};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use serialport::{DataBits, FlowControl, Parity, StopBits};

/// 波特率档位的选择结果。
///
/// 抽成自由函数是为了**可测**：这段"选哪一档"的语义（见下）很容易写错，而它是纯计算，
/// 不需要窗口也不需要串口就能断言。绘制那段只负责把点击翻译成 [`BaudPick`] 再调用它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaudPick {
    /// 点了列表里的某个档位
    Listed(u32),
    /// 点了「自定义…」这一档
    Custom,
}

/// 把一次点击应用到 `(baud_rate, baud_custom)` 上。
///
/// 三条语义，每一条都有理由：
///
/// 1. **点列表档位** → 记下该值并退出自定义档（`baud_custom = false`）。
/// 2. **点「自定义」且本来就是自定义档** → 什么都不做。否则用户输好的值会被种子覆盖。
/// 3. **点「自定义」且当前值在列表里** → 换成 [`BAUD_CUSTOM_SEED`]：用户选自定义的意图
///    就是"要输一个列表里没有的值"，留着他刚选的那个等于让他先删几个字符。
///    反过来，**当前值本来就不在列表里**（比如从旧存档读回来的 74880）→ 保留不动，
///    那正是他想输的东西。
///
/// 关键的一条不变式：**`baud_custom == true` 时 `baud_rate` 是唯一真值**，
/// 提交给驱动的永远是它；"选中哪一档"从不参与 `SerialConfig` 的组装。
pub(crate) fn apply_baud_pick(baud_rate: &mut u32, baud_custom: &mut bool, pick: BaudPick) {
    match pick {
        BaudPick::Listed(value) => {
            *baud_rate = value;
            *baud_custom = false;
        }
        BaudPick::Custom => {
            if !*baud_custom && BAUD_RATES.contains(baud_rate) {
                *baud_rate = BAUD_CUSTOM_SEED;
            }
            *baud_custom = true;
        }
    }
}

/// 连接类型单选（串口 / TCP 客户端 / UDP）。
///
/// 抽成自由函数而不是内联在 [`ComTool::show_sidebar`] 里：侧边栏那段闭包已经很长，
/// 而且它会**借用 `self.connection_kind`**——与闭包里其它 `&mut self` 的用法混在
/// 一起时，读的人很难一眼看出哪几个借用是同时活的。这里只收一个 `&mut` 字段。
fn connection_kind_radios(ui: &mut Ui, kind: &mut ConnectionKind, s: &Strings) {
    ui.horizontal(|ui| {
        for (value, label) in [
            (ConnectionKind::Serial, s.conn_serial),
            (ConnectionKind::TcpClient, s.conn_tcp),
            (ConnectionKind::Udp, s.conn_udp),
        ] {
            ui.radio_value(kind, value, label);
        }
    });
}

// ——— 连接按钮的外观 ———
//
// 这个按钮是侧边栏里唯一的"主操作"，2026-09-24 重做。重做的原因是它**看不出是个
// 按钮**：原先只是 `egui::Button::new(label).selected(is_connected)`，宽度、高度、
// 填充、描边、圆角一个都没给，全吃 egui 默认值。实测（`egui-0.36.1/src/style.rs`）：
// `button_padding` 只有 `vec2(4.0, 1.0)` ⇒ 高度几乎等于文字行高、宽度随文案长度变；
// 深色主题下底色 `gray(60)` 压在 `panel_fill` `gray(27)` 上、圆角 2px、**无描边**；
// 已连接时走 `selection.bg_fill`（深色主题是 `rgb(0,92,128)`）——一样很弱。
//
// 现在改为自绘（[`draw_connect_button`]），量全部显式给死。

/// 连接按钮的高度（逻辑像素）。32 是为了与下面的端口/波特率下拉框拉开主次，
/// 同时不至于像"大号操作卡"那样占掉一个折叠区的高度。
const CONNECT_BUTTON_HEIGHT: f32 = 32.0;

/// 连接按钮的圆角半径。
const CONNECT_BUTTON_CORNER: u8 = 6;

/// 连接按钮的**最小宽度**。宽度本身贴合内容（见 [`draw_connect_button`]），
/// 这个下限是给极短文案兜底的：UDP 未连接时只有「绑定」两个字，纯按内容算出来
/// 不到 90 点宽，看起来像标签而不是按钮。
const CONNECT_BUTTON_MIN_WIDTH: f32 = 130.0;

/// 连接按钮右边缘与面板右边界之间**至少**留出的空白。
///
/// 2026-09-24 用户实机目视：「按钮不需要那么长贴到右边框」。宽度改成贴合内容后，
/// 多数语言都用不到这个上限；它是给德语/俄语那类长译文兜底的——超长时宁可把文案
/// 按比例缩小（见 [`CONNECT_TEXT_MIN_SIZE`]），也不让按钮顶到面板边上。
const CONNECT_BUTTON_RIGHT_MARGIN: f32 = 24.0;

/// 连接按钮内、状态点之前的左留白，以及箭头之后的右留白。
const CONNECT_BUTTON_PADDING_X: f32 = 12.0;

/// 状态点直径。
const CONNECT_DOT_DIAMETER: f32 = 9.0;

/// 状态点描边宽度（只在空心那一档用得上）。
const CONNECT_DOT_STROKE: f32 = 1.5;

/// 状态点与文案之间的间距。
const CONNECT_DOT_GAP: f32 = 9.0;

/// 右侧箭头占的宽度与高度。
const ARROW_WIDTH: f32 = 5.0;
const ARROW_HEIGHT: f32 = 9.0;

/// 文案与箭头之间至少留出的间距。
const CONNECT_TEXT_ARROW_GAP: f32 = 8.0;

/// 文案基准字号。
///
/// **2026-09-24 用户实机目视后由 14 调到 16**。原先取 14 的理由是"与 egui 正文档
/// 一致"——这个理由本身也是错的：`TextStyle::Body` 在 egui 0.36.1 里**实测是 13.0**
/// （读的是 `egui-0.36.1` 本机源码与运行期 `resolve(..)`，不是记忆里的 14），
/// 所以 14 只是"比正文大 1 点"，肉眼几乎无差别。用户的原话是「打开窗口这四个字
/// 太小了」。现在这一档**故意明显大于**正文：它是主操作按钮的文案，不是正文。
const CONNECT_TEXT_SIZE: f32 = 16.0;

/// 文案缩到的最小字号。15 种语言里德语/俄语明显长于中文，放不下时按比例缩到
/// 这里为止；再长的文案（目前没有）宁可略微压到箭头附近，也不截断——截断会让人
/// 看不出按钮是干什么的。
///
/// 14 这个下限的取法是"**不低于正文**"（`Body` = 13.0）：缩到最狠也要和正文齐平，
/// 不能因为译文长就把按钮上的字缩成最小的那一档。
const CONNECT_TEXT_MIN_SIZE: f32 = 14.0;

/// 连接按钮的**动作**档——决定底色，与文案无关。
///
/// 注意它与底部状态栏的圆点是"同色不同义"：状态栏报"现在处于什么状态"，
/// 这里报"按下去会做什么"。两者只在「正在打开」这一档重合（都是琥珀
/// `#F5A623`），有测试钉住这个重合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectButtonTone {
    /// 未连接：绿＝"按下去会连上"。
    Idle,
    /// 已发起、通道尚未就绪：琥珀＝"正在进行"。
    Opening,
    /// 已连接：红＝"按下去会断开"（准破坏动作）。
    Connected,
}

/// 由"是否已连接 / 是否正在打开"定出按钮的动作档。
///
/// `is_opening` 只在串口出现（它的 `open()` 只拉起后台线程，端口要到那里才真正
/// 打开）。这一档必须**压过**已连接之外的一切：那段窗口期里通道还没就绪，把按钮
/// 直接画成已连接的红色，用户会以为已经能发数据了。
pub(crate) fn connect_button_tone(is_connected: bool, is_opening: bool) -> ConnectButtonTone {
    match (is_connected, is_opening) {
        (false, _) => ConnectButtonTone::Idle,
        (true, true) => ConnectButtonTone::Opening,
        (true, false) => ConnectButtonTone::Connected,
    }
}

/// 按钮底色。返回的是 `(常态, 悬停, 按下)` 三档里选中的那一档。
///
/// 按下比常态**更暗**、悬停比常态**更亮**（深浅两套主题同向）——这与 egui 默认
/// 控件在深色主题下的行为一致（`inactive` `gray(60)` → `hovered` `gray(70)` →
/// `active` `gray(55)`），所以手感不会与旁边的下拉框、单选钮打架。
pub(crate) fn connect_button_fill(
    tone: ConnectButtonTone,
    dark: bool,
    hovered: bool,
    pressed: bool,
) -> Color32 {
    let (base, hover, press) = match (tone, dark) {
        (ConnectButtonTone::Idle, false) => (
            Color32::from_rgb(0x17, 0x79, 0x4A),
            Color32::from_rgb(0x1E, 0x8F, 0x58),
            Color32::from_rgb(0x0F, 0x5C, 0x38),
        ),
        (ConnectButtonTone::Idle, true) => (
            Color32::from_rgb(0x1A, 0x7F, 0x4B),
            Color32::from_rgb(0x21, 0x90, 0x5A),
            Color32::from_rgb(0x14, 0x66, 0x3C),
        ),
        // 「正在打开」两套主题共用同一个琥珀，而且**就是底部状态栏那个琥珀**
        // （`ConnectionIndicator::pending`）：它俩说的是同一件事，各写一个近似色
        // 迟早会漂移成两个"差不多但不一样"的黄。有测试钉住这个相等。
        (ConnectButtonTone::Opening, _) => (
            Color32::from_rgb(0xF5, 0xA6, 0x23),
            Color32::from_rgb(0xFF, 0xB9, 0x3D),
            Color32::from_rgb(0xC9, 0x86, 0x1A),
        ),
        // 断开是准破坏动作，两套主题共用同一档红：它要的就是"被看见"。
        (ConnectButtonTone::Connected, _) => (
            Color32::from_rgb(0xB3, 0x26, 0x1E),
            Color32::from_rgb(0xC4, 0x33, 0x2A),
            Color32::from_rgb(0x8F, 0x1C, 0x16),
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

/// 按钮内的墨色（文案、状态点、箭头共用一个颜色）。
///
/// 琥珀底必须配深色字：白字压在 `#F5A623` 上对比度只有约 1.8:1，压 `#1B1B1B`
/// 约 8:1；浅色主题的 `#E08A00` 上配深字也有约 5.9:1。绿底与红底则必须配白字。
pub(crate) fn connect_button_text_color(tone: ConnectButtonTone) -> Color32 {
    match tone {
        ConnectButtonTone::Opening => Color32::from_rgb(0x1B, 0x1B, 0x1B),
        ConnectButtonTone::Idle | ConnectButtonTone::Connected => Color32::WHITE,
    }
}

/// 状态点是否画成空心圆。只有"还没连上"这一档是空心的。
pub(crate) fn connect_dot_is_hollow(tone: ConnectButtonTone) -> bool {
    matches!(tone, ConnectButtonTone::Idle)
}

/// 连接按钮的文案。
///
/// **判据是 `is_connected`，不是别的**：为真就一律属于"关闭/断开"那一侧——**包括**
/// 串口的「正在打开」（那时 `is_connected` 也为真）。若那时显示「打开串口」，用户
/// 会以为刚才那下没生效而重复点，而按下去实际是**关闭**：这正是 F4 修过的
/// "文案与动作相反"。那一态要说的是"正在进行"，由琥珀底色 + 按钮下方的小字交代。
pub(crate) fn connect_button_label(
    kind: ConnectionKind,
    is_connected: bool,
    s: &Strings,
) -> &'static str {
    if is_connected {
        if kind.uses_serial_settings() {
            s.close_port_button
        } else {
            s.disconnect_button
        }
    } else {
        match kind {
            ConnectionKind::Serial => s.open_port_button,
            ConnectionKind::TcpClient => s.connect_button,
            ConnectionKind::Udp => s.bind_button,
        }
    }
}

/// 画「打开/关闭连接」主操作按钮，返回它的响应。
///
/// **自绘而不是 `egui::Button`**，三条理由：
///
/// 1. 尺寸要**显式给死**：高度固定，宽度按内容算、夹在 [`CONNECT_BUTTON_MIN_WIDTH`]
///    与「可用宽度 − [`CONNECT_BUTTON_RIGHT_MARGIN`]」之间。`Button` 的尺寸只由文案
///    与 `button_padding` 决定，想调下限、想留右边距都得绕开它；
/// 2. 按钮内部要排三个元素（状态点 / 文案 / 右侧箭头），`Button` 只画居中文字；
/// 3. 底色按"动作语义"取（可打开＝绿、可关闭＝红、进行中＝琥珀），用 `Button::fill`
///    得在每个调用点重算主题与悬停态，等于把配色规则抄得到处都是。
///
/// 抽成自由函数还有一个更实际的原因：能被无头测试直接驱动（见
/// `tests::painted_button_variant`）。内联在 `show_sidebar` 的闭包里，就等于一次也
/// 不会被 `cargo test` 画到——`show_sidebar` 要的 `Ui` 在无头测试里是造得出来的，
/// 但那样每条断言都得先绕开侧边栏其余几百行控件。
fn draw_connect_button(ui: &mut Ui, tone: ConnectButtonTone, label: &str) -> egui::Response {
    let ink = connect_button_text_color(tone);

    // 宽度**贴合内容**，不再吃满整行。2026-09-24 用户实机目视：「按钮不需要那么长
    // 贴到右边框」——一块占满侧边栏的实心色块看起来像横幅，不像按钮。
    //
    // 必须先量后定宽：文本按 `CONNECT_TEXT_SIZE` 量一遍，加上两侧留白、状态点、
    // 箭头就是"刚好放得下一行字"的宽度。量文本用的 painter 只在块内活着——出了块
    // 才 `allocate_exact_size`，因为两者都要借 `ui`，同时借会冲突。
    let natural_text = {
        let painter = ui.painter();
        painter
            .layout_no_wrap(
                label.to_owned(),
                FontId::proportional(CONNECT_TEXT_SIZE),
                ink,
            )
            .size()
            .x
    };
    let content_width = CONNECT_BUTTON_PADDING_X * 2.0
        + CONNECT_DOT_DIAMETER
        + CONNECT_DOT_GAP
        + natural_text
        + CONNECT_TEXT_ARROW_GAP
        + ARROW_WIDTH;
    // 上限也夹一次最小宽度：侧边栏被拉到最窄时 `available_width()` 本身就很小，
    // 不夹的话下面的 `.min()` 会算出比最小宽度还小的值，clamp 的上下界就反了。
    let width_ceiling =
        (ui.available_width() - CONNECT_BUTTON_RIGHT_MARGIN).max(CONNECT_BUTTON_MIN_WIDTH);
    let width = content_width
        .max(CONNECT_BUTTON_MIN_WIDTH)
        .min(width_ceiling);

    let (rect, response) =
        ui.allocate_exact_size(vec2(width, CONNECT_BUTTON_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);

    let fill = connect_button_fill(
        tone,
        ui.visuals().dark_mode,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(CONNECT_BUTTON_CORNER), fill);

    // 状态点：空心＝还没连上，实心＝已发起。
    let radius = CONNECT_DOT_DIAMETER / 2.0;
    let dot = pos2(
        rect.left() + CONNECT_BUTTON_PADDING_X + radius,
        rect.center().y,
    );
    if connect_dot_is_hollow(tone) {
        painter.circle_stroke(dot, radius, Stroke::new(CONNECT_DOT_STROKE, ink));
    } else {
        painter.circle_filled(dot, radius, ink);
    }

    // 右侧箭头**画几何图形，不用 `▸` 字符**：符号字形要靠字体兜，而 `epaint` 只取
    // 字体里的矢量轮廓，字体没覆盖到就是一个方框——接收区那条教训对界面文字同样成立。
    let arrow_center_x = rect.right() - CONNECT_BUTTON_PADDING_X - ARROW_WIDTH / 2.0;
    painter.add(Shape::convex_polygon(
        vec![
            pos2(
                arrow_center_x - ARROW_WIDTH / 2.0,
                rect.center().y - ARROW_HEIGHT / 2.0,
            ),
            pos2(arrow_center_x + ARROW_WIDTH / 2.0, rect.center().y),
            pos2(
                arrow_center_x - ARROW_WIDTH / 2.0,
                rect.center().y + ARROW_HEIGHT / 2.0,
            ),
        ],
        ink,
        Stroke::NONE,
    ));

    // 文案：从状态点右侧起。先按基准字号量一次，放不下就按比例缩，缩到
    // `CONNECT_TEXT_MIN_SIZE` 为止（不截断——截断后用户看不出这按钮是干什么的）。
    let text_left = dot.x + radius + CONNECT_DOT_GAP;
    let room = arrow_center_x - ARROW_WIDTH / 2.0 - CONNECT_TEXT_ARROW_GAP - text_left;
    let natural = painter
        .layout_no_wrap(
            label.to_owned(),
            FontId::proportional(CONNECT_TEXT_SIZE),
            ink,
        )
        .size()
        .x;
    let size = if natural > room && natural > 0.0 {
        (CONNECT_TEXT_SIZE * room / natural).max(CONNECT_TEXT_MIN_SIZE)
    } else {
        CONNECT_TEXT_SIZE
    };
    painter.text(
        pos2(text_left, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(size),
        ink,
    );

    response
}

/// 侧边栏本轮产生的操作请求。
///
/// 用结构体而不是多个 `&mut bool` 出参：新增操作时不必再改签名，调用方
/// 也能一次性看清有哪些分支要处理。
#[derive(Default)]
pub(crate) struct SidebarAction {
    /// 用户点了收起按钮。
    pub close_requested: bool,
    /// 用户点了「清空日志」。
    pub clear_requested: bool,
    /// 用户点了「导出日志」。
    pub export_requested: bool,
    /// 用户点了「开始记录」/「停止记录」。
    pub log_toggle_requested: bool,
    /// 用户点了日志的「选择目录」。
    pub pick_log_dir_requested: bool,
}

impl ComTool {
    /// 绘制右侧边栏。
    ///
    /// `open` 由调用方以局部变量传入再回写——`sidebar_open` 是 `self` 的字段，
    /// 若直接取 `&mut self.sidebar_open` 会与下面的 `&mut self` 冲突。
    pub(crate) fn show_sidebar(&mut self, ui: &mut Ui, open: &mut bool) -> SidebarAction {
        let mut action = SidebarAction::default();

        // 文案表是 `&'static`，取出来之后就不借用 `self` 了，闭包里的 `&mut self`
        // 仍然可用（`Language` 是 `Copy`，这一句只是把语言字段抄了一份）。
        let s = self.language.strings();

        // 本帧要展示的只读快照。在绘制前统一取值，之后即便闭包内改了串口
        // 参数，已画出的信息区也仍是本帧开始时的状态，不会前后矛盾。
        let port_name = self.selected_port.clone();
        let is_connected = self.connected;
        // "已经发起打开、但通道还没就绪"——只有串口会有这一档。
        let is_opening = self.opening();
        // 控制线（RTS / DTR）勾选框**能不能点**。判据是"通道是否真的就绪"
        // （与发送闸门同一条），⚠️ **不是 `is_connected`**：那一位的含义是
        // "已经发起过一次打开"，串口在 `open()` 返回后到端口真正打开之间会短暂停在
        // 那一档（界面显示"正在打开"）。此时点下去命令只能进后台队列排队
        // （见 [`crate::app::ComTool::apply_line_state`] 的投递语义），
        // 而勾选框会先显示成已勾选 ⇒ 界面在说谎。
        let lines_controllable = self.channel_is_ready();
        let current_baud = self.baud_rate;
        let hex_mode = self.hex_view;
        let color_mode = self.color_view;
        let timestamp_mode = self.timestamp_prefix;

        // 注意：接收/发送字节数**不在这里取值**了——它们是底部状态栏的读数
        // （见 `status_bar.rs`），侧边栏只保留"当前设置"类的描述。

        // 连接类型决定了「显示哪一组参数」与「状态那一行怎么说」，所以它也要在绘制前
        // 定死：下面的信息区与参数区必须来自同一个时刻的类型，否则用户刚点完单选就
        // 会看到"标题写着连接信息、底下还是波特率"这种半新半旧的画面。
        let kind = self.connection_kind;
        let net_peer = self.net_peer_label();
        let net_local_port = self.net_local_port;

        // UDP 实际绑定到的本地端口：`local_port` 配 0 时由系统分配，用户必须能看见
        // 最终分到哪个端口，否则不知道该让设备往哪回包。串口 / TCP 拿不到（trait 的
        // 默认实现返回 `None`），取不到就退回用户填的值。
        let bound_local_port = self
            .channel
            .as_ref()
            .and_then(|c| c.local_addr())
            .map_or(net_local_port, |addr| addr.port());

        // 状态词按类型分叉：UDP 没有连接，说"已连接"会让用户以为对端在线、
        // 以为掉线会被发现（见 `ConnectionKind` 的文档）。
        //
        // 第三档「正在打开」只有串口会有（`is_opening` 的判定见 `ComTool::opening`）：
        // 串口的 `open()` 只拉起后台线程，端口要到那里才真正打开。这一档**必须**
        // 说出来——否则界面在那一小段里显示"已连接"，用户点「发送」才发现消息发不
        // 出去，看起来就像程序坏了（那正是 FEATURE_GAP_AND_PLAN.md §4 记的假阳性）。
        let status_text = match (is_connected, is_opening, kind) {
            (true, true, _) => s.status_opening,
            (true, false, ConnectionKind::Udp) => s.status_bound,
            (true, false, _) => s.status_connected,
            (false, _, ConnectionKind::Udp) => s.status_unbound,
            (false, _, _) => s.status_disconnected,
        };

        // 「显示模式」那一行由「HEX / 文本」+「+ 颜色」+「+ 字节查看」拼成。也在这里
        // 定好：闭包内再去读 `self` 会让同一行的几个勾选项状态来自不同时刻。
        let display_mode_text = {
            let mut text = String::from(if hex_mode { s.hex } else { s.text });
            if color_mode {
                text.push_str(s.suffix_color);
            }
            if self.ascii_view {
                text.push_str(s.suffix_byte_view);
            }
            text
        };
        let timestamp_text = timestamp_prefix_label(s, timestamp_mode);
        let port_list = self.ports.clone();
        let is_logging = self.is_logging();
        let log_path = self
            .log_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let log_written = self.log_written_bytes();
        let log_dir = self.effective_log_dir().display().to_string();

        egui::Panel::right("right_sidebar")
            .resizable(true)
            .default_size(280.0)
            .min_size(200.0)
            .max_size(600.0)
            .show_collapsible(ui, open, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading(s.nav_sidebar);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 2026-09-24 起走全界面统一的按钮外观（见 `crate::button`）：
                        // 原先这里是 `Button::new(">>").min_size(vec2(32, 32))`，即
                        // egui 默认的"灰标签"外观，与「打开串口」不是一套语言。
                        if crate::button::standard(ui, ">>")
                            .on_hover_text(s.collapse)
                            .clicked()
                        {
                            action.close_requested = true;
                        }
                    });
                });
                ui.separator();

                // 「串口信息」= 端口/波特率/状态 + **可用串口列表** + 显示模式/时间戳前缀。
                //
                // 这里刻意把原先独立的「可用串口」折叠区**并了进来**：两者描述的
                // 都是"当前这条连接长什么样"，分两个折叠头既占竖向空间，又让用户
                // 要在两个地方之间来回看（选了哪个端口、列表里还有哪些）。
                // 网络模式下端口列表没有意义（永远是空的），所以只在串口模式下附上。
                //
                // 「显示模式」「时间戳前缀」也挪到这里（原先挂在「接收统计」下）：
                // 它们是**当前设置**的描述，不是累计读数——读数已经进底部状态栏了。
                egui::CollapsingHeader::new(if kind.uses_serial_settings() {
                    s.serial_info
                } else {
                    s.connection_info
                })
                .default_open(true)
                .show(ui, |ui| {
                    if kind.uses_serial_settings() {
                        ui.label(fill(
                            s.port_label,
                            &[&if port_name.is_empty() {
                                s.not_selected
                            } else {
                                port_name.as_str()
                            }],
                        ));
                        ui.label(fill(s.baud_label, &[&current_baud]));

                        // 可用串口列表：接在端口信息之后，用一个小标题分隔，
                        // 避免与上面的"端口: COM3"混淆成读数。
                        ui.add_space(4.0);
                        ui.weak(s.available_ports);
                        if port_list.is_empty() {
                            ui.weak(s.none_placeholder);
                        } else {
                            for p in &port_list {
                                ui.label(p);
                            }
                        }
                    } else {
                        ui.label(fill(
                            s.target_label,
                            &[&if net_peer.is_empty() {
                                s.not_selected
                            } else {
                                net_peer.as_str()
                            }],
                        ));
                        if kind == ConnectionKind::Udp {
                            ui.label(fill(s.local_port_line, &[&bound_local_port]));
                        }
                    }
                    ui.label(status_text);

                    ui.add_space(4.0);
                    ui.label(fill(s.display_mode_line, &[&display_mode_text]));
                    ui.label(fill(s.timestamp_prefix_line, &[&timestamp_text]));
                });

                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    // —— 连接类型 ——
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new(s.connection_type));
                        // 连接中锁住类型：切了之后，界面上展示的参数（比如"目标
                        // 192.168.1.10"）就与真正打开的那条通道对不上了，而按钮此时
                        // 是「断开」——点下去断的是**另一种**传输。先断开再切。
                        ui.add_enabled_ui(!is_connected, |ui| {
                            connection_kind_radios(ui, &mut self.connection_kind, s);
                        })
                        .response
                        .on_disabled_hover_text(s.switch_kind_needs_disconnect);
                    });

                    // —— 连接 / 断开 ——
                    //
                    // 文案与动作**必须同向**：修 F4 之前这里一直写着反的（已连接时
                    // 显示"打开串口"、点下去却是关闭）。类型不同则用不同的词——
                    // 串口说"打开/关闭"，TCP 说"连接/断开"，UDP 说"绑定/断开"
                    // （UDP 没有连接，说"已连接"会让人以为对端在线）。
                    //
                    // 「正在打开」也归到"已发起"那一侧：此时再显示「打开串口」会让
                    // 用户以为刚才那下没生效而重复点，而按下去实际是**关闭**
                    // （`self.connected` 为真）——又一次"文案与动作相反"。
                    let label = connect_button_label(kind, is_connected, s);
                    if draw_connect_button(ui, connect_button_tone(is_connected, is_opening), label)
                        .clicked()
                    {
                        if self.connected {
                            self.close_connection();
                        } else {
                            self.open_connection();
                        }
                    }

                    // 「正在打开」要单独说一句。它与下面那条"参数没填全"是两回事：
                    // 那个是**点下去不会有反应**，这个是**已经点了、正在等**，
                    // 共用一条提示会把两件事说成一件事。
                    //
                    // 两条提示都放在按钮**下方**单独一行：提示是整句话，与按钮同行会
                    // 被挤成几个字一断，读不成句（按钮收窄之后右边还留着边距，更放不下）。
                    if is_opening {
                        ui.weak(s.opening_hint);
                    } else if !is_connected && !self.connection_ready() {
                        // 参数没填全时按钮点下去会"没反应"（`connection_ready`
                        // 拦下），所以要把原因摆出来。
                        ui.weak(if kind.uses_serial_settings() {
                            s.need_port_to_connect
                        } else {
                            s.need_target_to_connect
                        });
                    }
                    ui.add_space(4.0);

                    if kind.uses_serial_settings() {
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.port_field));
                            egui::ComboBox::from_id_salt("port")
                                .selected_text(if self.selected_port.is_empty() {
                                    s.select_port
                                } else {
                                    &self.selected_port
                                })
                                .show_ui(ui, |ui| {
                                    for port in &self.ports {
                                        ui.selectable_value(
                                            &mut self.selected_port,
                                            port.clone(),
                                            port,
                                        );
                                    }
                                });
                            // 紧凑档：这一行还有标签与下拉框，「刷新」用 24 高的小号，
                            // 免得把整行顶高（见 `crate::button::ButtonSize`）。
                            if crate::button::compact(ui, s.refresh).clicked() {
                                self.refresh_ports();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.baud_field));

                            // 下拉框的当前显示文本：选自定义时显示「自定义」而不是那个
                            // 数字——否则用户看到"自定义"档里显示 115200，会以为没生效。
                            let selected_text = if self.baud_custom {
                                s.baud_custom_entry.to_string()
                            } else {
                                self.baud_rate.to_string()
                            };

                            // 这里只把点击翻译成 [`BaudPick`]，攒在 `pick` 里；
                            // **真正的状态变更在闭包外面**统一交给 `apply_baud_pick`。
                            // 好处是"选哪一档"的语义只有一份实现（而且是纯函数、有测试），
                            // 闭包里那份 `&mut self` 借用也少了两处。
                            let mut pick = None;

                            egui::ComboBox::from_id_salt("baud")
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    for baud in &BAUD_RATES {
                                        // 手写选中判定而不是 `selectable_value`：
                                        // `selectable_value` 只比较"值相等"，而这里
                                        // 选中的依据是 `baud_custom` 标记（见该字段的
                                        // 文档——用户选了自定义档、又输了一个恰好等于
                                        // 列表项的数字时，仍应停留在自定义档）。
                                        let selected = !self.baud_custom && self.baud_rate == *baud;
                                        if ui.selectable_label(selected, baud.to_string()).clicked()
                                        {
                                            pick = Some(BaudPick::Listed(*baud));
                                        }
                                    }

                                    ui.separator();

                                    let custom_selected = self.baud_custom;
                                    if ui
                                        .selectable_label(custom_selected, s.baud_custom_entry)
                                        .clicked()
                                    {
                                        pick = Some(BaudPick::Custom);
                                    }
                                });

                            if let Some(pick) = pick {
                                apply_baud_pick(&mut self.baud_rate, &mut self.baud_custom, pick);
                            }

                            // 只有选中「自定义」才出现输入框：其余档位下它是个多余的控件，
                            // 而且会诱导用户"在这里改一个已经选好的档"，把两个控件的关系
                            // 搞乱。越界由 `range` 当场夹住（不做"打开端口时偷偷改掉"）。
                            if self.baud_custom {
                                ui.add(
                                    egui::DragValue::new(&mut self.baud_rate)
                                        .range(CUSTOM_BAUD_MIN..=CUSTOM_BAUD_MAX)
                                        .speed(1000.0),
                                )
                                .on_hover_text(fill(
                                    s.baud_custom_hint,
                                    &[&CUSTOM_BAUD_MIN, &CUSTOM_BAUD_MAX],
                                ));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.data_bits));
                            egui::ComboBox::from_id_salt("data_bits")
                                .selected_text(format!("{:?}", self.data_bits))
                                .show_ui(ui, |ui| {
                                    for data_bits in [
                                        DataBits::Five,
                                        DataBits::Six,
                                        DataBits::Seven,
                                        DataBits::Eight,
                                    ] {
                                        ui.selectable_value(
                                            &mut self.data_bits,
                                            data_bits,
                                            format!("{:?}", data_bits),
                                        );
                                    }
                                });

                            ui.add_sized([70.0, 20.0], egui::Label::new(s.stop_bits));
                            egui::ComboBox::from_id_salt("stop_bits")
                                .selected_text(format!("{:?}", self.stop_bits))
                                .show_ui(ui, |ui| {
                                    for stop_bits in [StopBits::One, StopBits::Two] {
                                        ui.selectable_value(
                                            &mut self.stop_bits,
                                            stop_bits,
                                            format!("{:?}", stop_bits),
                                        );
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.parity));
                            egui::ComboBox::from_id_salt("parity")
                                .selected_text(format!("{:?}", self.parity))
                                .show_ui(ui, |ui| {
                                    for parity in [Parity::None, Parity::Odd, Parity::Even] {
                                        ui.selectable_value(
                                            &mut self.parity,
                                            parity,
                                            format!("{:?}", parity),
                                        );
                                    }
                                });

                            ui.add_sized([70.0, 20.0], egui::Label::new(s.flow_control));
                            egui::ComboBox::from_id_salt("flow_control")
                                .selected_text(format!("{:?}", self.flow_control))
                                .show_ui(ui, |ui| {
                                    for flow_control in [
                                        FlowControl::None,
                                        FlowControl::Software,
                                        FlowControl::Hardware,
                                    ] {
                                        ui.selectable_value(
                                            &mut self.flow_control,
                                            flow_control,
                                            format!("{:?}", flow_control),
                                        );
                                    }
                                });
                        });

                        // —— 控制线 RTS / DTR（N1）——
                        //
                        // 三个约束都体现在这一段里：
                        //
                        // * **通道没就绪时置灰**（判据 = `lines_controllable`，见它上面
                        //   那段注释）。通道没开就没有地方投递命令，"点了没反应"不如
                        //   一眼看出不能点；置灰的理由**常显**在勾选框下面
                        //   （`ui.weak(line_state_hint)`），与「面板显隐」那处置灰时
                        //   的写法一致。⚠️ 早先只用 `on_disabled_hover_text` 给理由，
                        //   不把鼠标放上去就看不到——用户只看到一个点不动的框，
                        //   不知道为什么。
                        // * 只画勾选框、**不画"当前电平"**。`serialport` 只提供写、不
                        //   提供读（见 [`crate::app::ComTool::rts`]），驱动回读不到引脚，
                        //   界面就绝不能假装知道自己把线拉到了哪里。措辞也照这个来：
                        //   勾选框表达"我们请求的电平"。
                        // * 打开端口**不会**顺手拉线，所以这两个勾选框的初值恒为未勾选
                        //   （见 [`crate::app::ComTool::open_connection`]）：上电瞬间把
                        //   DTR 拉高会把不少开发板直接拽进复位。
                        //
                        // 另外：流控选「硬件」时，本驱动在 Windows 上取
                        // `RtsControl::Enable`（`serialport` 的内部行为），RTS 由驱动
                        // 负责保持。此时手动勾选未必是线上的最终电平——这正是上面那句
                        // "请求"的含义。
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.line_state_header));
                            let mut rts = self.rts;
                            let mut dtr = self.dtr;
                            let rts_response = ui.add_enabled(
                                lines_controllable,
                                egui::Checkbox::new(&mut rts, "RTS"),
                            );
                            let dtr_response = ui.add_enabled(
                                lines_controllable,
                                egui::Checkbox::new(&mut dtr, "DTR"),
                            );

                            // 先把 `changed()` 取出来：`on_hover_text` 会**消费**
                            // `Response`（它不是 `Copy`），链式调用之后再问就晚了。
                            let rts_changed = rts_response.changed();
                            let dtr_changed = dtr_response.changed();

                            // 悬停提示只管"勾选框的含义"这一件事（勾的是请求电平、
                            // 不是测量值）。**为什么不能操作**改由下面那行常显小字说，
                            // 所以这里不再挂 `on_disabled_hover_text`——同一件事写两处，
                            // 迟早一处改了一处没改。
                            rts_response.on_hover_text(s.line_state_hover);
                            dtr_response.on_hover_text(s.line_state_hover);

                            // 一次只改一根：`None` 表示"那一根不动"（见
                            // [`crate::app::ComTool::apply_line_state`]）。
                            if rts_changed {
                                self.apply_line_state(Some(rts), None);
                            }
                            if dtr_changed {
                                self.apply_line_state(None, Some(dtr));
                            }
                        });

                        // 不能操作的原因**常显**在这两个勾选框下面。这一行与上面
                        // `add_enabled(lines_controllable, …)` 读的是**同一个变量**，
                        // 两处不可能说两套话；判据一旦退化（例如有人改回
                        // `is_connected`），这一行会同进同出，测试据此就能抓到。
                        if !lines_controllable {
                            ui.weak(s.line_state_hint);
                        }
                    } else {
                        // 网络参数：地址 + 端口。串口那一整组（端口名 / 波特率 /
                        // 数据位 / 停止位 / 校验 / 流控）在这里**整段不画**——
                        // 留着它们会让人以为对 TCP 连接也有影响。
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.remote_host_field));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.net_remote_host)
                                    .hint_text(s.remote_host_hint)
                                    .desired_width(150.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized([70.0, 20.0], egui::Label::new(s.remote_port_field));
                            ui.add(
                                egui::DragValue::new(&mut self.net_remote_port)
                                    .range(0..=u16::MAX)
                                    .speed(1.0),
                            );
                        });
                        // 本地绑定端口只对 UDP 有意义：TCP 的本地端口由系统在
                        // connect 时挑，用户改不了。
                        if kind == ConnectionKind::Udp {
                            ui.horizontal(|ui| {
                                ui.add_sized([70.0, 20.0], egui::Label::new(s.local_port_field));
                                ui.add(
                                    egui::DragValue::new(&mut self.net_local_port)
                                        .range(0..=u16::MAX)
                                        .speed(1.0),
                                );
                                ui.weak(s.local_port_hint);
                            });
                        }
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    // —— 再往上：清空日志 / 导出日志 ——
                    ui.horizontal(|ui| {
                        if crate::button::standard(ui, s.clear_log).clicked() {
                            action.clear_requested = true;
                        }
                        if crate::button::standard(ui, s.export_log).clicked() {
                            action.export_requested = true;
                        }
                    });

                    ui.add_space(6.0);

                    egui::CollapsingHeader::new(s.file_log)
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label = if is_logging {
                                    s.stop_logging
                                } else {
                                    s.start_logging
                                };
                                // 底色跟**按下去会做什么**走（与连接按钮同一条规则）：
                                // 还没记录时按下去是"开始"，属主操作 ⇒ 绿；已经在记录时
                                // 按下去是"停止"，不是发起动作 ⇒ 退回中性灰。
                                // 既不给"停止"上红色——本项目刻意没有红色档（见
                                // `crate::button` 的模块文档），也不让它继续绿着，
                                // 否则"绿的停止键"会让人以为按下去还会开始。
                                let toggle = if is_logging {
                                    crate::button::standard(ui, label)
                                } else {
                                    crate::button::primary(ui, label)
                                };
                                if toggle.clicked() {
                                    action.log_toggle_requested = true;
                                }
                                if crate::button::standard(ui, s.pick_dir).clicked() {
                                    action.pick_log_dir_requested = true;
                                }
                            });

                            if is_logging {
                                ui.label(fill(s.logged_bytes, &[&log_written]));
                                let name = std::path::Path::new(&log_path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| log_path.clone());
                                // 只显示文件名：完整路径太长会把侧边栏撑开，
                                // 悬停时再给出完整路径。
                                ui.label(name).on_hover_text(&log_path);
                            } else {
                                ui.weak(s.not_logging);
                            }

                            ui.weak(fill(s.output_dir_line, &[&log_dir]));
                        });
                });
            });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按钮宽度的判据**区间**——用来从形状列表里认出"那个矩形是连接按钮"的尺寸
    /// 指纹，不是实现里的限制（实现见 `CONNECT_BUTTON_MIN_WIDTH` 与
    /// `CONNECT_BUTTON_RIGHT_MARGIN`）。
    ///
    /// * 下界 130 与实现的 `CONNECT_BUTTON_MIN_WIDTH` 对齐，且必须**远大于**右侧那条
    ///   32 宽的收起按钮，否则"按宽度认出按钮"这个指纹就白写了；
    /// * 上界 200 是"还没退回全宽"的护栏：2026-09-24 之前按钮吃满整行（侧边栏默认
    ///   280 宽，扣掉面板留白约 264），用户实机目视后要求收窄。只留下界的话，
    ///   哪天又变回全宽不会被发现。
    ///
    /// 两个数字都用字面量写死、**不引用实现常量**：引用了就变成"实现改小、判据跟着
    /// 变松"，那是空转。
    const BUTTON_WIDTH_FLOOR: f32 = 130.0;
    const BUTTON_WIDTH_CEILING: f32 = 200.0;

    /// 一次绘制里取出的填充矩形：`(矩形, 填充色)`。
    type PaintedRect = (egui::Rect, Color32);

    /// 一次绘制里取出的圆形：`(圆心, 半径, 填充色, 描边宽, 描边色)`。
    ///
    /// 抽出别名是因为裸元组当返回类型会触发 clippy 的 `type_complexity`——
    /// 本项目要求"0 新增警告"。
    type PaintedCircle = (egui::Pos2, f32, Color32, f32, Color32);

    /// 一次绘制里**第一段文字**的字号。
    ///
    /// 按钮里只有一段文字（文案），所以"第一段"就是它。取不到时返回 `None`，
    /// 由调用方决定怎么报错——这里不 panic，是为了让调用方的错误消息能带上
    /// 更具体的上下文。
    fn painted_font_size(out: &egui::FullOutput) -> Option<f32> {
        out.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::epaint::Shape::Text(text) => text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.font_id.size),
            _ => None,
        })
    }

    /// 便捷入口：把一对 `(baud_rate, baud_custom)` 过一遍 `apply_baud_pick`。
    fn pick(start: u32, custom: bool, pick: BaudPick) -> (u32, bool) {
        let mut baud_rate = start;
        let mut baud_custom = custom;
        apply_baud_pick(&mut baud_rate, &mut baud_custom, pick);
        (baud_rate, baud_custom)
    }

    /// 语义 1：点列表档位 → 记下该值、退出自定义档。
    #[test]
    fn listed_pick_clears_custom_flag() {
        // 从自定义档点回列表：值被换成列表项，标记落回 false。
        assert_eq!(pick(250_000, true, BaudPick::Listed(9600)), (9600, false));
        // 本来就在列表档上点另一个档位：只换值。
        assert_eq!(
            pick(115_200, false, BaudPick::Listed(9_600)),
            (9_600, false)
        );
    }

    /// 语义 2：点「自定义」且本来就是自定义档 → 什么都不做。
    ///
    /// 这条是防回归的重点：若实现写成"点自定义一律塞种子值"，用户手输的 250000
    /// 会被自己那一次点击覆盖掉。
    #[test]
    fn custom_pick_when_already_custom_keeps_typed_value() {
        // 手输一个列表里没有的值后再点一次「自定义」，值必须原样保留。
        assert_eq!(pick(74_880, true, BaudPick::Custom), (74_880, true));
        // 即使那个手输的值**恰好**在列表里（比如 9600），也要保留——
        // 这正是"用独立 bool 记录选中档位"而不是从值推导的理由。
        assert_eq!(pick(9_600, true, BaudPick::Custom), (9_600, true));
    }

    /// 语义 3a：点「自定义」且当前值在列表里 → 换成 `BAUD_CUSTOM_SEED`。
    ///
    /// 用户选自定义的意图就是"要输一个列表里没有的值"，把他刚选的那个留着他得先删
    /// 几个字符。顺便钉住种子值本身**不在**列表里——否则这个"省事"就是假的。
    #[test]
    fn custom_pick_when_value_is_listed_reseeds() {
        assert_eq!(
            pick(115_200, false, BaudPick::Custom),
            (BAUD_CUSTOM_SEED, true)
        );
        assert!(!BAUD_RATES.contains(&BAUD_CUSTOM_SEED));
        assert!((CUSTOM_BAUD_MIN..=CUSTOM_BAUD_MAX).contains(&BAUD_CUSTOM_SEED));
    }

    /// 语义 3b：点「自定义」且当前值本来就不在列表里 → 保留不动。
    ///
    /// 典型来源是从旧存档读回来的值（存档里只有 `baud_rate`，没有档位信息）。
    #[test]
    fn custom_pick_when_value_is_unlisted_preserves_it() {
        assert_eq!(pick(74_880, false, BaudPick::Custom), (74_880, true));
        assert!(!BAUD_RATES.contains(&74_880));
    }

    /// 不变式：任何一次点击之后，`baud_rate` 都是一个能提交给驱动的真实值，
    /// 绝不会是"哨兵"。这就是当初删掉 `BAUD_CUSTOM` 哨兵常量、改用 bool 的原因。
    #[test]
    fn baud_rate_is_always_a_real_value() {
        for start in [9_600, 115_200, 74_880, BAUD_CUSTOM_SEED] {
            for custom in [false, true] {
                for p in [BaudPick::Listed(9_600), BaudPick::Custom] {
                    let (rate, _) = pick(start, custom, p);
                    assert!(
                        (CUSTOM_BAUD_MIN..=CUSTOM_BAUD_MAX).contains(&rate),
                        "起点 {start} / custom={custom} / {p:?} 之后得到越界的 {rate}"
                    );
                }
            }
        }
    }

    // ——— 以下三条锁定 2026-09-24 的界面重构（接收统计→状态栏、可用串口→串口信息）———

    /// 把侧边栏画一遍，取出**所有真实落笔的文字**。
    ///
    /// 走 `Context::run_ui`（`__run_test_ui` 会把输出丢掉），并显式清 `textures_delta`
    /// 以免 drop 时 panic 造成假红。
    fn painted_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let mut open = true;
            app.show_sidebar(ui, &mut open);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }
        out.textures_delta.clear();
        texts
    }

    /// **接收/发送字节数必须已经从侧边栏消失**——它们搬去了底部状态栏。
    ///
    /// 这是重构的"减法"一半，最容易被漏掉：只在新位置加读数、旧位置忘了删，
    /// 结果是同一个数字在窗口里出现两次，用户不知道该信哪个。
    ///
    /// 判据是"侧边栏画出的文字里不含这两个带数字的标签"。用 `fill` 拼出带真实
    /// 数值的完整串再查——比"查是否包含『接收字节数』"更严：万一有人把标签留下、
    /// 只是不再填数字，前一种查法抓不到，这一种能。
    #[test]
    fn sidebar_no_longer_shows_the_byte_counters() {
        let mut app = ComTool::default();
        app.receive_text.push_str("hello");
        app.tx_bytes = 1234;

        let s = crate::app::Language::Zh.strings();
        let rx = crate::strings::fill(s.rx_bytes_label, &[&app.receive_text.len()]);
        let tx = crate::strings::fill(s.tx_bytes_label, &[&app.tx_bytes]);

        let painted = painted_texts(&mut app);
        assert!(
            !painted.contains(&rx),
            "侧边栏仍在画接收字节数 {rx:?}——它应当只在底部状态栏出现。实际：{painted:?}"
        );
        assert!(
            !painted.contains(&tx),
            "侧边栏仍在画发送字节数 {tx:?}——它应当只在底部状态栏出现。实际：{painted:?}"
        );
    }

    /// **可用串口列表必须画在「串口信息」里**，不再有独立的「可用串口」折叠头。
    ///
    /// 判据分两半：
    /// 1. 端口名（例如 `COM3`）确实被画了出来——证明列表并进来了，没被顺手删掉；
    /// 2. 旧的独立折叠头文案（`s.available_ports`，"可用串口"）**不再**作为标题出现。
    ///
    /// 第 2 点半带上一个坑：`s.available_ports` 现在仍被用作串口信息区里的小标题
    /// （`ui.weak(...)`），所以不能直接断言"整个界面不含这四个字"。改成断言
    /// **折叠头文案集合**里不含它——`CollapsingHeader` 的标题也是画出来的文字，
    /// 但它旁边会紧跟展开/收起，无法直接区分。因此换个更可靠的判据：端口名出现，
    /// 且**串口信息标题出现**、**旧的"接收统计"标题不出现**（后者已整条删除）。
    #[test]
    fn available_ports_render_inside_the_serial_info_section() {
        let mut app = ComTool {
            ports: vec!["COM3".to_string(), "COM7".to_string()],
            selected_port: "COM3".to_string(),
            ..ComTool::default()
        };

        let s = crate::app::Language::Zh.strings();
        let painted = painted_texts(&mut app);

        // 1) 端口名画出来了（列表并进来了）
        assert!(
            painted.iter().any(|t| t == "COM7"),
            "可用串口列表里的 COM7 没画出来——合并时可能把列表整个丢了。实际：{painted:?}"
        );
        // 2) 串口信息标题在（它俩现在同一个折叠区）
        assert!(
            painted.iter().any(|t| t == s.serial_info),
            "串口信息标题没画出来。实际：{painted:?}"
        );
        // 3) 旧的「接收统计」整条已删除，不该再出现
        assert!(
            !painted.iter().any(|t| t == "接收统计"),
            "侧边栏仍在画已删除的「接收统计」标题。实际：{painted:?}"
        );
    }

    /// **显示模式 / 时间戳前缀**应当留在侧边栏（并入串口信息），而不是被一起搬走。
    ///
    /// 这两行是"当前设置"的描述，不是累计读数。重构时最容易一刀切"凡是原先在
    /// 接收统计里的都搬去状态栏"，把这两行也带走——那会让状态栏塞进设置描述。
    #[test]
    fn sidebar_keeps_display_mode_and_timestamp_lines() {
        let mut app = ComTool::default();
        let s = crate::app::Language::Zh.strings();
        let painted = painted_texts(&mut app);

        let mode_prefix = s.display_mode_line.split("{}").next().unwrap_or("");
        let ts_prefix = s.timestamp_prefix_line.split("{}").next().unwrap_or("");
        assert!(
            painted.iter().any(|t| t.starts_with(mode_prefix)),
            "侧边栏没画「显示模式」行（前缀 {mode_prefix:?}）。实际：{painted:?}"
        );
        assert!(
            painted.iter().any(|t| t.starts_with(ts_prefix)),
            "侧边栏没画「时间戳前缀」行（前缀 {ts_prefix:?}）。实际：{painted:?}"
        );
    }

    // ——— 以下六条锁定 2026-09-24 的连接按钮重做（自绘、全宽、三态配色）———

    /// 一次无头绘制取出的所有可断言素材。
    ///
    /// **合并成一次绘制**是必须的：按钮的矩形和提示文字的位置要互相比对，
    /// 若分两次 `run_ui` 取，比较的就不是同一帧——主题、画布尺寸、绕行布局
    /// 任何一处不同，两次的落点都会差几个点，"在按钮下方"这类判据就成了假的。
    struct PaintedFrame {
        /// 连接按钮的矩形（按尺寸指纹认出，见 [`painted_frame`]）。
        button: egui::Rect,
        /// 所有填充矩形：`(矩形, 填充色)`。
        rects: Vec<PaintedRect>,
        /// 所有文字块：`(左上角, 文本)`。
        texts: Vec<(egui::Pos2, String)>,
    }

    /// 把侧边栏画一遍，取回 [`PaintedFrame`]。
    ///
    /// 刻意**不收圆形**：这里没有任何断言用得上它，返回一堆"看起来在被检查、
    /// 实际没人看"的数据只会让人误以为覆盖到了状态点——状态点的判据在
    /// [`painted_button_variant`] 那条路上。
    ///
    /// 按钮靠**尺寸指纹**认出来：`Shape` 列表里没有"这是按钮"这种标签，
    /// 只能用外观反推。两条判据缺一不可——**高度恰好是 `CONNECT_BUTTON_HEIGHT`
    /// 且宽度落在 `BUTTON_WIDTH_FLOOR`..=`BUTTON_WIDTH_CEILING` 之间**：
    ///
    /// * 只按高度找会抓到右上角那个 `>>` 收起按钮（`min_size(vec2(32.0, 32.0))`，
    ///   同样 32 高，但只有 32 宽、够不到下界）——这是本测试第一版踩到的坑；
    /// * 只按宽度找会抓到状态栏/面板背景那类整幅矩形（它们高度不是 32）。
    ///
    /// 真出错时的 panic 会把所有矩形打出来，便于对照。
    fn painted_frame(app: &mut ComTool, theme: egui::ThemePreference) -> PaintedFrame {
        let ctx = egui::Context::default();
        // 主题要先在 context 上设好：`show_sidebar` 不负责下发主题（那是
        // `ComTool::ui` 里 `apply_theme` 的活），所以测试得自己把 context 摆成目标主题。
        ctx.set_theme(theme);
        // 屏幕尺寸显式给：不给的话 `Panel::right` 拿到的可用宽度会退化，
        // "全宽"就无从判定。
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let mut open = true;
            app.show_sidebar(ui, &mut open);
        });

        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
                egui::epaint::Shape::Text(text) => {
                    texts.push((text.pos, text.galley.text().to_owned()));
                }
                _ => {}
            }
        }
        out.textures_delta.clear();

        let button = rects
            .iter()
            .map(|(rect, _)| *rect)
            .find(|rect| {
                (rect.height() - CONNECT_BUTTON_HEIGHT).abs() < 0.5
                    && (BUTTON_WIDTH_FLOOR..=BUTTON_WIDTH_CEILING).contains(&rect.width())
            })
            .unwrap_or_else(|| {
                panic!(
                    "侧边栏里没有「高 {CONNECT_BUTTON_HEIGHT} 且宽在 {BUTTON_WIDTH_FLOOR}..=\
                     {BUTTON_WIDTH_CEILING} 之间」的填充矩形——连接按钮没画自己的背景，\
                     或者宽度又变回吃满整行了。实际矩形：{:?}",
                    rects.iter().map(|(rect, _)| *rect).collect::<Vec<_>>()
                )
            });
        PaintedFrame {
            button,
            rects,
            texts,
        }
    }

    /// 单独把某一档的按钮画一遍，取出 `(底色, 圆形列表, 文字)`。
    ///
    /// 直接驱动 [`draw_connect_button`]（它是自由函数），不走整条侧边栏：
    /// `show_sidebar` 造不出"已连接"那一档——那需要一条**真就绪**的通道替身，
    /// 而 `ComTool::default()` 的 `channel` 是 `None`（⇒ `opening()` 为真）。
    fn painted_button_variant(
        tone: ConnectButtonTone,
        label: &str,
        dark: bool,
    ) -> (Color32, Vec<PaintedCircle>, Vec<String>) {
        let ctx = egui::Context::default();
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let mut out = ctx.run_ui(Default::default(), |ui| {
            draw_connect_button(ui, tone, label);
        });

        let mut fill = None;
        let mut circles = Vec::new();
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::epaint::Shape::Rect(rect)
                    if (rect.rect.height() - CONNECT_BUTTON_HEIGHT).abs() < 0.5 =>
                {
                    fill = Some(rect.fill);
                }
                egui::epaint::Shape::Circle(circle) => circles.push((
                    circle.center,
                    circle.radius,
                    circle.fill,
                    circle.stroke.width,
                    circle.stroke.color,
                )),
                egui::epaint::Shape::Text(text) => {
                    texts.push(text.galley.text().to_owned());
                }
                _ => {}
            }
        }
        out.textures_delta.clear();

        (fill.expect("按钮没画背景矩形"), circles, texts)
    }

    /// 三个动作档的映射：**没连上就谈不上"正在打开"**。
    #[test]
    fn connect_button_tone_covers_all_three_states() {
        assert_eq!(
            connect_button_tone(false, false),
            ConnectButtonTone::Idle,
            "未连接必须是「可打开」档"
        );
        assert_eq!(
            connect_button_tone(false, true),
            ConnectButtonTone::Idle,
            "`is_opening` 只在已连接时才可能为真；没连上就不该画成琥珀"
        );
        assert_eq!(
            connect_button_tone(true, true),
            ConnectButtonTone::Opening,
            "已发起但通道未就绪 ⇒ 琥珀档"
        );
        assert_eq!(
            connect_button_tone(true, false),
            ConnectButtonTone::Connected,
            "已连接 ⇒ 红档"
        );
    }

    /// **「正在打开」时按钮文案必须仍在"关闭"侧**。
    ///
    /// 这一态里 `is_connected` 也为真，两件事合起来才是完整判据 ⇒ 断言成对写。
    /// 若哪天有人把 `is_opening` 当成"还没连上"去选文案，按钮会显示「打开串口」
    /// 而按下去是**关闭**——那正是 F4 修过的"文案与动作相反"。
    #[test]
    fn connect_button_label_stays_on_the_close_side_while_opening() {
        let s = crate::app::Language::Zh.strings();

        // 串口「正在打开」这一态：tone 是琥珀，文案仍是「关闭串口」。
        assert_eq!(connect_button_tone(true, true), ConnectButtonTone::Opening);
        assert_eq!(
            connect_button_label(ConnectionKind::Serial, true, s),
            s.close_port_button,
            "「正在打开」时文案必须仍是「关闭串口」——按下去确实是关闭"
        );

        // 其余五格：三类连接 × 已连接/未连接。
        assert_eq!(
            connect_button_label(ConnectionKind::Serial, false, s),
            s.open_port_button
        );
        assert_eq!(
            connect_button_label(ConnectionKind::TcpClient, true, s),
            s.disconnect_button
        );
        assert_eq!(
            connect_button_label(ConnectionKind::TcpClient, false, s),
            s.connect_button
        );
        assert_eq!(
            connect_button_label(ConnectionKind::Udp, true, s),
            s.disconnect_button
        );
        assert_eq!(
            connect_button_label(ConnectionKind::Udp, false, s),
            s.bind_button
        );
    }

    /// **底色跟"按下去会做什么"走，跟状态栏那套不同义。**
    ///
    /// 方向必须断言**通道大小**，不能只断言"两色不同"——后者会被反色骗过
    /// （把红绿对调它也过）。悬停更亮 / 按下更暗则逐个通道值之和比较，
    /// 深浅两套主题、三个档位都要成立。
    #[test]
    fn connect_button_fill_follows_the_action_not_the_status() {
        fn brightness(color: Color32) -> u32 {
            u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b())
        }

        for dark in [false, true] {
            let idle = connect_button_fill(ConnectButtonTone::Idle, dark, false, false);
            let connected = connect_button_fill(ConnectButtonTone::Connected, dark, false, false);

            assert!(
                idle.g() > idle.r() && idle.g() > idle.b(),
                "dark={dark} 时「可打开」档不偏绿：{idle:?}"
            );
            assert!(
                connected.r() > connected.g() && connected.r() > connected.b(),
                "dark={dark} 时「可关闭」档不偏红：{connected:?}"
            );

            for tone in [
                ConnectButtonTone::Idle,
                ConnectButtonTone::Opening,
                ConnectButtonTone::Connected,
            ] {
                let base = connect_button_fill(tone, dark, false, false);
                let hover = connect_button_fill(tone, dark, true, false);
                let press = connect_button_fill(tone, dark, false, true);

                assert!(
                    brightness(hover) > brightness(base),
                    "dark={dark} / {tone:?}：悬停没有比常态更亮（{base:?} → {hover:?}）"
                );
                assert!(
                    brightness(press) < brightness(base),
                    "dark={dark} / {tone:?}：按下没有比常态更暗（{base:?} → {press:?}）"
                );
                // 按下与悬停同时为真时以按下为准（指针按着不放时也会 hovered）。
                assert_eq!(
                    connect_button_fill(tone, dark, true, true),
                    press,
                    "dark={dark} / {tone:?}：按下 + 悬停时应以按下为准"
                );
            }
        }
    }

    /// **「正在打开」的琥珀与底部状态栏是同色**。
    ///
    /// 这是项目里"同一含义只能有一处颜色定义"的落地：状态栏的圆点与这个按钮
    /// 说的都是"正在打开"，两边各写一个琥珀，迟早会漂移成两个近似色。
    #[test]
    fn connect_button_opening_tone_reuses_the_status_bar_amber() {
        let bar = crate::status_bar::connection_indicator(
            true,
            true,
            ConnectionKind::Serial,
            crate::app::Language::Zh.strings(),
        )
        .color();

        for dark in [false, true] {
            assert_eq!(
                connect_button_fill(ConnectButtonTone::Opening, dark, false, false),
                bar,
                "dark={dark} 时「正在打开」的底色与状态栏琥珀不一致"
            );
        }
    }

    /// **按钮真的接到了侧边栏里，而且是内容宽、固定高度、自己画背景。**
    ///
    /// 这条与上面几条纯函数测试分开写，正是为了防"函数写得对、但界面没调用"——
    /// 断言"侧边栏这一帧里存在一个高 32、宽度落在判据区间里的填充矩形"。
    /// 老实现（`egui::Button` 默认外观）过不了这条：它的底高度≈文字行高、宽度随文案。
    #[test]
    fn connect_button_paints_its_own_background_inside_the_sidebar() {
        let mut app = ComTool::default();
        let frame = painted_frame(&mut app, egui::ThemePreference::Dark);
        let button = frame.button;

        assert!(
            button.width() >= BUTTON_WIDTH_FLOOR,
            "按钮宽 {}，低于下界 {BUTTON_WIDTH_FLOOR}——按钮没画自己的背景",
            button.width()
        );
        assert!(
            button.width() <= BUTTON_WIDTH_CEILING,
            "按钮宽 {}，高于上界 {BUTTON_WIDTH_CEILING}——又变回吃满整行了",
            button.width()
        );
        assert!(
            button.height() >= CONNECT_BUTTON_HEIGHT - 0.5,
            "按钮高 {}，小于 CONNECT_BUTTON_HEIGHT={CONNECT_BUTTON_HEIGHT}",
            button.height()
        );

        let fill = frame
            .rects
            .iter()
            .find(|(rect, _)| *rect == button)
            .map(|(_, fill)| *fill)
            .expect("按钮矩形不在矩形列表里");
        assert_eq!(
            fill,
            connect_button_fill(ConnectButtonTone::Idle, true, false, false),
            "按钮底色不是「可打开」那一档绿——又退回 egui 默认的弱灰底了"
        );
    }

    /// **按钮下方那条提示必须真的在按钮下方。**
    ///
    /// 这是按钮重做带来的**必然布局后果**：原先提示与按钮在同一个 `ui.horizontal`
    /// 里，按钮占满整行之后同行只剩几个字宽、读不成句，所以移到下一行（收窄之后
    /// 右边仍留着边距，这一整句话一样放不下）。
    /// 这条测试钉的就是这个位置关系——若哪天有人把提示塞回按钮右侧（或改成同行
    /// 右对齐），文字的 y 会落到按钮中线附近，`pos.y < button.max.y` 立刻报红。
    #[test]
    fn connect_button_hint_sits_below_the_button() {
        let mut app = ComTool::default();
        // 提示只在"参数没填全"时出现，而 `ComTool::default()` 默认已经选好 COM1
        // （`connection_ready()` 为真）⇒ 先把端口清掉，否则这一帧里根本没有这条提示，
        // 测试就会去断言一个不存在的东西。第一版就是这么写的，报"没画这条提示"。
        app.selected_port.clear();

        let frame = painted_frame(&mut app, egui::ThemePreference::Dark);
        let s = crate::app::Language::Zh.strings();
        let prefix = s.need_port_to_connect.split("{}").next().unwrap_or("");
        assert!(
            !prefix.is_empty(),
            "这条提示的文案以占位符开头，测试取不到可用于匹配的前缀：{:?}",
            s.need_port_to_connect
        );

        let hint = frame
            .texts
            .iter()
            .find(|(_, text)| text.starts_with(prefix))
            .unwrap_or_else(|| {
                panic!(
                    "未选端口时侧边栏没画这条提示：{:?}；实际画了：{:?}",
                    s.need_port_to_connect,
                    frame
                        .texts
                        .iter()
                        .map(|(pos, text)| (pos.y, text.as_str()))
                        .collect::<Vec<_>>()
                )
            });

        assert!(
            hint.0.y >= frame.button.max.y - 0.5,
            "提示「{}」画在 y={:.1}，而按钮底边是 y={:.1}——提示必须移到按钮下方：\
             与按钮同行放不下这一整句话",
            hint.1,
            hint.0.y,
            frame.button.max.y
        );
    }

    /// **状态点：只有"还没连上"是空心圆**，另外两档实心，且墨色与文案同色。
    ///
    /// 判据里的圆必须**落在按钮的垂直中线上**：侧边栏里单选钮也画圆，
    /// 只断言"存在一个半径 4.5 的圆"会被它们骗过（radio 的圆半径也是这个量级）。
    #[test]
    fn connect_button_dot_is_hollow_only_before_connecting() {
        let radius_ok = |r: f32| (r - CONNECT_DOT_DIAMETER / 2.0).abs() < 0.5;

        // 未连接：空心——fill 透明、描边 1.5。
        let (_, idle_circles, _) =
            painted_button_variant(ConnectButtonTone::Idle, "打开串口", true);
        let idle_dot = idle_circles
            .iter()
            .find(|(_, radius, _, _, _)| radius_ok(*radius))
            .expect("未连接时按钮里没有状态点");
        assert_eq!(
            idle_dot.2,
            Color32::TRANSPARENT,
            "未连接的状态点不是空心的（fill 应当透明）：{idle_dot:?}"
        );
        assert!(
            (idle_dot.3 - CONNECT_DOT_STROKE).abs() < 0.01,
            "未连接的状态点描边宽 {} 不是 {CONNECT_DOT_STROKE}",
            idle_dot.3
        );
        assert_eq!(
            idle_dot.4,
            connect_button_text_color(ConnectButtonTone::Idle),
            "未连接的状态点描边色与文案色不一致"
        );

        // 已连接：实心白点。
        let (_, connected_circles, _) =
            painted_button_variant(ConnectButtonTone::Connected, "关闭串口", true);
        let connected_dot = connected_circles
            .iter()
            .find(|(_, radius, _, _, _)| radius_ok(*radius))
            .expect("已连接时按钮里没有状态点");
        assert_eq!(
            connected_dot.2,
            connect_button_text_color(ConnectButtonTone::Connected),
            "已连接的状态点不是实心白点：{connected_dot:?}"
        );

        // 「正在打开」：实心，且是压在琥珀上的深墨色（白字在琥珀上只有约 1.8:1）。
        let (_, opening_circles, _) =
            painted_button_variant(ConnectButtonTone::Opening, "关闭串口", true);
        let opening_dot = opening_circles
            .iter()
            .find(|(_, radius, _, _, _)| radius_ok(*radius))
            .expect("「正在打开」时按钮里没有状态点");
        assert_eq!(
            opening_dot.2,
            connect_button_text_color(ConnectButtonTone::Opening),
            "「正在打开」的状态点不是深墨色：{opening_dot:?}"
        );
        assert_ne!(
            opening_dot.2,
            Color32::WHITE,
            "「正在打开」用了白墨色——压在琥珀底上对比度不足"
        );
    }

    /// **长文案把按钮压到上限为止，但右边缘永远留白、字号永远不低于下界。**
    ///
    /// 德语/俄语的译文远长于中文。`draw_connect_button` 的处理是：宽度先按内容算，
    /// 撞到「可用宽度 − `CONNECT_BUTTON_RIGHT_MARGIN`」就停住，再按比例缩字号（缩到
    /// `CONNECT_TEXT_MIN_SIZE` 为止）。两头都要钉住——只钉"不会无限变宽"会漏掉
    /// "字被缩到看不见"，只钉"字不会太小"会漏掉"按钮顶到面板边上"。
    #[test]
    fn connect_button_keeps_a_right_margin_when_the_label_is_long() {
        const CANVAS_WIDTH: f32 = 280.0;

        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::Dark);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(CANVAS_WIDTH, 200.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            draw_connect_button(
                ui,
                ConnectButtonTone::Idle,
                "Seriellen Anschluss dauerhaft schliessen",
            );
        });

        let mut button = None;
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Rect(rect) = &clipped.shape
                && (rect.rect.height() - CONNECT_BUTTON_HEIGHT).abs() < 0.5
            {
                button = Some(rect.rect);
            }
        }
        let font_size = painted_font_size(&out);
        out.textures_delta.clear();

        let button = button.expect("长文案下按钮没画自己的背景矩形");
        assert!(
            button.width() <= CANVAS_WIDTH - CONNECT_BUTTON_RIGHT_MARGIN + 0.5,
            "按钮宽 {}，顶到了画布右边缘（画布 {CANVAS_WIDTH}，右边距要求 \
             {CONNECT_BUTTON_RIGHT_MARGIN}）",
            button.width()
        );

        let font_size = font_size.expect("长文案下按钮里没画文字");
        // 两个阈值都用**字面量**，理由与上面那条字号测试相同：引用
        // `CONNECT_TEXT_SIZE` / `CONNECT_TEXT_MIN_SIZE` 就变成自证。这一版最初正是
        // 这么写的——注入"把下限压到 6"时它照样通过（实测字号 11.31 大于 6），
        // 是空转验证把它抓出来的。
        assert!(
            font_size < 16.0,
            "长文案没有触发缩小（字号 {font_size}）——这条测试根本没走到被测的那条路上"
        );
        assert!(
            font_size >= 14.0 - 0.01,
            "长文案把字号缩到了 {font_size}，低于下限 14"
        );
    }

    /// **按钮文案字号不许小于 16——而且"比正文大"这个条件本身不算数。**
    ///
    /// 2026-09-24 用户实机目视：「打开窗口这四个字太小了」。根因是这一档原先按
    /// "与 egui 正文档一致"取了 14——而 `TextStyle::Body` 在 0.36.1 里**实测是 13.0**，
    /// 14 只是"比正文大 1 点"，肉眼无差别。
    ///
    /// 判据**写死 16.0**、不引用 `CONNECT_TEXT_SIZE`（引用就变成自证）。这里踩过一次
    /// 空转：第一版写成 `size > body_size`，注入"把字号改回 14"时照样通过，因为
    /// 14 > 13 成立。下限必须对着"用户会不会觉得小"定，不是对着"是否大于某个默认值"。
    #[test]
    fn connect_button_label_is_larger_than_the_body_default() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::Dark);
        // 画布给足宽度，确保走的是"基准字号"那条路，而不是被上限压缩的那条。
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(400.0, 200.0),
            )),
            ..Default::default()
        };
        // 正文的字号**从实际生效的 style 里取**，不写死 14.0：egui 的字号将来若整体
        // 上调，这条判据跟着走；而 `CONNECT_TEXT_SIZE` 一旦被改回去，它立刻报红。
        let mut body_size = 0.0_f32;
        let mut out = ctx.run_ui(input, |ui| {
            body_size = egui::TextStyle::Body.resolve(ui.style()).size;
            draw_connect_button(ui, ConnectButtonTone::Idle, "打开串口");
        });
        let size = painted_font_size(&out).expect("按钮里没画文案");
        out.textures_delta.clear();

        // 判据用**字面量下限**，不是"大于正文"。第一版写成 `size > body_size`，
        // 结果把字号注入改回 14.0 时它照样通过——因为 `Body` 实测只有 13.0，
        // "比正文大"这个条件在用户抱怨的那个字号上**已经成立**。这次空转验证把它
        // 抓了出来：判据的松紧必须对着"用户会不会觉得小"来定，不是对着"是否大于
        // 某个默认值"。
        const MIN_LABEL_SIZE: f32 = 16.0;
        assert!(
            size >= MIN_LABEL_SIZE,
            "按钮文案字号 {size}，小于下限 {MIN_LABEL_SIZE}（正文才 {body_size}）——\
             用户反馈过这几个字太小"
        );
    }

    // ——— 以下一条锁定 2026-09-24 "全界面按钮统一外观" ———

    /// **侧边栏里的命令按钮真的换成了统一外观**（高 28 + 中性灰实心底），
    /// 而不是 egui 默认那块"灰标签"（高约一行字、圆角 2、没有实心底色）。
    ///
    /// 这条与 `button.rs` 里那些纯函数测试分开写，正是为了防"函数写得对、界面没
    /// 调用"——它断言的是**这一帧画面里**真的出现了统一外观的实心块。
    ///
    /// `ComTool::default()` 下画得到的高 28 中性灰按钮有三个：「>> 收起」「清空日志」
    /// 「导出日志」。「开始/停止记录」与「选择目录」在默认收起的「文件日志」折叠头里，
    /// 这一帧画不到；它们的主操作那一颗还是绿的（`primary`），也不在这条判据的范围内。
    #[test]
    fn sidebar_buttons_use_the_shared_button_style() {
        let mut app = ComTool::default();
        let frame = painted_frame(&mut app, egui::ThemePreference::Dark);

        // 深色主题、常态、可用状态下的中性灰——就是 `crate::button::standard` 画的底。
        let expected = crate::button::button_fill(
            crate::button::ButtonTone::Neutral,
            true,
            false,
            false,
            true,
        );
        let matching: Vec<egui::Rect> = frame
            .rects
            .iter()
            .filter(|(rect, fill)| (rect.height() - 28.0).abs() < 0.5 && *fill == expected)
            .map(|(rect, _)| *rect)
            .collect();

        assert!(
            matching.len() >= 3,
            "侧边栏里只有 {} 个「高 28 + 中性灰实心底」的按钮（期望 ≥ 3：「>> 收起」\
             「清空日志」「导出日志」），说明它们没走统一外观。实际矩形：{:?}",
            matching.len(),
            frame
                .rects
                .iter()
                .map(|(rect, fill)| (*rect, *fill))
                .collect::<Vec<_>>()
        );
    }

    // ——— 控制线（RTS / DTR）勾选框的置灰判据（2026-09-27）———

    /// 给侧边栏/参照控件用的画面尺寸。
    ///
    /// 和 [`painted_frame`] 一样显式给屏幕尺寸：不给的话 `Panel::right` 拿到的可用
    /// 尺寸会退化，侧边栏里的内容可能整个落进 `is_rect_visible` 的判否侧 ——
    /// 那样"文字有没有画出来"这类判据就全成了假红。
    fn sidebar_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        }
    }

    /// 画出侧边栏，返回 **DTR 勾选框文字的颜色** 与 **「为什么不能勾」那一行是否画了出来**。
    ///
    /// 每次都用**全新的 `Context`**：egui 0.36 的控件样式要读**上一帧**同 id 的
    /// `WidgetState`（`widget_style::checkbox_style(&classes, state)`），复用同一个
    /// context 连着画「禁用 → 启用」会让后一帧拿到前一帧的状态、颜色滞后一帧。
    /// 每次新建就没有这层交叉污染。
    fn dtr_color_and_hint(app: &mut ComTool) -> (egui::Color32, bool) {
        let ctx = egui::Context::default();
        let hint = app.language.strings().line_state_hint;
        let mut out = ctx.run_ui(sidebar_input(), |ui| {
            let mut open = true;
            app.show_sidebar(ui, &mut open);
        });

        let mut color = None;
        let mut hint_painted = false;
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                let painted = text.galley.text();
                if painted == "DTR" {
                    color = Some(text.fallback_color);
                }
                if painted == hint {
                    hint_painted = true;
                }
            }
        }
        out.textures_delta.clear();

        (
            color.expect("侧边栏这一帧没有画出 DTR 勾选框"),
            hint_painted,
        )
    }

    /// 「启用 / 禁用分别长什么样」的**标尺**：画一对参照勾选框（一个正常、一个
    /// `add_enabled(false, …)`），返回它们文字的前景色。
    ///
    /// **为什么不写死颜色常量**：颜色是 egui 按主题与控件状态算出来的，写死等于把
    /// egui 的实现细节钉进断言——它一升级就假红，而且那也说明不了"这就叫禁用色"。
    /// 拿同一套默认样式里的参照对当标尺，判据就变成"侧边栏那个框画得跟『禁用参照』一样"。
    ///
    /// 颜色落在 `epaint::TextShape::fallback_color` 上（galley 里写
    /// `Color32::PLACEHOLDER` 的地方由它填，见该字段的文档）。实测：启用
    /// `#B4B4B4FF`、禁用 `#5A5A5A80`（禁用态连 alpha 一起被乘了 0.5）。
    fn reference_checkbox_colors() -> (egui::Color32, egui::Color32) {
        let ctx = egui::Context::default();
        let mut enabled = true;
        let mut disabled = false;
        let mut out = ctx.run_ui(sidebar_input(), |ui| {
            ui.add(egui::Checkbox::new(&mut enabled, "REF_ENABLED"));
            ui.add_enabled(false, egui::Checkbox::new(&mut disabled, "REF_DISABLED"));
        });

        let mut enabled_color = None;
        let mut disabled_color = None;
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                match text.galley.text() {
                    "REF_ENABLED" => enabled_color = Some(text.fallback_color),
                    "REF_DISABLED" => disabled_color = Some(text.fallback_color),
                    _ => {}
                }
            }
        }
        out.textures_delta.clear();

        (
            enabled_color.expect("参照勾选框没画出启用那一个"),
            disabled_color.expect("参照勾选框没画出禁用那一个"),
        )
    }

    /// 造一个**串口**应用：`open_connection()` 走真实路径、通道用替身。
    ///
    /// `ready = false` 让替身扮「后台线程已经起来、端口还没打开」（串口的 `OPENING`
    /// 那一档：`connected` 为真、通道未就绪）——这正是本轮收紧判据要治的状态。
    fn serial_app(ready: bool) -> ComTool {
        let (channel, handle) = crate::communicationtrait::fake::FakeChannel::new();
        if !ready {
            // 必须在 `open()` **之前**登记：替身由它决定"打开后是否当场就绪"。
            handle.defer_ready();
        }
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            ..ComTool::default()
        };
        app.open_connection();
        assert!(app.connected, "替身通道应当能打开成功");
        assert_eq!(
            app.channel_is_ready(),
            ready,
            "替身没摆成预期的那一档（ready = {ready}）"
        );
        app
    }

    /// **RTS / DTR 能不能点，看的是"通道就绪"，不是 `connected`。**
    ///
    /// 这是 2026-09-27 收的一处判据不一致：发送闸门看 `channel_is_ready()`，而这两个
    /// 勾选框看的是 `connected`（含义只是"已经发起过一次打开"）。串口在 `open()` 返回后
    /// 到端口真正打开之间会短暂停在前者真、后者假的那一档，那时勾选框可点、命令只能进
    /// 后台队列排队，而勾选框**已经显示成勾上了**——界面在说谎。
    ///
    /// 四条断言各有分工：
    /// ① 参照对本身必须能区分启用与禁用（否则下面的颜色断言全是空转：都拿同一个值比）；
    /// ② 完全没连接 ⇒ 画成禁用态，**且把原因常显出来**（原先只在悬停时给）；
    /// ③ 串口「正在打开」⇒ 同样是禁用态（老判据在这一档是可点的，所以这条会红）；
    /// ④ 通道真的就绪 ⇒ 才是启用态，且那行原因不再出现。
    #[test]
    fn control_line_checkboxes_follow_the_channel_not_the_connected_flag() {
        let (enabled_ref, disabled_ref) = reference_checkbox_colors();
        assert_ne!(
            enabled_ref, disabled_ref,
            "参照的那对勾选框画出了同一个颜色 —— 拿它们做判据的断言会全部变成空转"
        );

        // ② 完全没连接：默认应用没有通道。
        let mut idle = ComTool::default();
        let (color, hint) = dtr_color_and_hint(&mut idle);
        assert_eq!(
            color, disabled_ref,
            "没连接时 DTR 勾选框没画成禁用态（实测 {color:?}，禁用参照 {disabled_ref:?}）"
        );
        assert!(
            hint,
            "没连接时没有把「为什么不能勾」写出来 —— 用户只看到一个点不动的框"
        );

        // ③ 串口「正在打开」：`connected` 为真、通道未就绪。
        let mut opening = serial_app(false);
        assert!(
            opening.connected,
            "前提：这一档里 `connected` 必须为真 —— 老判据正是看它，所以那时勾选框可点"
        );
        assert!(opening.opening(), "前提：应当是「正在打开」那一档");
        let (color, hint) = dtr_color_and_hint(&mut opening);
        assert_eq!(
            color, disabled_ref,
            "串口正在打开时 DTR 勾选框可点了 —— 命令只能进后台队列排队，界面却在说已勾上"
        );
        assert!(hint, "「正在打开」那一档也没写出为什么不能勾");

        // ④ 通道就绪：这才该是可点的。
        let mut ready = serial_app(true);
        let (color, hint) = dtr_color_and_hint(&mut ready);
        assert_eq!(
            color, enabled_ref,
            "通道已就绪，DTR 勾选框却还是灰的（实测 {color:?}，启用参照 {enabled_ref:?}）"
        );
        assert!(!hint, "通道已就绪，却还在显示「连接后才能手动控制」");
    }
}
