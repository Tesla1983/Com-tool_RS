use crate::communicationtrait::{CommunicationEvent, CommunicationManager};
use crate::serial_impl::portsettings::{BAUD_RATES, BaudRate, PortSettings};
use crate::serial_impl::serial::{SerialCommunication, scan_ports};
use eframe::egui;
use egui::{Button, Ui};
use rfd::FileDialog;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const SCROLL_AREA_ID: &str = "receive_scroll";
//const RIGHT_PANEL_ID: &str = "right_panel";

fn utf8_complete_prefix_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    if len == 0 {
        return 0;
    }

    // 找到尾部连续 continuation 字节（0b10xxxxxx）的起点 start：
    // 循环退出时 bytes[start-1] 是 lead 字节（若 start > 0）
    let mut start = len;
    while start > 0 && (bytes[start - 1] & 0xC0) == 0x80 {
        start -= 1;
    }
    if start == len {
        // 尾部不是未完成的序列
        return len;
    }
    if start == 0 {
        // 整段都是 continuation 字节（异常流）：无法回溯 lead，交给上层按非法字节标记
        return len;
    }
    let lead = bytes[start - 1];
    let width = if lead < 0x80 {
        1
    } else if lead >= 0xC0 && lead <= 0xDF {
        2
    } else if lead >= 0xE0 && lead <= 0xEF {
        3
    } else if lead >= 0xF0 && lead <= 0xF7 {
        4
    } else {
        1 // 非法起始字节，当作单字节处理（会被标记为 [XX]）
    };
    let seq_start = start - 1; // 未完成序列的 lead 字节位置
    if seq_start + width <= len {
        len
    } else {
        seq_start // 从 lead 字节起整个保留，等下一个数据块补齐
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

/// 界面语言（方案 A：目前仅作为设置项占位，尚未接入文案切换）
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Language {
    Zh,
    En,
}
/// ASCII 视图：\r \n \t 等常见控制字符的转义显示颜色（蓝）
const ASCII_CTRL_COLOR: egui::Color32 = egui::Color32::from_rgb(36, 114, 200);
/// ASCII 视图：其余不可见字节（其它控制字符 / 高位字节）的显示颜色（灰）
const ASCII_NONPRINT_COLOR: egui::Color32 = egui::Color32::from_rgb(128, 128, 128);

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

/// 把一段原始字节逐字节解析为 ASCII 可视化 spans：
/// - 0x20..=0x7E：可打印字符，原样显示（默认色）
/// - \r \n \t：显示为 "\\r" "\\n" "\\t"（蓝色；其中 \n 额外追加真实换行以保持行结构）
/// - 其它控制字符（0x00..=0x1F、0x7F）：显示为 "[0D]" 形式（灰色）
/// - 高位字节（0x80..=0xFF）：显示为 "\xFF" 形式（灰色）
fn ascii_spans_from_bytes(bytes: &[u8]) -> Vec<ColoredSpan> {
    fn flush(spans: &mut Vec<ColoredSpan>, normal: &mut String) {
        if !normal.is_empty() {
            spans.push(ColoredSpan {
                text: std::mem::take(normal),
                style: AnsiStyle::default(),
            });
        }
    }

    let mut spans = Vec::new();
    let mut normal = String::new();
    for &b in bytes {
        match b {
            0x20..=0x7E => normal.push(b as char),
            b'\n' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\n\n".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            b'\r' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\r".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            b'\t' => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: "\\t".to_string(),
                    style: AnsiStyle {
                        fg: Some(ASCII_CTRL_COLOR),
                        bold: false,
                    },
                });
            }
            0x00..=0x1F | 0x7F => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: format!("[{b:02X}]"),
                    style: AnsiStyle {
                        fg: Some(ASCII_NONPRINT_COLOR),
                        bold: false,
                    },
                });
            }
            _ => {
                flush(&mut spans, &mut normal);
                spans.push(ColoredSpan {
                    text: format!("\\x{b:02X}"),
                    style: AnsiStyle {
                        fg: Some(ASCII_NONPRINT_COLOR),
                        bold: false,
                    },
                });
            }
        }
    }
    flush(&mut spans, &mut normal);
    spans
}

