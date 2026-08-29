#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::app::ComTool;
use crate::serial::scan_ports;
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
mod config;
mod serial;

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
