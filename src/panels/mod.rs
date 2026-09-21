//! 顶部菜单栏与设置 / 信息窗口。
//!
//! 与 [`crate::sidebar`] 同一条约定：绘制函数只负责画控件，并把「用户要求做什么」
//! 记录成 action 结构体，真正的副作用（退出程序、打开窗口）由 `app.rs` 统一派发。
//! 直接把值绑到 `ComTool` 字段上的控件（勾选框、下拉框、数值框）不经过 action——
//! 它们改的是状态，不是副作用。
//!
//! 模块内都是 `impl ComTool` 的固有方法，因此这里没有对外 re-export：方法随类型
//! 可见，不需要（也无法）用 `use` 再引出一次。

mod info;
mod menu_bar;
mod settings;

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
}
