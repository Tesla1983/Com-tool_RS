use crate::serial::scan_ports;
use eframe::egui;
use egui::WidgetType::ScrollBar;
use egui::{Label, Layout, Ui};
use rfd::FileDialog;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use std::{default, thread};

const SCROLL_AREA_ID: &str = "receive_scroll";

fn utf8_complete_prefix_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    if len == 0 {
        return 0;
    }

    let mut i = len;
    while i > 0 && (bytes[i - 1] & 0xC0) == 0x80 {
        i -= 1;
    }
    if i == len {
        // 尾部不是未完成的序列
        return len;
    }
    let lead = bytes[i];
    let width = if lead < 0x80 {
        1
    } else if lead >= 0xC0 && lead <= 0xDF {
        2
    } else if lead >= 0xE0 && lead <= 0xEF {
        3
    } else if lead >= 0xF0 && lead <= 0xF7 {
        4
    } else {
        1 // 非法起始字节，当作单字节处理（会被替换为 �）
    };
    if i + width <= len {
        len
    } else {
        i // 保留不完整的多字节序列尾部
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AnsiStyle {
    pub fg: Option<egui::Color32>,
    pub bold: bool,
}

impl Default for AnsiStyle {
    fn default() -> Self {
        Self {
            fg: None,
            bold: false,
        }
    }
}

pub struct ColoredSpan {
    pub text: String,
    pub style: AnsiStyle,
}

fn basic_color(n: u8) -> egui::Color32 {
    match n {
        30 => egui::Color32::from_rgb(0, 0, 0),
        31 => egui::Color32::from_rgb(205, 49, 49),
        32 => egui::Color32::from_rgb(13, 188, 121),
        33 => egui::Color32::from_rgb(229, 229, 16),
        34 => egui::Color32::from_rgb(36, 114, 200),
        35 => egui::Color32::from_rgb(188, 63, 188),
        36 => egui::Color32::from_rgb(17, 168, 205),
        37 => egui::Color32::from_rgb(229, 229, 229),
        _ => egui::Color32::WHITE,
    }
}

fn bright_color(n: u8) -> egui::Color32 {
    match n {
        90 => egui::Color32::from_rgb(128, 128, 128),
        91 => egui::Color32::from_rgb(255, 0, 0),
        92 => egui::Color32::from_rgb(0, 255, 0),
        93 => egui::Color32::from_rgb(255, 255, 0),
        94 => egui::Color32::from_rgb(92, 92, 255),
        95 => egui::Color32::from_rgb(255, 0, 255),
        96 => egui::Color32::from_rgb(0, 255, 255),
        97 => egui::Color32::from_rgb(255, 255, 255),
        _ => egui::Color32::WHITE,
    }
}

fn color256(n: u8) -> egui::Color32 {
    if n < 16 {
        if n < 8 {
            basic_color(30 + n)
        } else {
            bright_color(90 + (n - 8))
        }
    } else if n < 232 {
        let i = n as u32 - 16;
        let conv = |x: u32| -> u8 { if x == 0 { 0 } else { (55 + x * 40) as u8 } };
        egui::Color32::from_rgb(conv(i / 36), conv((i % 36) / 6), conv(i % 6))
    } else {
        let g = (8 + (n as u32 - 232) * 10) as u8;
        egui::Color32::from_rgb(g, g, g)
    }
}

pub struct AnsiDecoder {
    pending_ansi: Vec<u8>,
    pending_utf8: Vec<u8>,
    pub style: AnsiStyle,
}

pub struct ComTool {
    // 串口列表
    pub ports: Vec<String>,

    // 当前选择的串口
    pub selected_port: String,

    // 当前波特率
    pub baud_rate: u32,

    // 是否已经打开串口
    pub connected: bool,

    // 接收区
    pub receive_text: String,

    // 发送区
    pub send_text: String,

    // 接收线程 -> GUI
    pub rx: Option<Receiver<Vec<u8>>>,

    // GUI -> 串口发送线程
    pub tx: Option<Sender<Vec<u8>>>,

    // 接收线程停止信号
    pub stop_tx: Option<Sender<()>>,

    // 发送线程停止信号
    pub tx_stop: Option<Sender<()>>,

    pub rx_handle: Option<thread::JoinHandle<()>>,
    pub tx_handle: Option<thread::JoinHandle<()>>,
    pub hex_view: bool,
    pub display_spans: Vec<ColoredSpan>,
    pub decoder: AnsiDecoder,
    pub color_view: bool,
}

impl Default for AnsiDecoder {
    fn default() -> Self {
        Self {
            pending_ansi: Vec::new(),
            pending_utf8: Vec::new(),
            style: AnsiStyle::default(),
        }
    }
}

impl AnsiDecoder {
    pub fn reset(&mut self) {
        self.pending_ansi.clear();
        self.pending_utf8.clear();
        self.style = AnsiStyle::default();
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ColoredSpan> {
        let mut spans = Vec::new();
        for &b in bytes {
            if self.pending_ansi.is_empty() {
                if b == 0x1B {
                    // 遇到 ESC：先输出已积累的普通文本，再开始收集转义序列
                    self.flush_utf8(&mut spans);
                    self.pending_ansi.push(b);
                } else {
                    self.pending_utf8.push(b);
                }
            } else {
                // 正在收集 ESC 序列
                self.pending_ansi.push(b);
                if self.pending_ansi.len() == 2 {
                    // ESC 后的第一个字节：必须是 '[' 才构成 CSI 序列，否则忽略整段
                    if b != b'[' {
                        self.pending_ansi.clear();
                    }
                } else if (0x40..=0x7E).contains(&b) {
                    // CSI 参数之后的终结字节（'m' 为 SGR，其余忽略）
                    let seq = std::mem::take(&mut self.pending_ansi);
                    self.parse_ansi(&seq);
                }
            }
        }
        self.flush_utf8(&mut spans);
        spans
    }

    fn flush_utf8(&mut self, spans: &mut Vec<ColoredSpan>) {
        if self.pending_utf8.is_empty() {
            return;
        }
        let valid_len = utf8_complete_prefix_len(&self.pending_utf8);
        let (complete, tail) = self.pending_utf8.split_at(valid_len);
        if !complete.is_empty() {
            let text = String::from_utf8_lossy(complete).into_owned();
            spans.push(ColoredSpan {
                text,
                style: self.style,
            });
        }
        self.pending_utf8 = tail.to_vec();
    }

    fn parse_ansi(&mut self, seq: &[u8]) {
        if seq.len() < 3 || seq[0] != 0x1B || seq[1] != b'[' {
            return;
        }
        let final_byte = *seq.last().unwrap();
        if final_byte != b'm' {
            return; // 非 SGR（光标移动、清屏等），忽略
        }
        let params_str = String::from_utf8_lossy(&seq[2..seq.len() - 1]);
        let params: Vec<u32> = params_str
            .split(';')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect();
        self.apply_sgr(&params);
    }

    fn apply_sgr(&mut self, params: &[u32]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.style = AnsiStyle::default(),
                1 => self.style.bold = true,
                2 | 22 => self.style.bold = false,
                39 => self.style.fg = None,
                30..=37 => self.style.fg = Some(basic_color(params[i] as u8)),
                90..=97 => self.style.fg = Some(bright_color(params[i] as u8)),
                38 => {
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        self.style.fg = Some(color256(params[i + 2] as u8));
                        i += 3;
                        continue;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        self.style.fg = Some(egui::Color32::from_rgb(
                            params[i + 2] as u8,
                            params[i + 3] as u8,
                            params[i + 4] as u8,
                        ));
                        i += 5;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

impl ComTool {
    pub fn refresh_ports(&mut self) {
        self.ports = scan_ports();

        if !self.ports.contains(&self.selected_port) {
            self.selected_port = self.ports.first().cloned().unwrap_or_default();
        }
    }

    pub fn export_receive_text(&mut self) {
        if self.receive_text.is_empty() {
            self.receive_text.push_str("没有可导出的数据\n");
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .set_title("导出接收数据.txt")
            .add_filter("文本文件", &["txt"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &self.receive_text.as_bytes()) {
            Ok(_) => {
                self.receive_text
                    .push_str(&format!("已导出到: {}\n", path.display()));
            }
            Err(e) => {
                self.receive_text
                    .push_str(&format!("导出接收数据失败: {}\n", e));
            }
        }
    }

    pub fn open_port(&mut self) {
        if self.selected_port.is_empty() {
            return;
        }

        let port = match serialport::new(&self.selected_port, self.baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
        {
            Ok(port) => port,

            Err(e) => {
                self.receive_text
                    .push_str(&format!("打开串口失败: {}\n", e));
                return;
            }
        };

        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>();
        let (send_tx, send_rx) = mpsc::channel::<Vec<u8>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (tx_stop_tx, tx_stop_rx) = mpsc::channel::<()>();

        let mut read_port = match port.try_clone() {
            Ok(port) => port,
            Err(e) => {
                self.receive_text
                    .push_str(&format!("复制串口失败: {}\n", e));
                return;
            }
        };

        // 接收线程
        let rx_handle = thread::spawn(move || {
            let mut buffer = [0u8; 1024];

            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }

                match read_port.read(&mut buffer) {
                    Ok(size) if size > 0 => {
                        let data = buffer[..size].to_vec();

                        if data_tx.send(data).is_err() {
                            break;
                        }
                    }

                    Ok(_) => {}

                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}

                    Err(_) => {
                        break;
                    }
                }
            }
        });

        let tx_handle = thread::spawn(move || {
            let mut write_port = port;

            loop {
                if tx_stop_rx.try_recv().is_ok() {
                    break;
                }

                match send_rx.try_recv() {
                    Ok(data) => {
                        if let Err(e) = write_port.write_all(&data) {
                            eprintln!("串口发送失败: {}", e);
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
        });

        self.rx = Some(data_rx);
        self.tx = Some(send_tx);
        self.stop_tx = Some(stop_tx);
        self.tx_stop = Some(tx_stop_tx);
        self.rx_handle = Some(rx_handle);
        self.tx_handle = Some(tx_handle);
        self.connected = true;

        let open_msg = format!("已打开 {}\n", self.selected_port);
        self.receive_text.push_str(&open_msg);
        // 一次性追加一条彩色提示到彩色日志（不随渲染每帧重复）
        self.display_spans.push(ColoredSpan {
            text: open_msg,
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
    }

    pub fn close_port(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(tx_stop) = self.tx_stop.take() {
            let _ = tx_stop.send(());
        }

        // 等待发送/接收线程退出，确保读写句柄真正释放
        if let Some(handle) = self.tx_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.rx_handle.take() {
            let _ = handle.join();
        }

        self.rx = None;
        self.tx = None;
        self.connected = false;

        self.receive_text.push_str("串口已关闭\n");
        // 一次性追加一条彩色提示到彩色日志（不随渲染每帧重复）
        self.display_spans.push(ColoredSpan {
            text: "串口已关闭\n".to_string(),
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(205, 49, 49)), // 红
                bold: false,
            },
        });
    }

    pub fn send_data(&mut self) {
        if !self.connected {
            return;
        }

        let data = self.send_text.as_bytes().to_vec();

        if let Some(tx) = &self.tx {
            let _ = tx.send(data);
        }

        self.send_text.clear();
    }

    pub fn receive_data(&mut self) {
        let mut received = Vec::new();

        if let Some(rx) = &self.rx {
            while let Ok(data) = rx.try_recv() {
                received.extend_from_slice(&data);
            }
        }

        if !received.is_empty() {
            if self.hex_view {
                // HEX 模式：直接按字节显示，不做 ANSI/UTF-8 处理
                let mut line = String::new();
                for b in &received {
                    line.push_str(&format!("{:02X} ", b));
                }
                line.push('\n');
                self.receive_text.push_str(&line);
                self.display_spans.push(ColoredSpan {
                    text: line,
                    style: AnsiStyle::default(),
                });
            } else {
                // 文本模式：解析 ANSI 颜色并跨包拼接 UTF-8
                let spans = self.decoder.feed(&received);
                for span in spans {
                    self.receive_text.push_str(&span.text);
                    self.display_spans.push(span);
                }
            }
        }
    }

    
    pub fn build_layout(&self, default_color: egui::Color32) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        for span in &self.display_spans {
            job.append(
                span.text.as_str(),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(14.0),
                    color: span.style.fg.unwrap_or(default_color),
                    ..Default::default()
                },
            );
        }
        job
    }

    pub fn render_color_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 把 display_spans（含一次性开关状态提示）构造成带颜色的 LayoutJob
        let job = self.build_layout(egui::Color32::WHITE);

        // ③ 在函数内部绘制滚动区，替换掉主界面里默认的滚动区
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(400.0)
            .id_salt(SCROLL_AREA_ID)
            .show(ui, |ui| {
                ui.add(egui::Label::new(job).wrap());
            });
    }
}

impl Default for ComTool {
    fn default() -> Self {
        let ports = scan_ports();

        let selected_port = ports.first().cloned().unwrap_or_default();

        Self {
            ports,
            selected_port,
            baud_rate: 115200,
            connected: false,
            receive_text: String::new(),
            send_text: String::new(),
            rx: None,
            tx: None,
            stop_tx: None,
            tx_stop: None,
            rx_handle: None,
            tx_handle: None,
            hex_view: false,
            display_spans: Vec::new(),
            decoder: AnsiDecoder::default(),
            color_view: false,
        }
    }
}

impl eframe::App for ComTool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 检查串口接收数据
        self.receive_data();

        // 不断刷新 GUI
        ui.ctx().request_repaint_after(Duration::from_millis(30));

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("");

            ui.separator();

            // 串口设置
            ui.horizontal(|ui| {
                ui.label("串口:");

                egui::ComboBox::from_id_salt("port")
                    .selected_text(if self.selected_port.is_empty() {
                        "请选择串口"
                    } else {
                        &self.selected_port
                    })
                    .show_ui(ui, |ui| {
                        for port in &self.ports {
                            ui.selectable_value(&mut self.selected_port, port.clone(), port);
                        }
                    });

                if ui.button("刷新").clicked() {
                    self.refresh_ports();
                }

                ui.label("波特率:");

                egui::ComboBox::from_id_salt("baud")
                    .selected_text(self.baud_rate.to_string())
                    .show_ui(ui, |ui| {
                        for baud in [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600] {
                            ui.selectable_value(&mut self.baud_rate, baud, baud.to_string());
                        }
                    });

                if !self.connected {
                    if ui.button("打开串口").clicked() {
                        self.open_port();
                    }
                } else if ui.button("关闭串口").clicked() {
                    self.close_port();
                }
            });

            ui.separator();

            // 接收区
            ui.horizontal(|ui| {
                ui.label("接收设置:");
                ui.selectable_value(&mut self.hex_view, true, "HEX");
                ui.selectable_value(&mut self.hex_view, false, "文本");
                ui.horizontal(|ui| {
                ui.checkbox(&mut self.color_view, "颜色");
                });
            });

            // 接收区
            if self.color_view {
                // 彩色模式：滚动区在函数内部自行绘制
                self.render_color_receive(ui);
            } else {
                // 普通文本模式：保持默认滚动区 + TextEdit
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(400.0)
                    .id_salt(SCROLL_AREA_ID)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.receive_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(20)
                                .interactive(false),
                        );
                    });
            }

            ui.horizontal(|ui| {
                if ui.button("清空").clicked() {
                    self.receive_text.clear();
                    self.display_spans.clear();
                    self.decoder.reset();

                    // 重置滚动区位置（load 是读取，重置需要用 default 再 store）
                    let id = ui.make_persistent_id(SCROLL_AREA_ID);
                    egui::scroll_area::State::default().store(ui.ctx(), id);
                }
                if ui.button("导出").clicked() {
                    self.export_receive_text();
                }
            });

            ui.separator();

            // 发送区
            ui.label("发送:");

            ui.add(
                egui::TextEdit::multiline(&mut self.send_text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(4),
            );

            ui.horizontal(|ui| {
                let send_clicked = ui.button("发送").clicked();

                if send_clicked {
                    self.send_data();
                }

                if ui.button("清空发送").clicked() {
                    self.send_text.clear();
                }
            });
        });
    }
}

/*
to do list

1. 完善串口刷新功能,单片机重启的时候能够持续输出日志

2. 完善串口发送功能,能够发送字符串和十六进制数据

3. 完善串口接收功能,能够接收字符串和十六进制数据

4. 完善串口设置功能,能够设置串口波特率,数据位,停止位,校验位

*/

/*
开发笔记：快速将函数左移对齐（即减少缩进）的快捷键是 **Shift + Tab** 。

*/