//! 右侧边栏：串口信息回显、接收统计、连接参数设置与日志操作入口。
//!
//! 绘制函数只负责画控件并把「用户按了哪个按钮」记录下来，真正的状态变更与
//! 副作用（关闭串口、清空日志、弹出导出对话框）全部交回调用方执行。这样
//! 派发逻辑集中在 `app.rs` 一处，侧边栏不会在渲染过程中偷偷改状态。

use crate::app::ComTool;
use crate::serial_impl::BAUD_RATES;
use egui::Ui;
use serialport::{DataBits, FlowControl, Parity, StopBits};

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

        // 本帧要展示的只读快照。在绘制前统一取值，之后即便闭包内改了串口
        // 参数，已画出的信息区也仍是本帧开始时的状态，不会前后矛盾。
        let port_name = self.selected_port.clone();
        let is_connected = self.connected;
        let current_baud = self.baud_rate;
        let hex_mode = self.hex_view;
        let color_mode = self.color_view;
        let rx_bytes = self.receive_text.len();
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
                    ui.heading("侧边栏");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn = egui::Button::new(egui::RichText::new(">>").size(20.0))
                            .min_size(egui::vec2(32.0, 32.0));
                        if ui.add(btn).on_hover_text("收起").clicked() {
                            action.close_requested = true;
                        }
                    });
                });
                ui.separator();

                egui::CollapsingHeader::new("串口信息")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "端口: {}",
                            if port_name.is_empty() {
                                "未选择"
                            } else {
                                &port_name
                            }
                        ));
                        ui.label(format!("波特率: {}", current_baud));
                        ui.label(if is_connected {
                            "状态: 已连接"
                        } else {
                            "状态: 未连接"
                        });
                    });

                egui::CollapsingHeader::new("接收统计")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!("接收字节数: {}", rx_bytes));
                        ui.label(format!(
                            "显示模式: {}{}{}",
                            if hex_mode { "HEX" } else { "文本" },
                            if color_mode { " + 颜色" } else { "" },
                            if self.ascii_view {
                                " + 字节查看"
                            } else {
                                ""
                            }
                        ));
                    });

                egui::CollapsingHeader::new("可用串口").show(ui, |ui| {
                    if port_list.is_empty() {
                        ui.weak("（无）");
                    } else {
                        for p in &port_list {
                            ui.label(p);
                        }
                    }
                });

                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new("端口"));
                        egui::ComboBox::from_id_salt("port")
                            .selected_text(if self.selected_port.is_empty() {
                                "请选择端口"
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
                        if ui.button("刷新").clicked() {
                            self.refresh_ports();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new("波特率"));
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

                        let label = if self.connected {
                            "打开串口"
                        } else {
                            "关闭串口"
                        };
                        let button = egui::Button::new(label).selected(self.connected);
                        if ui.add(button).clicked() {
                            if self.connected {
                                self.close_port();
                            } else {
                                self.open_port();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new("数据位"));
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

                        ui.add_sized([70.0, 20.0], egui::Label::new("停止位"));
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
                        ui.add_sized([70.0, 20.0], egui::Label::new("校验"));
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

                        ui.add_sized([70.0, 20.0], egui::Label::new("流控"));
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

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    // —— 再往上：清空日志 / 导出日志 ——
                    ui.horizontal(|ui| {
                        if ui.button("清空日志").clicked() {
                            action.clear_requested = true;
                        }
                        if ui.button("导出日志").clicked() {
                            action.export_requested = true;
                        }
                    });

                    ui.add_space(6.0);

                    egui::CollapsingHeader::new("文件日志")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label = if is_logging {
                                    "停止记录"
                                } else {
                                    "开始记录"
                                };
                                if ui.button(label).clicked() {
                                    action.log_toggle_requested = true;
                                }
                                if ui.button("选择目录").clicked() {
                                    action.pick_log_dir_requested = true;
                                }
                            });

                            if is_logging {
                                ui.label(format!("已记录 {} 字节", log_written));
                                let name = std::path::Path::new(&log_path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| log_path.clone());
                                // 只显示文件名：完整路径太长会把侧边栏撑开，
                                // 悬停时再给出完整路径。
                                ui.label(name).on_hover_text(&log_path);
                            } else {
                                ui.weak("未记录");
                            }

                            ui.weak(format!("输出目录: {log_dir}"));
                        });
                });
            });

        action
    }
}