/// 由一组 ColoredSpan 构造等宽 14px 的 LayoutJob（彩色/ASCII 两种接收视图共用）
fn layout_job_from(spans: &[ColoredSpan], default_color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in spans {
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

/// 绘制接收区滚动视图（可选中、右键复制全部日志），返回是否点击了「复制全部日志」
fn show_receive_scroll(ui: &mut Ui, job: egui::text::LayoutJob) -> bool {
    let mut copy_all = false;
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height(400.0)
        .id_salt(SCROLL_AREA_ID)
        .show(ui, |ui| {
            let response = ui.add(egui::Label::new(job).wrap().selectable(true));
            response.context_menu(|ui| {
                if ui.button("复制全部日志").clicked() {
                    copy_all = true;
                    ui.close();
                }
            });
        });
    copy_all
}

/// 普通文本模式接收区：不折行（Extend）+ 纵/横双向滚动，
/// 等宽 galley 绘制保证盲文 ASCII 图案整行对齐；可选中、右键复制全部日志
fn show_plain_receive(ui: &mut Ui, text: &str) {
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::monospace(14.0),
        egui::Color32::BLACK,
        f32::INFINITY, // 折行宽度 = ∞ → 永不折行
    );
    egui::ScrollArea::both()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height(400.0)
        .id_salt(SCROLL_AREA_ID)
        .show(ui, |ui| {
            let response = ui.add(
                egui::Label::new(job)
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true),
            );
            response.context_menu(|ui| {
                if ui.button("复制全部日志").clicked() {
                    ui.ctx().copy_text(text.to_owned());
                    ui.close();
                }
            });
        });
}

pub struct ComTool {
    // 串口列表
    pub ports: Vec<String>,

    // 当前选择的串口
    pub selected_port: String,

    // 当前波特率
    pub baud_rate: BaudRate,

    // 当前串口参数
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,

    // 是否已经打开串口
    pub connected: bool,

    // 接收区
    pub receive_text: String,

    // 发送区
    pub send_text: String,

    // 串口通信管理器（基于 CommunicationManager trait）
    pub serial: Option<Box<dyn CommunicationManager>>,

    // 串口事件通道（DataReceived / Error / ConnectionClosed）
    pub event_rx: Option<Receiver<CommunicationEvent>>,

    // 上次自动重连尝试时间（用于节流，避免高频重试）
    pub last_reconnect_attempt: Option<std::time::Instant>,

    // 上次成功打开的串口（自动重连时优先用它，避免跳到 COM1）
    pub last_opened_port: Option<String>,

    pub hex_view: bool,
    pub display_spans: Vec<ColoredSpan>,
    pub decoder: AnsiDecoder,
    pub color_view: bool,
    pub sidebar_open: bool,
    pub language: Language,
    pub show_settings: bool,
    pub ascii_view: bool,

