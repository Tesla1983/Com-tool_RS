#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::app::ComTool;

use eframe::egui;
use egui::epaint::text;
use egui::{Ui, output};
use egui_chinese_font;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::iter::Filter;
use std::str::Chars;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

mod app;
mod communicationtrait;
mod config;
mod serial_impl;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        ..Default::default()
    };

    eframe::run_native(
        "ComTool-RS",
        options,
        Box::new(|cc| {
            let _ = egui_chinese_font::setup_chinese_fonts(&cc.egui_ctx);

            Ok(Box::new(ComTool::default()))
        }),
    )
}

/*
to do list

1. 完善串口刷新功能,单片机重启的时候能够持续输出日志（已完成）

2. 完善串口发送功能,能够发送字符串和十六进制数据

3. 完善串口设置功能,能够设置,数据位,停止位,校验位

*/

/*
开发笔记：
快速将函数左移对齐（即减少缩进）的快捷键是 **Shift + Tab** 。
使用Markdown文件预览功能,先打开Markdown文件，然后点击右上角的文本编辑器，展开下拉菜单，选择相应的功能即可。
使用CodeGeeXCommit Message功能，先暂存 → 才能生成 Commit Message → 提交
使用  Ctrl + G  可以快速跳转到指定行数

⣿⣿⣿⣿⣿⠟⠋⠄⠄⠄⠄⠄⠄⠄⢁⠈⢻⢿⣿⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⠃⠄⠄⠄⠄⠄⠄⠄⠄⠄⠄⠄⠈⡀⠭⢿⣿⣿⣿⣿
⣿⣿⣿⣿⡟⠄⢀⣾⣿⣿⣿⣷⣶⣿⣷⣶⣶⡆⠄⠄⠄⣿⣿⣿⣿
⣿⣿⣿⣿⡇⢀⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⠄⠄⢸⣿⣿⣿⣿
⣿⣿⣿⣿⣇⣼⣿⣿⠿⠶⠙⣿⡟⠡⣴⣿⣽⣿⣧⠄⢸⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣾⣿⣿⣟⣭⣾⣿⣷⣶⣶⣴⣶⣿⣿⢄⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣿⣿⡟⣩⣿⣿⣿⡏⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣹⡋⠘⠷⣦⣀⣠⡶⠁⠈⠁⠄⣿⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣍⠃⣴⣶⡔⠒⠄⣠⢀⠄⠄⠄⡨⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣿⣦⡘⠿⣷⣿⠿⠟⠃⠄⠄⣠⡇⠈⠻⣿⣿⣿⣿
⣿⣿⣿⣿⡿⠟⠋⢁⣷⣠⠄⠄⠄⠄⣀⣠⣾⡟⠄⠄⠄⠄⠉⠙⠻
⡿⠟⠋⠁⠄⠄⠄⢸⣿⣿⡯⢓⣴⣾⣿⣿⡟⠄⠄⠄⠄⠄⠄⠄⠄
⠄⠄⠄⠄⠄⠄⠄⣿⡟⣷⠄⠹⣿⣿⣿⡿⠁⠄⠄⠄⠄⠄⠄⠄⠄

*/
