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
use egui::Ui;
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
                        let btn = egui::Button::new(egui::RichText::new(">>").size(20.0))
                            .min_size(egui::vec2(32.0, 32.0));
                        if ui.add(btn).on_hover_text(s.collapse).clicked() {
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
                    ui.horizontal(|ui| {
                        let label = if is_connected {
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
                        };
                        let button = egui::Button::new(label).selected(is_connected);
                        if ui.add(button).clicked() {
                            if self.connected {
                                self.close_connection();
                            } else {
                                self.open_connection();
                            }
                        }

                        // 「正在打开」要单独说一句。它与下面那条"参数没填全"是两回事：
                        // 那个是**点下去不会有反应**，这个是**已经点了、正在等**，
                        // 共用一条提示会把两件事说成一件事。
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
                    });
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
                            if ui.button(s.refresh).clicked() {
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
                        // * 未连接时**置灰**。通道没开就没有地方投递命令，"点了没反应"
                        //   不如一眼看出不能点；置灰的理由挂在 `on_disabled_hover_text`
                        //   上，不让用户去猜。
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
                            let rts_response =
                                ui.add_enabled(is_connected, egui::Checkbox::new(&mut rts, "RTS"));
                            let dtr_response =
                                ui.add_enabled(is_connected, egui::Checkbox::new(&mut dtr, "DTR"));

                            // 先把 `changed()` 取出来：`on_hover_text` 会**消费**
                            // `Response`（它不是 `Copy`），链式调用之后再问就晚了。
                            let rts_changed = rts_response.changed();
                            let dtr_changed = dtr_response.changed();

                            // 两种提示各管一段，都挂上就不用在这里写 if：
                            // `on_hover_text` 在控件被禁用时本来就不显示（egui 的定义），
                            // 禁用的解释交给 `on_disabled_hover_text`。
                            rts_response
                                .on_hover_text(s.line_state_hover)
                                .on_disabled_hover_text(s.line_state_hint);
                            dtr_response
                                .on_hover_text(s.line_state_hover)
                                .on_disabled_hover_text(s.line_state_hint);

                            // 一次只改一根：`None` 表示"那一根不动"（见
                            // [`crate::app::ComTool::apply_line_state`]）。
                            if rts_changed {
                                self.apply_line_state(Some(rts), None);
                            }
                            if dtr_changed {
                                self.apply_line_state(None, Some(dtr));
                            }
                        });
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
                        if ui.button(s.clear_log).clicked() {
                            action.clear_requested = true;
                        }
                        if ui.button(s.export_log).clicked() {
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
                                if ui.button(label).clicked() {
                                    action.log_toggle_requested = true;
                                }
                                if ui.button(s.pick_dir).clicked() {
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
}