    // ASCII 视图的可见化文本 / spans（与 receive_text、display_spans 平行维护）
    pub ascii_text: String,
    pub ascii_spans: Vec<ColoredSpan>,
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
            // 严格解码：合法片段原样输出；非法字节以灰色 [XX] 十六进制标记显示，
            // 不再静默生成 U+FFFD（字体缺字形时会渲染成不可读的 □）
            let bad_style = AnsiStyle {
                fg: Some(ASCII_NONPRINT_COLOR),
                bold: false,
            };
            let mut rest = complete;
            let mut normal = String::new();
            while !rest.is_empty() {
                match std::str::from_utf8(rest) {
                    Ok(s) => {
                        normal.push_str(s);
                        rest = &[];
                    }
                    Err(e) => {
                        let valid = e.valid_up_to();
                        if valid > 0 {
                            normal
                                .push_str(std::str::from_utf8(&rest[..valid]).unwrap_or_default());
                        }
                        // error_len()=None 表示末尾被截断的不完整序列，按剩余全部处理
                        let bad = e.error_len().unwrap_or(rest.len() - valid).max(1);
                        if !normal.is_empty() {
                            spans.push(ColoredSpan {
                                text: std::mem::take(&mut normal),
                                style: self.style,
                            });
                        }
                        let hex: String = rest[valid..valid + bad]
                            .iter()
                            .map(|b| format!("[{b:02X}]"))
                            .collect();
                        spans.push(ColoredSpan {
                            text: hex,
                            style: bad_style,
                        });
                        rest = &rest[valid + bad..];
                    }
                }
            }
            if !normal.is_empty() {
                spans.push(ColoredSpan {
                    text: normal,
                    style: self.style,
                });
            }
        }
        self.pending_utf8 = tail.to_vec();
    }

    fn parse_ansi(&mut self, seq: &[u8]) {
        if seq.len() < 3 || seq[0] != 0x1B || seq[1] != b'[' {
            return;
        }
        let final_byte = *seq.last().unwrap();
        if final_byte != b'm' {
            return;
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

    pub fn auto_reconnect(&mut self) {
        // 只有串口曾经打开过、现在断开时才尝试自动重连
        if self.connected || self.serial.is_none() {
            return;
        }

        let now = std::time::Instant::now();
        let need_retry = self
            .last_reconnect_attempt
            .map_or(true, |t| now.duration_since(t).as_millis() >= 500);
        if !need_retry {
            return;
        }
        self.last_reconnect_attempt = Some(now);

        let target = self
            .last_opened_port
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.selected_port.clone());
        if target.is_empty() {
            return;
        }
        self.selected_port = target;

        // 只刷新端口列表供 UI 显示，不改变所选端口
        self.ports = scan_ports();

        self.open_port();
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

        // 首次打开时创建串口通信管理器
        if self.serial.is_none() {
            self.serial = Some(Box::new(SerialCommunication::new()));
        }
        let serial = self.serial.as_mut().unwrap();

        // 把当前 UI 选择的参数应用到串口
        let settings = PortSettings {
            port_name: self.selected_port.clone(),
            baudrate: self.baud_rate.numeric_repr,
            data_bits: self.data_bits,
            stop_bits: self.stop_bits,
            parity: self.parity,
            flowcontrol: self.flow_control,
        };
        if let Err(e) = serial.update_settings(&settings) {
            self.receive_text
                .push_str(&format!("设置串口参数失败: {}\n", e));
            return;
        }

        let (event_tx, event_rx) = mpsc::channel::<CommunicationEvent>();
        if let Err(e) = serial.start(event_tx) {
            self.receive_text
                .push_str(&format!("打开串口失败: {}\n", e));
            return;
        }
        self.event_rx = Some(event_rx);
        self.connected = true;

        self.last_opened_port = Some(self.selected_port.clone());

        let open_msg = format!("已打开 {}\n", self.selected_port);
        self.receive_text.push_str(&open_msg);

        self.display_spans.push(ColoredSpan {
            text: open_msg,
            style: AnsiStyle {
                fg: Some(egui::Color32::from_rgb(13, 188, 121)), // 绿
                bold: false,
            },
        });
    }

    pub fn close_port(&mut self) {
        if let Some(serial) = &mut self.serial {
            let _ = serial.stop();
        }

        self.serial = None;
        self.event_rx = None;
        self.last_opened_port = None;
        self.last_reconnect_attempt = None;
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

        if let Some(serial) = &mut self.serial {
            if let Err(e) = serial.send_data(data) {
                self.receive_text.push_str(&format!("发送失败: {}\n", e));
            }
        }

        self.send_text.clear();
    }

    pub fn receive_data(&mut self) {
        let mut received = Vec::new();

        if let Some(rx) = &self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    CommunicationEvent::DataReceived(data) => {
                        received.extend_from_slice(&data);
                    }
                    CommunicationEvent::Error(msg) => {
                        self.receive_text.push_str(&format!("串口错误: {}\n", msg));
                        self.connected = false;
                    }
                    CommunicationEvent::ConnectionClosed => {
                        self.receive_text.push_str("串口连接已关闭\n");
                        self.connected = false;
                    }
                }
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
            } else if self.ascii_view {
                // ASCII 模式：逐字节解码，控制字符/高位字节转义为可见形式
                let spans = ascii_spans_from_bytes(&received);
                for span in spans {
                    self.ascii_text.push_str(&span.text);
                    self.receive_text.push_str(&span.text);
                    self.ascii_spans.push(span);
                }
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
        layout_job_from(&self.display_spans, default_color)
    }

    /// ASCII 视图的 LayoutJob（与彩色视图同款等宽 14px 样式）
    pub fn build_ascii_layout(&self, default_color: egui::Color32) -> egui::text::LayoutJob {
        layout_job_from(&self.ascii_spans, default_color)
    }

    pub fn render_color_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 把 display_spans（含一次性开关状态提示）构造成带颜色的 LayoutJob
        let job = self.build_layout(egui::Color32::BLACK);

        // ③ 在函数内部绘制滚动区，替换掉主界面里默认的滚动区
        if show_receive_scroll(ui, job) {
            ui.ctx().copy_text(self.receive_text.clone());
        }
    }

    /// ASCII 视图：逐字节解码显示，控制字符/高位字节转义可见化
    pub fn render_ascii_receive(&mut self, ui: &mut Ui) {
        // ① 先刷新接收数据（没有新数据时不会追加，重复调用安全）
        self.receive_data();

        // ② 由 ascii_spans 构造 LayoutJob
        let job = self.build_ascii_layout(egui::Color32::BLACK);

        // ③ 绘制滚动区（样式与彩色模式一致）
        if show_receive_scroll(ui, job) {
            ui.ctx().copy_text(self.receive_text.clone());
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
            baud_rate: BAUD_RATES[11],
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            connected: false,
            receive_text: String::new(),
            send_text: String::new(),
            serial: None,
            event_rx: None,
            last_reconnect_attempt: None,
            last_opened_port: None,
            hex_view: false,
            display_spans: Vec::new(),
            decoder: AnsiDecoder::default(),
            color_view: false,
            sidebar_open: true,
            language: Language::Zh,
            show_settings: false,
            ascii_view: false,
            ascii_text: String::new(),
            ascii_spans: Vec::new(),
        }
    }
}

