use crate::serial::scan_ports;
use eframe::egui;
use egui::Ui;
use rfd::FileDialog;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

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
    pub buffer: Vec<u8>,
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

        self.receive_text
            .push_str(&format!("已打开 {}\n", self.selected_port));
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

    pub fn strip_ansi_escape_sequences(input: &str) -> String {
        let mut output = String::new();
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii_alphabetic() {
                            chars.next();
                            break;
                        }
                        chars.next();
                    }
                }
            } else {
                output.push(c);
            }
        }
        output
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
                // HEX 模式：直接按字节显示，不做 UTF-8 处理
                let mut line = String::new();
                for b in &received {
                    line.push_str(&format!("{:02X} ", b));
                }
                self.receive_text.push_str(&line);
                self.receive_text.push('\n');
                self.buffer.clear();
            } else {
                // 文本模式：跨包拼接 UTF-8，非法字节替换为 �，绝不卡死
                self.buffer.extend_from_slice(&received);

                let valid_len = utf8_complete_prefix_len(&self.buffer);
                let (complete, tail) = self.buffer.split_at(valid_len);

                if !complete.is_empty() {
                    let lossy = String::from_utf8_lossy(complete);
                    let filtered_text = Self::strip_ansi_escape_sequences(&lossy);
                    self.receive_text.push_str(&filtered_text);
                }

                self.buffer = tail.to_vec();
            }
        }
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
            buffer: Vec::new(),
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
                ui.label("接收:");
                ui.selectable_value(&mut self.hex_view, true, "HEX");
                ui.selectable_value(&mut self.hex_view, false, "文本");
            });

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(400.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.receive_text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20)
                            .interactive(false),
                    );
                });

            ui.horizontal(|ui| {
                if ui.button("清空").clicked() {
                    self.receive_text.clear();
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
