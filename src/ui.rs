use crate::app::{AppState, LoginMode, Tab};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::Disableable as _;
use gpui_kit::Focusable as _;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, Context, ElementId, Entity, FontWeight, IntoElement, Render, Window};

pub(crate) const BG: u32 = 0xf3f5f9;
pub(crate) const PANEL: u32 = 0xffffff;
pub(crate) const TEXT: u32 = 0x111827;
pub(crate) const MUTED: u32 = 0x6b7280;
pub(crate) const BORDER: u32 = 0xe5e7eb;
pub(crate) const PRIMARY: u32 = 0x2563eb;
pub(crate) const PRIMARY_SOFT: u32 = 0xe8efff;
pub(crate) const DANGER: u32 = 0xdc2626;
pub(crate) const SUCCESS: u32 = 0x15803d;
pub(crate) const WARN: u32 = 0xb45309;

pub(crate) fn panel() -> gpui_kit::Div {
    div()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(BORDER))
}

pub(crate) fn input_field(
    state: &Entity<InputState>,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let field = state.clone();
    div()
        .id(ElementId::Name(
            format!("input-wrapper-{:?}", state.entity_id()).into(),
        ))
        .w_full()
        .on_mouse_down_out(cx.listener(move |_, _, window, cx| {
            if field.read(cx).focus_handle(cx).is_focused(window) {
                window.blur(cx);
            }
        }))
        .child(
            Input::new(state)
                .w_full()
                .focus_bordered(false)
                .rounded_sm()
                .bg(rgb(PANEL))
                .border_color(rgb(BORDER))
                .text_color(rgb(TEXT)),
        )
}

pub(crate) fn subtitle(text: impl Into<String>) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(MUTED))
        .child(gpui_kit::SharedString::from(text.into()))
}

pub(crate) fn title(text: impl Into<String>) -> gpui_kit::Div {
    div()
        .text_xl()
        .font_weight(FontWeight::BOLD)
        .child(gpui_kit::SharedString::from(text.into()))
}


