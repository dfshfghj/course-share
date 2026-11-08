use crate::app::AppState;
use crate::ui::{panel, subtitle, title, DANGER, MUTED, SUCCESS};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, Context, IntoElement, Window};

impl AppState {
    pub(crate) fn settings_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ffmpeg = crate::ffmpeg::locate(&self.ffmpeg_input.read(cx).value());
        div()
            .size_full()
            .p_6()
            .flex()
            .justify_center()
            .child(
                panel()
                    .w(px(720.0))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(title("设置"))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(subtitle("共享后端地址"))
                            .child(crate::ui::input_field(&self.backend_input, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(subtitle("回放下载目录"))
                            .child(crate::ui::input_field(&self.download_dir_input, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(subtitle("ffmpeg 路径（留空自动，优先使用内置）"))
                            .child(crate::ui::input_field(&self.ffmpeg_input, cx))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(if ffmpeg.source == "未找到" {
                                        DANGER
                                    } else {
                                        MUTED
                                    }))
                                    .child(format!(
                                        "当前使用（{}）：{}",
                                        ffmpeg.source,
                                        ffmpeg.path.display()
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("save-settings")
                                    .label("保存设置")
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_settings(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("test-backend")
                                    .label("测试后端")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.test_backend(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("logout-settings")
                                    .label("清除 Cookie 并退出")
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(subtitle(format!(
                                "当前状态：{}",
                                if self.http.is_some() { "已登录" } else { "未登录" }
                            )))
                            .child(subtitle(format!("Cookie：{}", self.cookie_summary())))
                            .child(subtitle(format!(
                                "下载目录：{}",
                                self.config.download_path().display()
                            )))
                            .child(subtitle(format!(
                                "已保存课程：{} 门 / 当前回放：{} 条 / 广场课程：{} 门",
                                self.courses.len(),
                                self.replays.len(),
                                self.plaza_courses.len()
                            ))),
                    )
                    .when_some(self.error.clone(), |this, error| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(rgb(DANGER))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(SUCCESS))
                                    .child("v 0.1.0"),
                            )
                    ),
            )
    }
}
