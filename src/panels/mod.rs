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
    use crate::app::{ComTool, Language};
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
            crate::receive::show_plain_receive(
                ui,
                &app.receive_text,
                app.auto_scroll,
                app.language.strings().copy_all_log,
                // 这一条只验"参数确实从 `ComTool` 传到了滚动区"，没有裁过内容，
                // 也就没有可补偿的东西。补偿本身由 `receive::tests` 里那条
                // 长时间停留的回归测试覆盖。
                None,
                // 高度上限：这条不验布局，给默认上限即可（让位逻辑另有一条测试）。
                crate::receive::RECEIVE_SCROLL_MAX_HEIGHT,
            );
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
            crate::receive::show_plain_receive(
                ui,
                &app.receive_text,
                app.auto_scroll,
                app.language.strings().copy_all_log,
                None,
                crate::receive::RECEIVE_SCROLL_MAX_HEIGHT,
            );
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

    /// 设置窗口的内容体要能单独画出来，并且**画得进去一个有限高度的容器**。
    ///
    /// 这条测的是"设置窗口加了滚动条"那个改动的两半：
    ///
    /// 1. 内容体 `settings_body` 被抽成了独立方法，所以能单独驱动。以前它整个长在
    ///    `Window::show` 的闭包里，无头测试只能连窗口一起画——窗口画得出来，并不代表
    ///    内容体本身没问题（比如某一项漏了 `ui.horizontal` 把宽度撑爆）。
    /// 2. 内容**确实"需要滚动"**——也就是它比可视区高。egui 在 `ScrollAreaOutput` 上
    ///    给这个信号的原文是："If this [`content_size`] is larger than [`inner_rect`],
    ///    then there was need for scrolling." 没有这一条，上面第 1 条只是"画得出来"，
    ///    退化成普通冒烟测试：把滚动区删掉、或者内容短到塞得下，它照样绿。
    #[test]
    fn settings_body_renders_inside_a_short_viewport() {
        let mut app = ComTool::default();
        let s = app.language.strings();
        let dir_before = app.log_dir.clone();

        const VIEWPORT_H: f32 = 120.0;

        let content_h = std::cell::Cell::new(0.0_f32);
        let viewport_h = std::cell::Cell::new(0.0_f32);
        // 内容实际被画到了多高（不受可视区裁剪影响的真实占位）。
        let painted_h = std::cell::Cell::new(0.0_f32);

        egui::__run_test_ui(|ui| {
            let short = egui::Rect::from_min_size(
                ui.max_rect().min,
                egui::vec2(ui.max_rect().width(), VIEWPORT_H),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(short), |ui| {
                let out = egui::ScrollArea::vertical()
                    .id_salt("settings_body_short_viewport_test")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        app.settings_body(ui, s);
                        // `content_ui` 的 min_rect 就是内容真实占用的高度：
                        // 它**应当**超过可视区——那正是需要滚动条的原因。
                        ui.min_rect().height()
                    });
                content_h.set(out.content_size.y);
                viewport_h.set(out.inner_rect.height());
                painted_h.set(out.inner);
            });
        });

        // ① 内容画得出来，且真的比可视区高。
        assert!(
            painted_h.get() > VIEWPORT_H,
            "设置内容只画了 {}px，没超过 {}px 的可视高度——这条测试退化成了普通冒烟测试",
            painted_h.get(),
            VIEWPORT_H
        );
        // ② egui 文档指定的"需要滚动"判据。
        assert!(
            content_h.get() > viewport_h.get(),
            "content_size({}px) 未超过 inner_rect({}px)，egui 认为不需要滚动",
            content_h.get(),
            viewport_h.get()
        );
        // ③ 可视区本身没有被内容顶开——滚动区的职责就是把这两者解耦。
        assert!(
            viewport_h.get() <= VIEWPORT_H + 1.0,
            "可视区被内容撑高了({}px > {}px)，滚动条就失去意义",
            viewport_h.get(),
            VIEWPORT_H
        );

        // ④ 绘制阶段**只准记录意图**，不准弹模态框：无头环境弹不了，真弹出会挂死。
        // `log_dir` 是"用户选了目录"的唯一落点，它没变就说明按钮确实没被点触发。
        assert_eq!(
            app.log_dir, dir_before,
            "绘制设置内容体不得直接触发目录选择或改动已选目录"
        );
    }

    /// 英文界面同样要能画出来。
    ///
    /// 文案表换了整整一列，这里才会暴露"英文那一列写坏了"的问题：`{}` 个数对不上
    /// （`fill` 会留下未填充的 `{}`）、某一列写成了空串、说明行比中文长得多把布局
    /// 撑坏……无头渲染至少能证明"切到 English 点一圈不会炸"。
    ///
    /// 外观仍要人工确认——这条测试证明不了控件里的字确实是英文。
    #[test]
    fn english_ui_renders_without_panic() {
        let mut app = ComTool {
            language: Language::En,
            show_settings: true,
            show_info: true,
            show_chart_panel: true,
            ..ComTool::default()
        };
        app.receive_text.push_str("hello\n");
        app.chart.push_bytes(b"1,2\n", app.chart_max_samples);

        egui::__run_test_ui(|ui| {
            app.show_menu_bar(ui);
        });

        let mut open = true;
        egui::__run_test_ui(|ui| {
            app.show_sidebar(ui, &mut open);
        });

        egui::__run_test_ui(|ui| {
            app.show_settings_window(ui);
            app.show_info_window(ui);
        });

        egui::__run_test_ui(|ui| {
            app.render_chart_panel(ui);
        });

        egui::__run_test_ui(|ui| {
            app.render_color_receive(ui);
            app.render_ascii_receive(ui);
        });
    }

    /// 已连接状态下的侧边栏：RTS / DTR 控件走"可点"分支，与未连接时的置灰分支
    /// 是两段不同的代码，两段都要画一遍（N1）。
    ///
    /// 顺带锁住一条约定：**绘制本身不得向通道投递任何命令**。无头渲染不投递指针
    /// 事件，所以 `changed()` 恒为 false——这正好是"只是把控件画出来"的等价物。
    #[test]
    fn connected_sidebar_renders_without_panic() {
        use crate::app::ConnectionKind;
        use crate::communicationtrait::fake::FakeChannel;

        let (channel, handle) = FakeChannel::new();
        let mut app = ComTool {
            channel: Some(Box::new(channel)),
            channel_kind: Some(ConnectionKind::Serial),
            connection_kind: ConnectionKind::Serial,
            selected_port: "COM_TEST".to_string(),
            ..ComTool::default()
        };
        app.open_connection();
        assert!(app.connected, "替身通道应当能打开成功");

        let mut open = true;
        egui::__run_test_ui(|ui| {
            app.show_sidebar(ui, &mut open);
        });

        assert!(
            handle.line_state_calls().is_empty(),
            "绘制侧边栏不得向通道投递线状态命令——只有用户真的点了那一下才会发"
        );
    }

    /// 常用数据那一行（N2）的四种状态都要能画出来：空列表、有若干条、已满、
    /// 以及未连接。
    ///
    /// 抽成 `ComTool::render_send_presets` 的意义就在这里：它原先写在
    /// `eframe::App::ui` 里，而 `ui()` 需要一个无头测试构造不出来的
    /// `eframe::Frame`，等于一次也不会被 `cargo test` 画到。
    #[test]
    fn preset_row_renders_without_panic() {
        let mut app = ComTool::default();

        // ① 空列表 + 未连接（「存为常用」置灰、「还没有常用数据」占位）
        assert!(app.send_presets.is_empty());
        egui::__run_test_ui(|ui| {
            app.render_send_presets(ui);
        });
        assert!(app.send_presets.is_empty(), "仅绘制不得改动列表");

        // ② 有几条 + 输入框内容重复（「存为常用」置灰的第三条理由）
        app.send_presets.add("AT");
        app.send_presets.add("AT+GMR");
        app.send_text = "AT".to_string();
        egui::__run_test_ui(|ui| {
            app.render_send_presets(ui);
        });
        assert_eq!(app.send_presets.len(), 2, "仅绘制不得改动列表");

        // ③ 已满（「存为常用」置灰的第一条理由）
        for i in 0..crate::send::MAX_SEND_PRESETS {
            app.send_presets.add(&format!("filler-{i}"));
        }
        assert!(app.send_presets.is_full());
        egui::__run_test_ui(|ui| {
            app.render_send_presets(ui);
        });
        assert!(app.send_presets.is_full(), "仅绘制不得淘汰任何一条");

        // ④ 英文界面（另一列文案，`preset_full` 是带占位符的模板）
        app.language = Language::En;
        egui::__run_test_ui(|ui| {
            app.render_send_presets(ui);
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

    /// 字体档位下拉必须**接在 `ui_font_scale` 字段上**，而不是画一个写死的值。
    ///
    /// 这条测试针对的是一种很隐蔽的写错方式：`selected_text(...)` 传了个常量，
    /// 界面上看起来完全正常，但下拉里显示的档位**从来不是用户选的那一档**。
    ///
    /// 断言方式是**按顺序读真实绘制输出**：把绘制出的文字按落笔顺序排成一串，
    /// 找到「字体大小」这个标签，**紧跟在它后面的那一条**就是下拉的选中文案，
    /// 它必须恰好等于当前档位名。
    ///
    /// ⚠️ 为什么不直接断言"画出的文字里包含当前档位名"——那样**是空转的**。
    /// `font_scale_preview`（「预览：{}」）本来就会把当前档位名写进去，那正是
    /// 预览的意思；只要预览行还在，把 `selected_text` 写成什么都不会被抓出来。
    /// 实测过：把 `selected_text` 写死成标准档，用"包含"断言时测试照样绿；
    /// 改成"紧跟标签之后的那一条"之后立刻变红。
    ///
    /// ⚠️ 无头环境不投递指针事件，所以**不能**用"模拟点一下再断言字段变了"——
    /// 那会得到一条恒真的空转测试（事件根本没送到，字段自然"没变"，断言永远成立）。
    /// 这里测的是"字段 → 显示"这条已经接通的连线，它是能被真实反驳的。
    #[test]
    fn font_scale_dropdown_is_wired_to_the_field() {
        use crate::app::UiFontScale;

        // 四档全来一遍（含默认档，覆盖"默认值也能正确显示"）。
        for scale in UiFontScale::ALL {
            let mut app = ComTool {
                ui_font_scale: scale,
                ..ComTool::default()
            };
            // 中英各来一遍：档位名必须来自文案表，不能写死中文。
            for language in [Language::Zh, Language::En] {
                app.language = language;
                let s = app.language.strings();

                let picked = match scale {
                    UiFontScale::Small => s.font_scale_small,
                    UiFontScale::Normal => s.font_scale_normal,
                    UiFontScale::Large => s.font_scale_large,
                    UiFontScale::ExtraLarge => s.font_scale_xlarge,
                };

                let shown = selected_font_scale_text(&mut app);

                assert_eq!(
                    shown, picked,
                    "字体档位下拉显示的是 {shown:?}，但字段里存的是 {scale:?}\
                     （应当显示 {picked:?}）——说明 selected_text 没有读取 \
                     ui_font_scale，或者 selectable_value 接在了局部变量上"
                );

                // 绘制是只读的：画一遍不得改动字段。
                assert_eq!(
                    app.ui_font_scale, scale,
                    "绘制设置内容体不得改动字体档位字段"
                );
            }
        }
    }

    /// 取字体档位下拉**选中文案**的实际绘制值。
    ///
    /// 做法：按落笔顺序把所有文字抠成 `Vec<String>`，找到「字体大小」标签所在位置，
    /// 取它**紧跟着的下一段**文字。egui 的绘制顺序与调用顺序一致，而下拉按钮就在
    /// 标签之后、预览行之前，所以这个位置上的文字就是下拉的选中文案。
    /// 语言相关的干扰项（预览行）排在更后面，不会落在这个位置上。
    fn selected_font_scale_text(app: &mut ComTool) -> String {
        let texts = painted_texts(app);

        // 标签在不同语言下文案不同，所以从当前语言取。
        let s = app.language.strings();
        let label = s.font_scale_label;
        let label_pos = texts.iter().position(|t| t == label).unwrap_or_else(|| {
            panic!("画出的文字里找不到字体档位的标签 {label:?}——标签本身没画出来？实际：{texts:?}")
        });

        texts
            .get(label_pos + 1)
            .unwrap_or_else(|| {
                panic!("「{label}」标签后面没有跟任何文字，下拉按钮是空的。实际：{texts:?}")
            })
            .clone()
    }

    /// 把一次绘制里**所有真实落笔的文字**按顺序抠出来。
    ///
    /// 走 `Context::run_ui` 拿 `FullOutput.shapes`，而不是 `__run_test_ui`——
    /// 后者把输出丢掉了（`drop_without_applying_deltas`），什么也观察不到，
    /// 正是"空转测试"的来源。
    fn painted_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let s = app.language.strings();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.settings_body(ui, s);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            // `Shape::Text` 装的是一个 `TextShape`，文字在它的 `galley` 里。
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }

        // `FullOutput` 带着纹理增量，直接 drop 会 panic（这正是 `__run_test_ui`
        // 内部要写 `drop_without_applying_deltas()` 的原因）。我们不需要纹理，
        // 显式丢弃即可——否则这条测试会因为一个与断言无关的清理问题而"红"，
        // 看起来像抓到了缺陷，其实是假的。
        out.textures_delta.clear();
        texts
    }

    /// 顶部菜单栏**不再提供「发送」菜单**（2026-09-24 用户要求删除）。
    ///
    /// 删它的理由：它的内容（发送格式 / 行尾 / 定时发送 + 间隔 / 发送历史）与
    /// 「发送区」表头那一行右侧的控件**逐项重复**，连排列顺序都一样。同一批设置
    /// 摆两套入口，用户改了一处会怀疑另一处是否同步。
    ///
    /// ⚠️ 为什么不能断言"画出的文字里不含 `send_title`"——**那会是空转的**。
    /// `send_title`（「发送」）在别处还要用：中央区那颗发送按钮、设置窗口的分节标题
    /// 都写这个字。这些文字**不在菜单栏这一帧里**，但一旦将来有人把菜单加回来、
    /// 再把断言写成"整帧不含"就恒真了——它证明不了菜单栏的状态。这里刻意只
    /// **驱动菜单栏**（`show_menu_bar`），断言的就是它的顶层按钮集合。
    ///
    /// ⚠️ 第二个坑：菜单是**折叠**的，未点击时子项（格式、行尾……）根本不落笔。
    /// 所以"子项没画出来"同样证明不了菜单被删——真正能区分的是**顶层按钮那一行**
    /// 里还有没有「发送」这个按钮。
    #[test]
    fn menu_bar_has_no_send_menu() {
        let app_langs = [Language::Zh, Language::En];
        for language in app_langs {
            let mut app = ComTool {
                language,
                ..ComTool::default()
            };
            let s = app.language.strings();

            let texts = menu_bar_texts(&mut app);

            // ① 删除的对象：顶层不能再出现「发送」这个菜单标题。
            assert!(
                !texts.iter().any(|t| t == s.send_title),
                "菜单栏顶层又画出了「发送」菜单 ({:?})——它的内容与发送区表头重复，\
                 应当保持删除。实际画出的文字：{texts:?}",
                s.send_title
            );

            // ② 活着的菜单必须还在（证明"删干净"不是"整个菜单栏都空了"）。
            //    缺了这几个同样会漏掉功能，只是漏的是另一边。
            for kept in [s.menu_file, s.settings_title, s.menu_view, s.text] {
                assert!(
                    texts.iter().any(|t| t == kept),
                    "菜单栏少了 {kept:?}——删除「发送」不应连累其它菜单。实际：{texts:?}"
                );
            }
        }
    }

    /// 中央区顶部那一行**不再有**「⚙ 设置」按钮（2026-09-24 用户要求删除）。
    ///
    /// 删它的理由：它与顶部菜单栏的「设置」菜单功能重复——两者都只是把
    /// `show_settings` 置 true，点开的是同一个窗口。用户要求**保留顶部的**。
    ///
    /// ⚠️ 与 `menu_bar_has_no_send_menu` 同一个坑：**不能**断言"整个界面里不含
    /// 「设置」二字"——菜单栏顶层菜单标题就叫「设置」（`s.settings_title`），
    /// 设置窗口自己的标题也是它。所以这里只驱动**顶部那一行**
    /// （`render_top_toolbar`），判据落在那块自己的落笔文字上。
    ///
    /// ⚠️ 用 `settings_button` 的**字面量**做判据，而不是引用字段——那个字段已经
    /// 随这次的删除从文案表里删掉了，引用不到。字面量同时钉住"文案表里没有把
    /// 按钮加回来"这层：若有人只把按钮加回 `render_top_toolbar`、又顺手把
    /// `settings_button` 条目加回 `strings.rs`，两处都会撞在这条上。
    #[test]
    fn top_toolbar_has_no_settings_button() {
        // 三个语种各来一遍：判据取自我们**知道**的按钮文案（中文一种），
        // 其余两种是英文，用来验证按钮无论如何都不出现。
        for language in [Language::Zh, Language::En] {
            let mut app = ComTool {
                language,
                ..ComTool::default()
            };
            let s = app.language.strings();

            let texts = top_toolbar_texts(&mut app);

            // ① 删除的对象：那一行里不能再出现「设置」按钮（含齿轮符号那版）。
            for ghost in ["⚙ 设置", "⚙ Settings"] {
                assert!(
                    !texts.iter().any(|t| t == ghost),
                    "中央区顶部的「{ghost}」按钮又被画出来了——它与菜单栏「设置」菜单\
                     功能重复，应当保持删除。实际画出的文字：{texts:?}"
                );
            }

            // ② 活着的控件必须还在：侧边栏显隐开关。缺了它这一行就空了，
            //    而"删干净"不该把另一边也删掉。
            assert!(
                texts.iter().any(|t| t == s.sidebar_toggle),
                "顶部那一行少了侧边栏开关 {:?}——删除设置按钮不应连累它。实际：{texts:?}",
                s.sidebar_toggle
            );
        }
    }

    /// 菜单栏的「设置」菜单**仍然提供**打开设置窗口的入口。
    ///
    /// 与 [`top_toolbar_has_no_settings_button`] 配对：那条证明"重复的那颗按钮删了"，
    /// 这条证明"剩下的那一个还在"。只删不查的话，把菜单栏入口一起删掉也会全绿，
    /// 用户就再也打不开设置窗口了。
    #[test]
    fn menu_bar_still_offers_settings() {
        for language in [Language::Zh, Language::En] {
            let mut app = ComTool {
                language,
                ..ComTool::default()
            };
            let s = app.language.strings();

            let texts = menu_bar_texts(&mut app);

            assert!(
                texts.iter().any(|t| t == s.settings_title),
                "菜单栏少了「设置」菜单——那是打开设置窗口的唯一入口了。实际：{texts:?}"
            );
            // 顺带钉住顶层菜单的整体集合：删按钮不该动这一行。
            for kept in [s.menu_file, s.menu_view, s.text] {
                assert!(
                    texts.iter().any(|t| t == kept),
                    "菜单栏少了 {kept:?}。实际：{texts:?}"
                );
            }
        }
    }

    /// 「视图」菜单主体里**画得出**「总在最前端」这一项。
    ///
    /// 判据是**文字**（文案表里的 `always_on_top`），中英各测一遍——这样"菜单里根本
    /// 没有这一项"和"这一项只有中文"两种情况都会红。
    ///
    /// ⚠️ 驱动的是**菜单主体**而不是整个菜单栏：菜单主体只在菜单被点开时才画，
    /// 直接驱动 `show_menu_bar` 取到的文字只有那四个顶层菜单名，断言会**恒假**
    /// （写死什么都红，等于没测）。这也是主体被抽成 `view_menu_body` 的理由。
    #[test]
    fn view_menu_offers_always_on_top() {
        for language in [Language::Zh, Language::En] {
            let mut app = ComTool {
                language,
                ..ComTool::default()
            };
            let s = app.language.strings();

            let texts = view_menu_texts(&mut app);

            assert!(
                texts.iter().any(|t| t == s.always_on_top),
                "「视图」菜单里少了「总在最前端」——用户没有地方打开它。实际：{texts:?}"
            );
            // 顺带钉住同一组里原有的「自动滚动」没被新增的这一项挤掉。
            assert!(
                texts.iter().any(|t| t == s.auto_scroll),
                "加「总在最前端」把「自动滚动」挤没了。实际：{texts:?}"
            );
        }
    }

    /// 「总在最前端」勾上后**真的给窗口下发命令**，取消后**真的退出置顶**。
    ///
    /// 这是本功能的判据本体：字段只是偏好值，用户要的是"窗口真的压在别人上面"。
    /// 读的是 egui 每帧输出里的视口命令（`FullOutput::viewport_output`）——它是公开的，
    /// 所以"发没发、发的是哪一档"在无头测试里可以直接断言，不必真的开一个窗口。
    ///
    /// 三条断言各自挡一类回归：
    /// ① 只加字段不下发（界面上能勾，窗口不动）；
    /// ② 每帧重复下发（Windows 上等于每帧一次 `SetWindowPos`）；
    /// ③ 取消勾选时不回到 `Normal`（窗口从此一直置顶，且没有任何入口能取消）。
    ///
    /// 缺的一半在 `config::tests::preferences_survive_round_trip`：字段要跟着存档走。
    /// 只有这条的话，把 `always_on_top` 标成 `#[serde(skip)]` 也照样全绿——那就是
    /// "勾了置顶，重启就没了"。
    #[test]
    fn always_on_top_sends_the_window_level_command() {
        use egui::ViewportCommand;
        use egui::viewport::WindowLevel;

        /// 这一帧下发的**窗口层级**命令（其它命令不参与判定）。
        fn window_levels(cmds: &[ViewportCommand]) -> Vec<WindowLevel> {
            cmds.iter()
                .filter_map(|c| match c {
                    ViewportCommand::WindowLevel(level) => Some(*level),
                    _ => None,
                })
                .collect()
        }

        // ① 勾上 ⇒ 下发 AlwaysOnTop
        let mut app = ComTool {
            always_on_top: true,
            ..ComTool::default()
        };
        let first = menu_bar_viewport_commands(&mut app);
        assert_eq!(
            window_levels(&first),
            vec![WindowLevel::AlwaysOnTop],
            "勾上「总在最前端」却没给窗口下发置顶命令——窗口不会有任何变化。实际：{first:?}"
        );

        // ② 状态没变 ⇒ 不再下发（去重）
        let second = menu_bar_viewport_commands(&mut app);
        assert!(
            window_levels(&second).is_empty(),
            "同一个状态被重复下发窗口层级命令——每帧一次 SetWindowPos。实际：{second:?}"
        );

        // ③ 取消勾选 ⇒ 必须显式回到 Normal
        app.always_on_top = false;
        let third = menu_bar_viewport_commands(&mut app);
        assert_eq!(
            window_levels(&third),
            vec![WindowLevel::Normal],
            "取消「总在最前端」后没把窗口放回 Normal——它从此一直压在别人上面。实际：{third:?}"
        );

        // 默认必须是"不置顶"：置顶会盖住别人的窗口，只能由用户主动打开。
        assert!(
            !ComTool::default().always_on_top,
            "默认不该是置顶——那等于替所有用户决定盖住别的窗口"
        );
    }

    /// 驱动**「视图」菜单主体**并取出其中落笔的所有文字。
    ///
    /// 与 [`menu_bar_texts`] 的区别：那个只画得到顶层菜单名，这个画的是菜单展开后
    /// 的内容（面板显隐 / 显示模式 / 自动滚动 / 总在最前端 / 时间戳前缀）。
    fn view_menu_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.view_menu_body(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }

        out.textures_delta.clear();
        texts
    }

    /// 驱动**菜单栏**并取出本帧下发给窗口的视口命令。
    ///
    /// 菜单栏每帧必画，且 `apply_always_on_top` 就挂在它上面，所以这是"界面真的把
    /// 偏好下发了"的接入点——纯函数测试全绿 ≠ 界面调了它。
    fn menu_bar_viewport_commands(app: &mut ComTool) -> Vec<egui::ViewportCommand> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.show_menu_bar(ui);
        });

        let commands = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|output| output.commands.clone())
            .unwrap_or_default();

        out.textures_delta.clear();
        commands
    }

    /// 驱动**中央区顶部那一行**并取出其中落笔的所有文字。
    fn top_toolbar_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.render_top_toolbar(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }

        out.textures_delta.clear();
        texts
    }

    /// 驱动**菜单栏自身**并取出其中落笔的所有文字。
    ///
    /// 与 [`painted_texts`]（驱动设置窗口）是两份不同的驱动入口：菜单栏的
    /// 「有没有某个菜单」只能在菜单栏这一帧里判定，不能拿整帧输出代替——整帧里
    /// 混着中央区的发送按钮（同样写着「发送」），会把判据污染成恒真。
    fn menu_bar_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.show_menu_bar(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }

        out.textures_delta.clear();
        texts
    }

    /// 发送区表头那一行右侧**仍然是**这四项设置的唯一入口。
    ///
    /// 与 [`menu_bar_has_no_send_menu`] 配对：那条证明"菜单删了"，这条证明
    /// "功能没丢"。只删不查的话，把发送区表头一起删掉也会全绿。
    ///
    /// 做法是把发送区那一行画出来，断言其中同时出现 格式 / 行尾 这两组单选按钮的
    /// 选项（三档格式与三档行尾各自取一个代表），以及"定时发送"这一项。它们都是
    /// 文案表里的条目，因此中英各测一遍。
    #[test]
    fn send_area_header_keeps_the_send_settings() {
        for language in [Language::Zh, Language::En] {
            let mut app = ComTool {
                language,
                ..ComTool::default()
            };
            let s = app.language.strings();

            let texts = send_area_header_texts(&mut app);

            // 格式三档：至少要能看到 HEX 与 Escape 这两个选项（Text 与其它词重名，
            // 不拿它当判据）。
            for expected in [s.hex, s.escape, s.ending_lf, s.ending_crlf, s.repeat_send] {
                assert!(
                    texts.iter().any(|t| t == expected),
                    "发送区表头少了 {expected:?}——删掉菜单栏「发送」时把功能也删掉了。\
                     实际：{texts:?}"
                );
            }
        }
    }

    /// 驱动**发送区表头那一行**（格式 / 行尾 / 定时 / 历史所在处）并取出文字。
    fn send_area_header_texts(app: &mut ComTool) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.render_send_settings_row(ui);
        });

        let mut texts = Vec::new();
        for clipped in &out.shapes {
            if let egui::epaint::Shape::Text(text_shape) = &clipped.shape {
                texts.push(text_shape.galley.text().to_owned());
            }
        }

        out.textures_delta.clear();
        texts
    }

    /// 字体档位必须真的作用于**设置窗口自己**，而不只是接收区。
    ///
    /// 这条把两半接起来测：界面实际用的字号应当等于当前档位的字号。如果哪天有人把
    /// `apply_font_scale` 从每帧调用里删掉，或者把它错放到接收区渲染内部，这条会红
    /// ——而直接调 `apply_font_scale` 的那些单测不会（它们证明不了"它被调用了"）。
    #[test]
    fn font_scale_applies_to_the_settings_window_itself() {
        use crate::app::UiFontScale;

        let mut app = ComTool {
            ui_font_scale: UiFontScale::ExtraLarge,
            ..ComTool::default()
        };
        let s = app.language.strings();

        let drawn_body_size = std::cell::Cell::new(0.0_f32);
        egui::__run_test_ui(|ui| {
            app.apply_font_scale(ui.ctx());
            drawn_body_size.set(
                ui.ctx()
                    .global_style()
                    .text_styles
                    .get(&egui::TextStyle::Body)
                    .expect("Body 样式必须存在")
                    .size,
            );
            app.settings_body(ui, s);
        });

        let expected = 13.0 * UiFontScale::ExtraLarge.factor();
        assert_eq!(
            drawn_body_size.get(),
            expected,
            "设置窗口里的正文字号应当是「基线 13 × 特大档系数」= {expected}"
        );
        assert!(
            drawn_body_size.get() > 13.0,
            "特大档下正文字号必须大于标准档的 13，否则这个功能等于没接上"
        );
    }
    /// 顶部那一行：**筛选在生效就显示命中数**——包括把筛选输入框那一行收起来的时候。
    ///
    /// 这条小字是"日志被筛过"唯一的常驻解释。少了它，用户收起筛选行之后看到一屏
    /// 稀疏的日志，只会以为设备少发了数据。
    ///
    /// 三条分开断：没筛时不许出现（否则这个指示器就成了噪音）、有筛时必须出现、
    /// **接收区隐藏时不出现**（那份数是上一帧记的，此时已无意义）。
    #[test]
    fn top_toolbar_reports_the_filter_state() {
        let s = crate::app::Language::Zh.strings();
        let count_of = |app: &ComTool| {
            crate::strings::fill(
                s.filter_count,
                &[&app.filter_matched_lines, &app.filter_total_lines],
            )
        };

        // ① 没筛：只有开关
        let mut app = ComTool::default();
        let texts = top_toolbar_texts(&mut app);
        assert!(
            texts.iter().any(|t| t == s.filter_toggle),
            "顶部少了筛选开关。实际：{texts:?}"
        );
        assert!(
            !texts.contains(&count_of(&app)),
            "还没筛选就显示了命中数。实际：{texts:?}"
        );

        // ② 有筛（输入框那行收着，默认就是收着的）：计数照样显示
        app.filter_text = "345".to_string();
        app.filter_matched_lines = 2;
        app.filter_total_lines = 7;
        let expected = count_of(&app);
        assert!(
            expected.contains('2') && expected.contains('7'),
            "前提：这条文案里应当带上两个数字，实际是 {expected:?}"
        );
        let texts = top_toolbar_texts(&mut app);
        assert!(
            texts.contains(&expected),
            "筛选生效却没显示命中数——用户看不出日志被筛过。期待 {expected:?}，实际：{texts:?}"
        );

        // ③ 接收区隐藏：不显示（没有可筛的内容，那份数是陈的）
        app.show_receive_panel = false;
        let texts = top_toolbar_texts(&mut app);
        assert!(
            !texts.contains(&expected),
            "接收区都隐藏了还显示命中数。实际：{texts:?}"
        );
    }
}
