use crate::app::AppState;
use crate::ui::{panel, subtitle, title, BORDER, DANGER, MUTED, SUCCESS, WARN};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, Context, IntoElement, Window};

impl AppState {
    pub(crate) fn settings_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(subtitle(
                        "后端地址用于共享广场；下载目录和 ffmpeg 只影响本机。",
                    ))
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
                            .child(subtitle("ffmpeg 路径（HLS 合并需要）"))
                            .child(crate::ui::input_field(&self.ffmpeg_input, cx)),
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
                                "已保存课程：{} 门 / 当前回放：{} 条 / 广场：{} 条",
                                self.courses.len(),
                                self.replays.len(),
                                self.shares.len()
                            ))),
                    )
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child("说明"),
                            )
                            .child(subtitle(
                                "下载只读取教学网数据，不修改课程状态。Cookie 仅保存在本机数据目录。",
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(WARN))
                                    .child("downloadVideo.action 直链已被标记为失效，统一下载器只使用 resourcese 的 HLS/MP4 直链。"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("好学 App 路线按 TODO 暂缓实现；当前专注教学网 + GUID 共享广场。"),
                            ),
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
                                    .child("版本 0.1.0"),
                            )
                            .child(subtitle("Rust + GPUI Kit 0.7.3")),
                    ),
            )
    }
}