impl eframe::App for ComTool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 检查串口接收数据
        self.receive_data();

        // 串口意外断开时自动重连（节流 500ms）
        self.auto_reconnect();

        // 不断刷新 GUI
        ui.ctx().request_repaint_after(Duration::from_millis(30));

        let port_name = self.selected_port.clone();
        let is_connected = self.connected;
        let current_baud = self.baud_rate;
        let hex_mode = self.hex_view;
        let color_mode = self.color_view;
        let rx_bytes = self.receive_text.len();
        let port_list = self.ports.clone();

        let mut sidebar_open = self.sidebar_open;
        let mut clear_clicked = false;
        let mut export_clicked = false;
        let mut close_clicked = false;

        egui::Panel::right("right_sidebar")
            .resizable(true)
            .default_size(280.0)
            .min_size(200.0)
            .max_size(600.0)
            .show_collapsible(ui, &mut sidebar_open, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("侧边栏");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn = egui::Button::new(egui::RichText::new(">>").size(20.0))
                            .min_size(egui::vec2(32.0, 32.0));
                        if ui.add(btn).on_hover_text("收起").clicked() {
                            close_clicked = true;
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
                        ui.label(format!("波特率: {}", current_baud.string_repr));
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
                            .selected_text(self.baud_rate.string_repr)
                            .show_ui(ui, |ui| {
                                for baud in &BAUD_RATES {
                                    ui.selectable_value(
                                        &mut self.baud_rate,
                                        *baud,
                                        baud.string_repr,
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
                            clear_clicked = true;
                        }
                        if ui.button("导出日志").clicked() {
                            export_clicked = true;
                        }
                    });
                });
            });

        // 回写状态 + 执行延迟动作（放在闭包外，避免与 &mut sidebar_open 借用冲突）
        self.sidebar_open = if close_clicked { false } else { sidebar_open };
        if clear_clicked {
            self.receive_text.clear();
            self.display_spans.clear();
            self.ascii_text.clear();
            self.ascii_spans.clear();
            self.decoder.reset();
            let id = ui.make_persistent_id(SCROLL_AREA_ID);
            egui::scroll_area::State::default().store(ui.ctx(), id);
        }
        if export_clicked {
            self.export_receive_text();
        }

        egui::CentralPanel::default().show(ui, |ui| {
            //ui.heading("");
            if self.show_settings {
                egui::Window::new("设置")
                    .collapsible(false)
                    .resizable(false)
                    .open(&mut self.show_settings)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(20.0, 20.0))
                    .show(ui.ctx(), |ui| {
                        ui.label("language:");
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(match self.language {
                                Language::Zh => "中文",
                                Language::En => "English",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.language, Language::Zh, "中文");
                                ui.selectable_value(&mut self.language, Language::En, "English");
                            });

                        ui.separator();
                        ui.weak("（语言选项暂未接入界面文案，仅作占位）");
                    });
            }

            ui.separator();

            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.sidebar_open, "☰ 侧边栏");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ 设置").clicked() {
                        self.show_settings = true;
                    }
                });
            });

            ui.separator();

            // 接收设置：文本/HEX 用 checkbox 表达（底层互斥，因为收到数据只能按一种格式解析）
            let mut show_hex = self.hex_view;
            let mut show_text = !self.hex_view;
            ui.horizontal(|ui| {
                ui.label("接收设置:");
                if ui.checkbox(&mut show_hex, "HEX").changed() && show_hex {
                    show_text = false; // 勾 HEX → 取消文本
                }
                if ui.checkbox(&mut show_text, "文本").changed() && show_text {
                    show_hex = false; // 勾文本 → 取消 HEX
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.color_view, "颜色");
                    ui.checkbox(&mut self.ascii_view, "字节查看");
                });
            });
            self.hex_view = show_hex; // 文本 = !hex_view；两个都取消时退回文本

            // 接收区
            if self.color_view {
                self.render_color_receive(ui);
            } else if self.ascii_view {
                // ASCII 模式：逐字节解码，控制字符/高位字节转义为可见形式
                self.render_ascii_receive(ui);
            } else {
                // 普通文本模式：不折行 + 纵/横双向滚动 + 可选中 + 右键复制全部
                show_plain_receive(ui, &self.receive_text);
            }

            ui.separator();

            // 发送区
            ui.label("发送区:");

            let send_edit_response = ui.add(
                egui::TextEdit::multiline(&mut self.send_text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(4)
                    // 与接收区同款等宽字体：ASCII 图案在输入时即可按空格对齐预览
                    .font(egui::FontId::monospace(14.0))
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        egui::Key::Enter,
                    )),
            );

            let enter_to_send = send_edit_response.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

            let mut do_send = false;
            let mut do_clear = false;
            ui.horizontal(|ui| {
                if ui.button("发送").clicked() {
                    do_send = true;
                }
                if ui.button("清空发送").clicked() {
                    do_clear = true;
                }
            });
            if enter_to_send {
                do_send = true;
            }
            if do_send {
                self.send_data();
            }
            if do_clear {
                self.send_text.clear();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把解析结果拼成纯文本，便于断言
    fn joined(bytes: &[u8]) -> String {
        ascii_spans_from_bytes(bytes)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn printable_passes_through() {
        assert_eq!(joined(b"Hello"), "Hello");
    }

    #[test]
    fn cr_lf_tab_escaped_and_newline_breaks_line() {
        // \r \n \t 显示为转义；\n 额外带一个真实换行
        assert_eq!(joined(b"A\r\nB\tC"), "A\\r\\n\nB\\tC");
    }

    #[test]
    fn control_and_high_bytes_are_visible() {
        assert_eq!(joined(&[0x00, 0x7F, 0xFF]), "[00][7F]\\xFF");
    }

    #[test]
    fn utf8_multibyte_is_shown_per_byte() {
        // “中” = E4 B8 AD → 按字节显示为三个 \xHH
        assert_eq!(joined("中".as_bytes()), "\\xE4\\xB8\\xAD");
    }

    /// 文本模式解码：把 AnsiDecoder 输出拼成纯文本
    fn decoded(bytes: &[u8]) -> String {
        AnsiDecoder::default()
            .feed(bytes)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn decoder_invalid_bytes_become_visible_hex_markers() {
        // 非法字节 0xFF 不再生成 U+FFFD，而是显示为灰色 [FF] 标记
        assert_eq!(decoded(b"AB\xFFCD"), "AB[FF]CD");
        // 非法序列 E0 80（第二字节越界）逐字节标记后继续正常解码
        assert_eq!(decoded(&[0xE0, 0x80, b'A']), "[E0][80]A");
        // 末尾不完整的序列先保留，等下一块数据补齐，不提前输出标记
        assert_eq!(decoded(&[0xE4, 0xB8]), "");
    }

    #[test]
    fn decoder_keeps_split_multibyte_across_feeds() {
        // “中” 被拆到两次 feed：第一次不完整 → 无输出；第二次补齐 → 完整字符
        let mut d = AnsiDecoder::default();
        assert!(d.feed(&[0xE4, 0xB8]).is_empty());
        assert_eq!(
            d.feed(&[0xAD, b'!'])
                .into_iter()
                .map(|s| s.text)
                .collect::<String>(),
            "中!"
        );
    }

    #[test]
    fn decoder_valid_utf8_passes_through() {
        assert_eq!(decoded("ok中⣿".as_bytes()), "ok中⣿");
    }
}