impl AppState {
    fn tab_button(
        &self,
        id: &'static str,
        label: &'static str,
        tab: Tab,
        cx: &mut Context<Self>,
    ) -> Button {
        let selected = self.tab == tab;
        let button = Button::new(id).label(label);
        let button = if selected { button.primary() } else { button.ghost() };
        button.on_click(cx.listener(move |this, _, window, cx| {
            this.tab = tab;
            if tab == Tab::Plaza {
                this.refresh_plaza(window, cx);
            }
            cx.notify();
        }))
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(62.0))
            .px_5()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Course Share"),
                    )
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.tab_button("tab-courses", "课程", Tab::Courses, cx))
                    .child(self.tab_button("tab-plaza", "广场", Tab::Plaza, cx))
                    .child(self.tab_button("tab-settings", "设置", Tab::Settings, cx))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(subtitle(if self.http.is_some() { "已登录" } else { "未登录" }))
                    .flex_shrink_0()
                    .when(self.http.is_some(), |this| {
                        this.child(
                            Button::new("logout")
                                .label("退出")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                        )
                    }),
            )
    }

    fn login_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.pending_login.clone();
        let password_mode = self.login_mode == LoginMode::Password;
        let button_label = if self.busy { "请稍候…" } else { "登录教学网" };
        let card = panel().w(px(520.0)).p_6().flex().flex_col().gap_4();

        let card = if let Some(pending) = pending {
            let kind = pending.challenge.kind.clone();
            let mut card = card
                .child(title("需要二次验证"))
                .child(subtitle(format!(
                    "{}：{}",
                    kind.label(),
                    pending.challenge.message
                )));
            if kind == crate::auth::VerificationKind::Sms {
                card = card.child(
                    Button::new("send-sms")
                        .label("发送短信验证码")
                        .ghost()
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send_sms_code(window, cx);
                        })),
                );
            }
            if kind == crate::auth::VerificationKind::Captcha {
                if let Some(path) = self.captcha_path.clone() {
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                gpui_kit::img(path)
                                    .w(px(160.0))
                                    .h(px(48.0))
                                    .rounded_md(),
                            )
                            .child(
                                Button::new("refresh-captcha")
                                    .label("刷新验证码")
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_captcha(window, cx);
                                    })),
                            ),
                    );
                } else {
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(subtitle("正在获取验证码图片…"))
                            .child(
                                Button::new("refresh-captcha-empty")
                                    .label("刷新验证码")
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_captcha(window, cx);
                                    })),
                            ),
                    );
                }
            }
            card
                .child(input_field(&self.verification_input, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("submit-verification")
                                .label("提交验证")
                                .primary()
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_verification(window, cx);
                                })),
                        )
                        .child(
                            Button::new("cancel-verification")
                                .label("取消")
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_verification(window, cx);
                                })),
                        ),
                )
        } else {
            card
                .child(title("登录教学网"))
                .child(subtitle(
                    "使用 IAAA 账号登录后读取个人课程与回放；不会保存密码。",
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.login_mode_button(
                            "mode-password",
                            "账号密码",
                            LoginMode::Password,
                            password_mode,
                            cx,
                        ))
                        .child(self.login_mode_button(
                            "mode-cookie",
                            "Cookie 导入",
                            LoginMode::Cookie,
                            !password_mode,
                            cx,
                        )),
                )
                .child(if password_mode {
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(input_field(&self.username, cx))
                        .child(input_field(&self.password, cx))
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(input_field(&self.cookie_input, cx))
                        .child(subtitle(
                            "在浏览器登录 course.pku.edu.cn 后，从 DevTools 复制 Cookie 请求头。",
                        ))
                        .into_any_element()
                })
                .child(
                    Button::new("login-submit")
                        .label(button_label)
                        .primary()
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.login_mode == LoginMode::Password {
                                this.start_login(window, cx);
                            } else {
                                this.start_cookie_login(window, cx);
                            }
                        })),
                )
        };

        let card = card.when_some(self.error.clone(), |this, error| {
            this.child(div().text_sm().text_color(rgb(DANGER)).child(error))
        });

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(card)
    }

    fn login_mode_button(
        &self,
        id: &'static str,
        label: &'static str,
        mode: LoginMode,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let button = Button::new(id).label(label);
        let button = if selected { button.primary() } else { button.ghost() };
        button.on_click(cx.listener(move |this, _, _, cx| {
            this.login_mode = mode;
            cx.notify();
        }))
    }

    fn footer(&self) -> impl IntoElement {
        let fraction = self
            .progress
            .as_ref()
            .map(|progress| {
                if progress.total == 0 {
                    0.0
                } else {
                    (progress.downloaded as f32 / progress.total as f32).clamp(0.0, 1.0)
                }
            })
            .unwrap_or(0.0);
        div()
            .min_h(px(56.0))
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(if self.error.is_some() { rgb(DANGER) } else { rgb(MUTED) })
                            .child(
                                self.error
                                    .clone()
                                    .unwrap_or_else(|| self.status.clone())
                                    .chars()
                                    .take(300)
                                    .collect::<String>(),
                            ),
                    )
                    .when(self.downloading, |this| {
                        this.child(subtitle(format!(
                            "新增{} / 跳过{} / 失败{}",
                            self.downloaded, self.skipped, self.failed
                        )))
                    })
                    .flex_shrink_0(),
            )
            .when_some(self.progress.clone(), |this, progress| {
                let width = 400.0 * fraction;
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(400.0))
                                .h(px(6.0))
                                .bg(rgb(BORDER))
                                .rounded_full()
                                .child(
                                    div()
                                        .w(px(width))
                                        .h(px(6.0))
                                        .bg(rgb(PRIMARY))
                                        .rounded_full(),
                                ),
                        )
                        .child(subtitle(format!("{} · {}", progress.title, progress.detail))),
                )
            })
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logged_in = self.http.is_some();
        let content = if self.tab == Tab::Plaza {
            self.plaza_view(window, cx).into_any_element()
        } else if self.tab == Tab::Settings {
            self.settings_view(window, cx).into_any_element()
        } else if logged_in {
            self.body(window, cx).into_any_element()
        } else {
            self.login_view(cx).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(self.header(cx))
            .child(div().flex_1().min_h(px(0.0)).child(content))
            .when(self.downloading || self.error.is_some(), |this| {
                this.child(self.footer())
            })
    }
}
