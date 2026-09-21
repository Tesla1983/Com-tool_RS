#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::app::ComTool;

use eframe::egui;
use egui::{FontData, FontDefinitions, FontFamily};

mod app;
mod communicationtrait;
mod config;
mod panels;
mod receive;
mod serial_impl;
mod sidebar;

/// 应用标题，同时也是 `eframe` 用来定位存档目录的 app id。
///
/// 只在这里定义一次：`run_native` 与「信息」窗口都引用它，避免两处字符串漂移后
/// 出现"关于窗口显示的存档路径不是真正的存档路径"。
pub const APP_TITLE: &str = "ComTool-RS";

/// 配置 egui 字体回退链。
///
/// Monospace 族：Hack（真等宽，ASCII 图案逐列对齐）→ 中文字体（汉字）→ Segoe UI Symbol（盲文点阵 ⣿⠿⠀，U+2800 区）
/// Proportional 族：中文字体（保持界面原观感）→ Segoe UI Symbol
///
/// 任一系统字体缺失时自动跳过，不崩溃。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // 中文字体（微软雅黑优先，失败则试其它常见路径）
    let chinese_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
    ];
    if let Some(data) = chinese_paths.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts
            .font_data
            .insert("chinese".into(), FontData::from_owned(data).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("chinese".into());
        // 注意：Monospace 族把中文字体放在 Hack 之后，保证 ASCII/空格用真等宽字形
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("chinese".into());
    }

    // 盲文点阵回退：微软雅黑与 Hack 均不含 U+2800-28FF，缺字形会显示 □
    let symbol_paths = [
        r"C:\Windows\Fonts\seguisym.ttf", // Segoe UI Symbol（完整覆盖 Braille Patterns）
    ];
    if let Some(data) = symbol_paths.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts
            .font_data
            .insert("symbol".into(), FontData::from_owned(data).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("symbol".into());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("symbol".into());
    }

    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);

            Ok(Box::new(ComTool::new(cc)))
        }),
    )
}

/*
to do list



4.
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
