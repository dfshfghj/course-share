use crate::app::AppState;
use crate::share::Share;
use crate::theme::tone;
use crate::ui::{subtitle, title, BORDER, MUTED, PANEL, PRIMARY};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::spinner::Spinner;
use gpui_component::Disableable as _;
use gpui_component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, uniform_list, AnyElement, Context, ElementId, FontWeight, IntoElement,
    ScrollWheelEvent, Window,
};

impl AppState {
    pub(crate) fn plaza_view(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if let Some(key) = self.plaza_selected_course.as_ref() {
            if let Some(course) = self.plaza_courses.iter().find(|course| &course.key == key) {
                return self.plaza_detail(course, cx).into_any_element();
            }
        }
        self.plaza_grid(cx).into_any_element()
    }

    fn plaza_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let courses = &self.plaza_courses;
        let mut cards = Vec::new();
        for (index, course) in courses.iter().enumerate() {
            let starred = self.is_course_starred(&course.key, &course.course_name);
            let star_label = if starred { "★" } else { "☆" };
            let key_for_star = course.key.clone();
            let name_for_star = course.course_name.clone();
            let key_for_open = course.key.clone();
            let mut meta = Vec::new();
            if !course.term.is_empty() {
                meta.push(course.term.clone());
            }
            if !course.teacher.is_empty() {
                meta.push(course.teacher.clone());
            }
            let mut card = div()
                .id(ElementId::Name(format!("plaza-card-{index}").into()))
                .w(px(300.0))
                .min_h(px(128.0))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .bg(tone(PANEL))
                .border_1()
                .border_color(tone(BORDER))
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.border_color(tone(PRIMARY)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_plaza_course(key_for_open.clone(), window, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(course.course_name.clone()),
                        )
                        .child(
                            Button::new(ElementId::Name(
                                format!("plaza-star-{index}").into(),
                            ))
                            .label(star_label)
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_star_course(
                                    key_for_star.clone(),
                                    name_for_star.clone(),
                                    cx,
                                );
                            })),
                        ),
                );
            if !meta.is_empty() {
                card = card.child(subtitle(meta.join(" · ")));
            }
            card = card.child(subtitle(format!("{} 条回放", course.count)));
            cards.push(card.into_any_element());
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.0))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(tone(BORDER))
                    .child(
                        div()
                            .w(px(520.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(crate::ui::input_field(&self.plaza_search, cx))
                            .child(
                                Button::new("plaza-search")
                                    .label("搜索")
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_plaza(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("plaza-grid-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.plaza_grid_scroll)
                            .on_scroll_wheel(cx.listener(
                                |this, _event: &ScrollWheelEvent, window, cx| {
                                    if !this.plaza_has_more {
                                        return;
                                    }
                                    let offset = this.plaza_grid_scroll.offset().y;
                                    let max = this.plaza_grid_scroll.max_offset().y;
                                    // offset is negative while scrolling down;
                                    // the remaining distance is max + offset.
                                    if f32::from(max + offset) < 240.0 {
                                        this.load_more_plaza(window, cx);
                                    }
                                },
                            ))
                            .child(
                                div()
                                    .p_6()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .justify_center()
                                            .gap_4()
                                            .children(cards),
                                    )
                                    .when(self.plaza_loading_more, |this| {
                                        this.child(
                                            div()
                                                .h(px(44.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .gap_2()
                                                .child(Spinner::new().small())
                                                .child(subtitle("加载更多…")),
                                        )
                                    })
                                    .when(
                                        !self.plaza_loading_more && self.plaza_has_more,
                                        |this| {
                                            this.child(
                                                div().flex().justify_center().child(
                                                    Button::new("plaza-load-more")
                                                        .label("加载更多")
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.load_more_plaza(window, cx);
                                                            },
                                                        )),
                                                ),
                                            )
                                        },
                                    )
                                    .when(
                                        !self.plaza_has_more && !courses.is_empty(),
                                        |this| {
                                            this.child(
                                                div().flex().justify_center().child(subtitle(
                                                    format!(
                                                        "已加载全部 {} 门课程",
                                                        self.plaza_total
                                                    ),
                                                )),
                                            )
                                        },
                                    ),
                            ),
                    )
                    .child(crate::ui_scroll::vertical_rail(
                        "plaza-grid-scrollbar",
                        &self.plaza_grid_scroll,
                    )),
            )
            .when(courses.is_empty(), |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(subtitle("广场暂时没有分享。可在“课程”里点击“发布到广场”。")),
                )
            })
    }

    fn plaza_detail(
        &self,
        course: &crate::share::CourseGroup,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = self.plaza_shares.len();
        let shares_for_list = self.plaza_shares.clone();
        let view = cx.entity().downgrade();
        let list = uniform_list("plaza-replay-list", count, move |range, _, cx| {
            let shares = shares_for_list.clone();
            view.update(cx, |this, cx| {
                range
                    .map(|index| this.plaza_replay_row(&shares[index], index, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .flex_1()
        .min_h_0()
        .track_scroll(&self.plaza_detail_scroll);

        let starred = self.is_course_starred(&course.key, &course.course_name);
        let star_label = if starred { "★" } else { "☆" };
        let key_for_star = course.key.clone();
        let name_for_star = course.course_name.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.0))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(tone(BORDER))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(title(course.course_name.clone()))
                            .child(
                                div()
                                    .flex_none()
                                    .child(subtitle(format!("{} 条回放", course.count))),
                            ),
                    )
                    .child(
                        Button::new("plaza-detail-star")
                            .label(star_label)
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_star_course(
                                    key_for_star.clone(),
                                    name_for_star.clone(),
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(list)
                    .child(crate::ui_scroll::uniform_rail(
                        "plaza-detail-scrollbar",
                        &self.plaza_detail_scroll,
                    )),
            )
            .when(self.plaza_detail_loading && count == 0, |this| {
                this.child(
                    div()
                        .h(px(44.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Spinner::new().small())
                        .child(subtitle("正在加载回放…")),
                )
            })
            .when(!self.plaza_detail_loading && count == 0, |this| {
                this.child(
                    div()
                        .h(px(44.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(subtitle("该课程暂时没有可下载的回放。")),
                )
            })
    }

    fn plaza_replay_row(
        &self,
        share: &Share,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let share_for_download = share.clone();
        let share_for_preview = share.clone();
        let url_for_copy = share.m3u8_url.clone();
        let time = if !share.record_time.trim().is_empty() {
            share.record_time.clone()
        } else if !share.record_date.trim().is_empty() {
            share.record_date.clone()
        } else {
            "—".to_string()
        };
        div()
            .h(px(56.0))
            .w_full()
            .flex_none()
            .px_5()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(tone(BORDER))
            .hover(|style| style.bg(tone(0xf8fafc)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(share.replay_title.clone()),
            )
            .child(
                div()
                    .w(px(170.0))
                    .flex_none()
                    .text_sm()
                    .text_color(tone(MUTED))
                    .truncate()
                    .child(time),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(subtitle(format!("下载 {}", share.downloads)))
                    .child(
                        Button::new(ElementId::Name(
                            format!("plaza-preview-{index}").into(),
                        ))
                        .label("预览")
                        .ghost()
                        .disabled(share.m3u8_url.is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preview_share(share_for_preview.clone(), cx);
                        })),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("plaza-download-{index}").into(),
                        ))
                        .label("下载")
                        .primary()
                        .disabled(share.m3u8_url.is_empty())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.download_share(share_for_download.clone(), window, cx);
                        })),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("plaza-copy-{index}").into(),
                        ))
                        .label("复制链接")
                        .ghost()
                        .disabled(url_for_copy.is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_text(url_for_copy.clone(), cx);
                        })),
                    ),
            )
            .into_any_element()
    }
}
