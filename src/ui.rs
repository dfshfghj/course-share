use crate::app::{AppState, DownloadTask, DownloadTaskState, LoginMode};
use crate::theme::tone;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::Disableable as _;
use gpui_component::{IconName, Sizable as _};
use gpui_kit::Focusable as _;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, ElementId, Entity, FontWeight, IntoElement, MouseButton, Render, Window};

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
        .bg(tone(PANEL))
        .border_1()
        .border_color(tone(BORDER))
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
                .bg(tone(PANEL))
                .border_color(tone(BORDER))
                .text_color(tone(TEXT)),
        )
}

pub(crate) fn subtitle(text: impl Into<String>) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(tone(MUTED))
        .child(gpui_kit::SharedString::from(text.into()))
}

pub(crate) fn title(text: impl Into<String>) -> gpui_kit::Div {
    div()
        .text_xl()
        .font_weight(FontWeight::BOLD)
        .child(gpui_kit::SharedString::from(text.into()))
}


impl AppState {
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
            this.child(div().text_sm().text_color(tone(DANGER)).child(error))
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

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_downloads.len();
        let summary = if active > 0 {
            format!(
                "下载中 {} · 新增{} / 跳过{} / 失败{}",
                active, self.downloaded, self.skipped, self.failed
            )
        } else {
            format!(
                "新增{} / 跳过{} / 失败{}",
                self.downloaded, self.skipped, self.failed
            )
        };
        let list_height = (self.download_tasks.len() as f32 * 34.0).clamp(34.0, 170.0);
        let rows = self
            .download_tasks
            .iter()
            .map(|task| self.download_task_row(task, cx));
        div()
            .min_h(px(56.0))
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .bg(tone(PANEL))
            .border_1()
            .border_color(tone(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(if self.error.is_some() { tone(DANGER) } else { tone(MUTED) })
                            .child(
                                self.error
                                    .clone()
                                    .unwrap_or_else(|| "下载进度".to_string())
                                    .chars()
                                    .take(300)
                                    .collect::<String>(),
                            ),
                    )
                    .child(subtitle(summary).flex_none()),
            )
            .when(!self.download_tasks.is_empty(), |this| {
                this.child(div().h(px(1.0)).w_full().bg(tone(BORDER))).child(
                    div()
                        .flex()
                        .w_full()
                        .child(
                            div()
                                .id("download-task-scroll")
                                .flex_1()
                                .min_w_0()
                                .h(px(list_height))
                                .overflow_y_scroll()
                                .track_scroll(&self.download_scroll)
                                .child(div().flex().flex_col().children(rows)),
                        )
                        .child(crate::ui_scroll::vertical_rail(
                            "download-task-scrollbar",
                            &self.download_scroll,
                        )),
                )
            })
    }

    fn download_task_row(
        &self,
        task: &DownloadTask,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fraction = if task.total == 0 {
            if matches!(
                task.state,
                DownloadTaskState::Done | DownloadTaskState::Skipped
            ) {
                1.0
            } else {
                0.0
            }
        } else {
            (task.downloaded as f32 / task.total as f32).clamp(0.0, 1.0)
        };
        let (label, color) = match task.state {
            DownloadTaskState::Queued => ("排队", MUTED),
            DownloadTaskState::Downloading => ("下载", PRIMARY),
            DownloadTaskState::Paused => ("暂停", WARN),
            DownloadTaskState::Merging => ("合并", WARN),
            DownloadTaskState::Done => ("完成", SUCCESS),
            DownloadTaskState::Skipped => ("跳过", MUTED),
            DownloadTaskState::Failed => ("失败", DANGER),
        };
        let mut actions: Vec<AnyElement> = Vec::new();
        if task.state == DownloadTaskState::Downloading {
            let task_id = task.id.clone();
            actions.push(
                Button::new(ElementId::Name(
                    format!("download-pause-{}", task.id).into(),
                ))
                .icon(IconName::Pause)
                .ghost()
                .accessibility_label("暂停")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pause_download(&task_id, cx);
                }))
                .into_any_element(),
            );
        }
        if matches!(
            task.state,
            DownloadTaskState::Paused | DownloadTaskState::Failed
        ) {
            let task_id = task.id.clone();
            actions.push(
                Button::new(ElementId::Name(
                    format!("download-resume-{}", task.id).into(),
                ))
                .icon(IconName::Play)
                .ghost()
                .accessibility_label("继续")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.resume_download(&task_id, cx);
                }))
                .into_any_element(),
            );
        }
        if matches!(
            task.state,
            DownloadTaskState::Queued
                | DownloadTaskState::Downloading
                | DownloadTaskState::Paused
                | DownloadTaskState::Merging
        ) {
            let task_id = task.id.clone();
            actions.push(
                Button::new(ElementId::Name(
                    format!("download-stop-{}", task.id).into(),
                ))
                .icon(IconName::Square)
                .ghost()
                .accessibility_label("停止")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.stop_download(&task_id, cx);
                }))
                .into_any_element(),
            );
        }
        div()
            .h(px(34.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(280.0))
                    .flex_none()
                    .truncate()
                    .text_sm()
                    .child(task.title.clone()),
            )
            .child(
                div()
                    .w(px(40.0))
                    .flex_none()
                    .text_sm()
                    .text_color(tone(color))
                    .child(label),
            )
            .child(
                div()
                    .w(px(130.0))
                    .flex_none()
                    .h(px(6.0))
                    .bg(tone(BORDER))
                    .rounded_full()
                    .child(
                        div()
                            .w(px(130.0 * fraction))
                            .h(px(6.0))
                            .bg(tone(color))
                            .rounded_full(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone(MUTED))
                    .child(task.detail.clone()),
            )
            .child(div().flex().items_center().gap_1().children(actions))
            .into_any_element()
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logged_in = self.http.is_some();
        let content = if logged_in {
            self.body(window, cx).into_any_element()
        } else {
            self.login_view(cx).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(tone(BG))
            .text_color(tone(TEXT))
            .child(
                gpui_component::TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("nav-back")
                                .icon(IconName::ArrowLeft)
                                .ghost()
                                .small()
                                .disabled(!self.can_go_back())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.go_back(window, cx);
                                })),
                        )
                        .child(
                            Button::new("nav-forward")
                                .icon(IconName::ArrowRight)
                                .ghost()
                                .small()
                                .disabled(!self.can_go_forward())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.go_forward(window, cx);
                                })),
                        )
                        .child(
                            div()
                                .pl_2()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Course Share"),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.course_sidebar(window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().flex_1().min_h_0().child(content))
                            .when(self.download_footer_visible(), |this| {
                                this.child(self.footer(cx))
                            }),
                    ),
            )
            .when(logged_in && self.pending_login.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(tone(BG))
                        .child(self.login_view(cx)),
                )
            })
    }
}
