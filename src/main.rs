#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::app::ComTool;

use eframe::egui;
use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

mod app;
mod app_icon;
mod chart;
mod communicationtrait;
mod config;
mod log_file;
mod net_impl;
mod panels;
mod receive;
mod send;
mod serial_impl;
mod sidebar;
mod status_bar;
mod strings;

/// 应用标题，同时也是 `eframe` 用来定位存档目录的 app id。
///
/// 只在这里定义一次：`run_native` 与「信息」窗口都引用它，避免两处字符串漂移后
/// 出现"关于窗口显示的存档路径不是真正的存档路径"。
pub const APP_TITLE: &str = "ComTool-RS";

/// 配置 egui 字体回退链。
///
/// Monospace 族：Hack（真等宽，ASCII 图案逐列对齐）→ 中文字体（汉字）→ 拉丁扩展字体（
/// 越南语带调字母、泰文）→ 韩文字体 → Segoe UI Symbol（盲文点阵 ⣿⠿⠀，U+2800 区）
/// Proportional 族：中文字体（保持界面原观感）→ 同上各补充字体 → Segoe UI Symbol
///
/// 挂载顺序即回退优先级：**先命中的字体出字形**。中文字体排在前，是因为界面默认中文；
/// 拉丁扩展字体（tahoma）排在其后，只为补微软雅黑缺失的越南语/泰文字符，不会抢中文的字
/// （雅黑已含全部汉字，前一步就命中了）。
///
/// 任一系统字体缺失时自动跳过，不崩溃。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // 往 Proportional 与 Monospace 两个族的回退链**末尾**各追加一个字体。
    //
    // 抽成闭包是因为下面有四个字体要挂，逐个写两遍 family 太啰嗦，也容易漏一个族
    // （曾踩过：只挂 Proportional 忘了 Monospace，结果接收区的越南语仍是方框）。
    let push = |fonts: &mut FontDefinitions, name: &str, paths: &[&str]| {
        if let Some(data) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert(name.to_owned(), FontData::from_owned(data).into());
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.into());
            }
        }
    };

    // 中文字体（微软雅黑优先，失败则试其它常见路径）。
    // 注意：Monospace 族里它排在 Hack 之后，保证 ASCII/空格用真等宽字形。
    push(
        &mut fonts,
        "chinese",
        &[
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyhbd.ttc",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
        ],
    );

    // 拉丁扩展 / 泰文字体（Tahoma）。
    //
    // 为什么需要它：微软雅黑**缺**越南语那 49 个带双变音调的预组合字符
    // （ề ố ộ ờ ơ ư 等，U+1EA0–U+1EF9 区），也缺**全部**泰文字符。
    // 实测 `tahoma.ttf` 对越南语全字母表与泰文全字符集**均为 0 缺失**，
    // 是同时解决这两种语言的单一字体。缺它时越南语界面会"大部分正常、个别词变方框"，
    // 比整片方框更隐蔽，所以这条挂载是必需项而非可选优化。
    push(&mut fonts, "latin_ext", &[r"C:\Windows\Fonts\tahoma.ttf"]);

    // 韩文字体（Malgun Gothic）。
    //
    // 微软雅黑、宋体、黑体、Segoe UI Symbol **都不含谚文**，缺它则韩语界面整片方框。
    // `malgun.ttf` 是 Windows 自带的韩文字体，实测覆盖谚文全字符集。
    push(&mut fonts, "korean", &[r"C:\Windows\Fonts\malgun.ttf"]);

    // 盲文点阵回退：微软雅黑与 Hack 均不含 U+2800-28FF，缺字形会显示 □。
    push(&mut fonts, "symbol", &[r"C:\Windows\Fonts\seguisym.ttf"]);

    // 注意：这里**没有**彩色 emoji 字体档。状态栏的圆点已经改成用 `Color32` 画的
    // 实心圆（见 `status_bar.rs` 的模块文档），不再依赖任何 emoji 字形——早先挂
    // `seguiemj.ttf` 也没用，因为 egui 只光栅化字体的矢量轮廓，不读 COLR/CPAL
    // 彩色图层，拿到的是那个单色圆。为将来可能出现的 emoji 需求留个位置，
    // 但**不要**为了让圆点有颜色而加回来。

    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        // 窗口与任务栏图标在编译期用代码绘制（见 `app_icon`）。不设置时
        // eframe 会退回它自带的 egui 图标；在 Windows 上 eframe 会把这份
        // RGBA 重新编码成 PNG 再交给 `CreateIconFromResourceEx`。
        viewport: egui::ViewportBuilder::default().with_icon(Arc::new(app_icon::generate())),
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
