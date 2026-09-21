//! 顶部菜单栏与设置 / 信息窗口。
//!
//! 与 [`crate::sidebar`] 同一条约定：绘制函数只负责画控件，并把「用户要求做什么」
//! 记录成 action 结构体，真正的副作用（退出程序、打开窗口）由 `app.rs` 统一派发。
//! 直接把值绑到 `ComTool` 字段上的控件（勾选框、下拉框、数值框）不经过 action——
//! 它们改的是状态，不是副作用。
//!
//! 模块内都是 `impl ComTool` 的固有方法，因此这里没有对外 re-export：方法随类型
//! 可见，不需要（也无法）用 `use` 再引出一次。

mod chart;
mod info;
mod menu_bar;
mod settings;

// 菜单栏里定义的**共享控件**要能被中央区（`app.rs` 的发送区）复用。模块内其余
// 内容都是 `impl ComTool` 的固有方法，随类型可见，不需要在这里再引出一次。
pub(crate) use menu_bar::{send_format_radios, send_line_ending_radios};

#[cfg(test)]
mod tests {
    use crate::app::ComTool;
    use eframe::egui;

    /// 无头渲染冒烟测试：菜单栏、设置窗口、信息窗口都能画出来而不 panic。
    ///
    /// 覆盖的是只有运行期才会暴露的问题——面板嵌套顺序、控件 ID 冲突、布局调用；
    /// 不需要窗口、不需要串口，因此可以直接进 `cargo test`。它**不能**替代肉眼
    /// 确认外观，只能证明"点下去不会炸"。
    #[test]
    fn menu_bar_and_windows_render_without_panic() {
        let mut app = ComTool::default();

        egui::__run_test_ui(|ui| {
            let action = app.show_menu_bar(ui);
            assert!(!action.quit_requested, "刚打开时不该请求退出");
            assert!(!action.info_requested);
            assert!(!action.settings_requested);
        });

        app.show_settings = true;
        app.show_info = true;
        egui::__run_test_ui(|ui| {
            app.show_settings_window(ui);
            app.show_info_window(ui);
        });
    }

    /// 三种接收视图各自的绘制路径都要能跑通（校验 auto_scroll 这条新参数确实
    /// 从 `ComTool` 一路传到了滚动区）。
    #[test]
    fn receive_views_render_without_panic() {
        let mut app = ComTool::default();
        app.receive_text.push_str("hello\n⣿\n");

        egui::__run_test_ui(|ui| {
            crate::receive::show_plain_receive(ui, &app.receive_text, app.auto_scroll);
        });
        egui::__run_test_ui(|ui| {
            app.render_color_receive(ui);
        });
        egui::__run_test_ui(|ui| {
            app.render_ascii_receive(ui);
        });

        // 关掉自动滚动后同样要能画（只是不再强制 stick_to_bottom）
        app.auto_scroll = false;
        egui::__run_test_ui(|ui| {
            crate::receive::show_plain_receive(ui, &app.receive_text, app.auto_scroll);
        });
    }

    /// 侧边栏（含文件日志区块）要能画出来，且不绘制时不得产生任何操作请求。
    #[test]
    fn sidebar_renders_without_panic() {
        let mut app = ComTool::default();

        let mut open = true;
        egui::__run_test_ui(|ui| {
            let action = app.show_sidebar(ui, &mut open);
            assert!(!action.log_toggle_requested, "仅绘制不应触发开始记录");
            assert!(!action.pick_log_dir_requested);
            assert!(!action.clear_requested);
            assert!(!action.export_requested);
        });
        assert!(open, "仅绘制不应改变侧边栏开关");
        assert!(!app.is_logging(), "仅绘制不得开始写日志文件");
    }

    /// 图表面板在三条分支上都能画出来：无数据、有曲线、以及解析有异常时的
    /// 诊断提示行。
    ///
    /// 这条测试的价值在于覆盖 `egui_plot` 本身——它有自己的内存状态与纹理管理，
    /// 出错只会在运行期暴露；无头渲染能在不开窗口、不碰串口的前提下把它跑一遍。
    /// 它证明的仍然只是"画得出来不 panic"，外观与交互手感仍需人工确认。
    #[test]
    fn chart_panel_renders_without_panic() {
        let mut app = ComTool {
            show_chart_panel: true,
            ..ComTool::default()
        };

        // ① 还没有数据：只画标题与"等待数据…"
        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        // ② 四条通道各一个样本（验收要求 ≥4 通道）：此时自动缩放面对的是
        //    零宽度的 x 范围，是最容易让坐标轴计算退化的一种输入
        app.chart.push_bytes(b"1,2,3,4\n", app.chart_max_samples);
        assert_eq!(app.chart.channel_count(), 4);
        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        // ③ 多个样本：正常的曲线
        app.chart
            .push_bytes(b"1.5,2.5,3.5,4.5\n", app.chart_max_samples);
        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        // ④ 有被忽略的字段：走"诊断提示"那条分支
        app.chart.push_bytes(b"junk\n", app.chart_max_samples);
        assert!(app.chart.ignored_fields() > 0);
        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        // ⑤ 关掉自动缩放：不再下发 auto_bounds
        app.chart_auto_bounds = false;
        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });
    }

    /// 面板隐藏时绘制函数必须直接返回，不能借绘制之名改动任何状态。
    #[test]
    fn hidden_chart_panel_draws_nothing() {
        let mut app = ComTool {
            show_chart_panel: false,
            ..ComTool::default()
        };
        app.chart.push_bytes(b"1,2\n", app.chart_max_samples);
        let samples_before = app.chart.sample_count();
        let channels_before = app.chart.channel_count();

        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        assert_eq!(app.chart.sample_count(), samples_before);
        assert_eq!(app.chart.channel_count(), channels_before);
    }
}
