//! 右侧边栏：串口信息回显、接收统计、连接参数设置与日志操作入口。
//!
//! 绘制函数只负责画控件并把「用户按了哪个按钮」记录下来，真正的状态变更与
//! 副作用（关闭串口、清空日志、弹出导出对话框）全部交回调用方执行。这样
//! 派发逻辑集中在 `app.rs` 一处，侧边栏不会在渲染过程中偷偷改状态。

use crate::app::{ComTool, ConnectionKind};
use crate::serial_impl::BAUD_RATES;
use crate::strings::{Strings, fill, timestamp_prefix_label};
use egui::Ui;
use serialport::{DataBits, FlowControl, Parity, StopBits};

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
        let current_baud = self.baud_rate;
        let hex_mode = self.hex_view;
        let color_mode = self.color_view;
        let timestamp_mode = self.timestamp_prefix;
        let rx_bytes = self.receive_text.len();

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
        let status_text = match (is_connected, kind) {
            (true, ConnectionKind::Udp) => s.status_bound,
            (true, _) => s.status_connected,
            (false, ConnectionKind::Udp) => s.status_unbound,
            (false, _) => s.status_disconnected,
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
                });

                egui::CollapsingHeader::new(s.rx_stats)
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(fill(s.rx_bytes_label, &[&rx_bytes]));
                        ui.label(fill(s.display_mode_line, &[&display_mode_text]));
                        ui.label(fill(s.timestamp_prefix_line, &[&timestamp_text]));
                    });

                // 「可用串口」只对串口有意义——网络模式下这一栏永远是空的，
                // 留着只会让人以为"没扫到可用端口"。
                if kind.uses_serial_settings() {
                    egui::CollapsingHeader::new(s.available_ports).show(ui, |ui| {
                        if port_list.is_empty() {
                            ui.weak(s.none_placeholder);
                        } else {
                            for p in &port_list {
                                ui.label(p);
                            }
                        }
                    });
                }

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

                        // 参数没填全时按钮点下去会"没反应"（`connection_ready`
                        // 拦下），所以要把原因摆出来。
                        if !is_connected && !self.connection_ready() {
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
                            egui::ComboBox::from_id_salt("baud")
                                .selected_text(self.baud_rate.to_string())
                                .show_ui(ui, |ui| {
                                    for baud in &BAUD_RATES {
                                        ui.selectable_value(
                                            &mut self.baud_rate,
                                            *baud,
                                            baud.to_string(),
                                        );
                                    }
                                });
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
