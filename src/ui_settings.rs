use crate::app::AppState;
use crate::theme::tone;
use crate::ui::{subtitle, title, DANGER, MUTED, SUCCESS};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, Context, IntoElement, Window};

impl AppState {
    pub(crate) fn settings_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ffmpeg = crate::ffmpeg::locate(&self.ffmpeg_input.read(cx).value());
        let project_url = "https://github.com/dfshfghj/course-share";
        div()
            .size_full()
            .p_6()
            .child(title("设置").mb_2())
            .child(
                div()
                .pl_4()
                .child(subtitle("主题"))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .items_center()
                        .gap_2()
                        .child(self.theme_button(
                            "theme-auto",
                            "跟随系统",
                            crate::theme::ThemeMode::Auto,
                            cx,
                        ))
                        .child(self.theme_button(
                            "theme-light",
                            "浅色",
                            crate::theme::ThemeMode::Light,
                            cx,
                        ))
                        .child(self.theme_button(
                            "theme-dark",
                            "深色",
                            crate::theme::ThemeMode::Dark,
                            cx,
                        )),
                )
            )
            .child(
                div()
                    .pl_4()
                    .child(subtitle("参数配置").mb_1())
                    .child(
                        div()
                        .pl_4()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child("共享后端地址")
                                .child(crate::ui::input_field(&self.backend_input, cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child("回放下载目录")
                                .child(crate::ui::input_field(&self.download_dir_input, cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child("ffmpeg 路径")
                                .child(crate::ui::input_field(&self.ffmpeg_input, cx))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(tone(if ffmpeg.source == "未找到" {
                                            DANGER
                                        } else {
                                            MUTED
                                        }))
                                        .child(format!(
                                            "当前使用：{}",
                                            ffmpeg.path.display()
                                        )),
                                ),
                        )
                    )
                    .child(
                        div()
                        .flex()
                        .justify_end()
                        .mb_4()
                        .child(
                            Button::new("save-settings")
                                .label("保存设置")
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_settings(window, cx);
                                })),
                        )
                    )
            )
            .child(
                div()
                .mb_8()
                .child(title("关于").mb_2())
                .child(
                    div()
                    .pl_4()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child("当前版本: ")
                    .child(
                    div()
                    .text_sm()
                    .text_color(tone(SUCCESS))
                    .child("v 0.1.0")
                    )
                )
                .child(
                    div()
                    .pl_4()
                    .flex()
                    .items_center()
                    .child("项目地址: ")
                    .child(
                        Button::new("about-project")
                            .icon(IconName::Github)
                            .label(project_url)
                            .link()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(error) = crate::preview::open_browser(project_url) {
                                    this.error = Some(format!("{error:#}"));
                                    cx.notify();
                                }
                            })),
                    )
                )
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .child(
                        Button::new("logout-settings")
                            .label("退出登录")
                            .w_32()
                            .warning()
                            .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                    ),
            )
    }

    fn theme_button(
        &self,
        id: &'static str,
        label: &'static str,
        mode: crate::theme::ThemeMode,
        cx: &mut Context<Self>,
    ) -> Button {
        let selected = crate::theme::ThemeMode::from_pref(&self.config.theme) == mode;
        let button = Button::new(id).label(label);
        let button = if selected { button.primary() } else { button.ghost() };
        button.on_click(cx.listener(move |this, _, _, cx| {
            this.set_theme(mode, cx);
        }))
    }
}
