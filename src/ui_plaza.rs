use crate::app::AppState;
use crate::smooth_scroll::{SmoothTarget, SmoothWheel};
use crate::theme::tone;
use crate::ui::{detail, title, BORDER, PANEL, PRIMARY};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::select::Select;
use gpui_component::IconName;
use gpui_component::spinner::Spinner;
use gpui_component::Disableable as _;
use gpui_component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, Context, ElementId, FontWeight, IntoElement, Window,
};
use std::rc::Rc;

impl AppState {
    pub(crate) fn plaza_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if let Some(key) = self.plaza_selected_course.clone() {
            if let Some(course) = self
                .plaza_courses
                .iter()
                .find(|course| course.key == key)
                .cloned()
            {
                return self.plaza_detail(&course, window, cx).into_any_element();
            }
        }
        self.plaza_grid(window, cx).into_any_element()
    }

    fn plaza_grid(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let courses = self.plaza_courses.clone();
        let columns = plaza_columns(window);
        let row_count = if courses.is_empty() {
            0
        } else {
            courses.len().div_ceil(columns)
        };
        let total = self.plaza_total;
        let footer = if courses.is_empty() {
            None
        } else if self.plaza_loading_more {
            Some(PlazaFooter::Loading)
        } else if self.plaza_has_more {
            Some(PlazaFooter::LoadMore)
        } else {
            Some(PlazaFooter::Done)
        };
        let mut sizes = vec![gpui_kit::size(px(300.0), px(160.0)); row_count];
        if let Some(kind) = footer {
            sizes.push(gpui_kit::size(
                px(300.0),
                px(match kind {
                    PlazaFooter::Loading => 44.0,
                    PlazaFooter::LoadMore => 52.0,
                    PlazaFooter::Done => 40.0,
                }),
            ));
        }
        let courses_for_list = std::rc::Rc::new(courses.clone());
        let list = gpui_kit::base::v_virtual_list(
            cx.entity(),
            "plaza-virtual-grid",
            std::rc::Rc::new(sizes),
            move |this, range, _, cx| {
                let courses = courses_for_list.clone();
                range
                    .map(|index| {
                        if index < row_count {
                            let start = index * columns;
                            let end = (start + columns).min(courses.len());
                            let cards = (start..end)
                                .map(|card_index| {
                                    this.plaza_card(&courses[card_index], card_index, cx)
                                })
                                .collect::<Vec<_>>();
                            div()
                                .h(px(160.0))
                                .w_full()
                                .flex()
                                .justify_center()
                                .gap_4()
                                .px_6()
                                .pt_3()
                                .children(cards)
                                .into_any_element()
                        } else {
                            match footer {
                                Some(PlazaFooter::Loading) => div()
                                    .h(px(44.0))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(Spinner::new().small())
                                    .child(detail("加载更多…"))
                                    .into_any_element(),
                                Some(PlazaFooter::LoadMore) => div()
                                    .h(px(52.0))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Button::new("plaza-load-more")
                                            .label("加载更多")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.load_more_plaza(cx);
                                            })),
                                    )
                                    .into_any_element(),
                                Some(PlazaFooter::Done) => div()
                                    .h(px(40.0))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(detail(format!("已加载全部 {total} 门课程")))
                                    .into_any_element(),
                                None => div().into_any_element(),
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.plaza_grid_scroll);

        let target = SmoothTarget::virtual_list(&self.plaza_grid_scroll);
        let view = cx.entity();
        let viewport = SmoothWheel::new(
            div().flex_1().min_w_0().h_full().flex().child(list),
            Rc::new(move |event, _window, cx| {
                let delta = event.delta.pixel_delta(px(18.)).y;
                let (current, min) = target.offset_bounds();
                let remaining = current - min;
                let target = target.clone();
                view.update(cx, |this, cx| {
                    this.smooth_wheel("plaza-grid", target, event, cx);
                    if delta < px(0.0)
                        && remaining < px(480.0)
                        && this.plaza_has_more
                        && !this.plaza_loading_more
                    {
                        this.load_more_plaza(cx);
                    }
                });
                true
            }),
        )
        .into_any_element();

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .min_h(px(64.0))
                    .flex_none()
                    .px_5()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(tone(BORDER))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .children(self.plaza_term_select.as_ref().map(|state| {
                                div().w(px(172.0)).flex_none().child(
                                    Select::new(state)
                                        .placeholder("全部学期")
                                        .search_placeholder("搜索学期…")
                                        .menu_width(px(220.0)),
                                )
                            }))
                            .children(self.plaza_college_select.as_ref().map(|state| {
                                div().w(px(160.0)).flex_none().child(
                                    Select::new(state)
                                        .placeholder("全部学院")
                                        .search_placeholder("搜索学院…")
                                        .menu_width(px(260.0)),
                                )
                            }))
                            .children(self.plaza_schedule_select.as_ref().map(|state| {
                                div().w(px(150.0)).flex_none().child(
                                    Select::new(state)
                                        .placeholder("全部课表")
                                        .search_placeholder("搜索课表…")
                                        .menu_width(px(180.0)),
                                )
                            }))
                            .child(
                                div()
                                    .w(px(300.0))
                                    .flex_none()
                                    .child(crate::ui::input_field(&self.plaza_search, cx)),
                            )
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
                    .child(viewport)
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
                        .child(detail("广场暂时没有分享。可在“课程”里点击“发布到广场”。")),
                )
            })
            .when(self.plaza_scrolled, |this| {
                this.child(
                    div()
                        .absolute()
                        .right(px(24.0))
                        .bottom(px(24.0))
                        .child(
                            Button::new("plaza-back-to-top")
                                .icon(IconName::ArrowUp)
                                .primary()
                                .accessibility_label("回到顶部")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.scroll_plaza_top(cx);
                                })),
                        ),
                )
            })
    }

    fn plaza_card(&self, course: &crate::share::CourseGroup, index: usize, cx: &mut Context<Self>) -> AnyElement {
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
        let mut details = Vec::new();
        if !course.college.is_empty() {
            details.push(course.college.clone());
        }
        if !course.credits.is_empty() {
            details.push(format!("{} 学分", course.credits));
        }
        if !course.course_type.is_empty() {
            details.push(course.course_type.clone());
        }
        if !course.schedule_type.is_empty() {
            details.push(course.schedule_type.clone());
        }
        let mut card = div()
            .id(ElementId::Name(format!("plaza-card-{index}").into()))
            .w(px(300.0))
            .h(px(144.0))
            .p_4()
            .flex()
            .flex_col()
            .gap_1()
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
                    .flex_shrink_0()
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
            card = card.child(detail(meta.join(" · ")).truncate().flex_shrink_0());
        }
        if !details.is_empty() {
            card = card.child(detail(details.join(" · ")).truncate().flex_shrink_0());
        }
        card.child(
            detail(format!("{} 条回放", course.count)).flex_shrink_0(),
        )
        .into_any_element()
    }

    fn plaza_detail(
        &mut self,
        course: &crate::share::CourseGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = self.plaza_shares.len();
        let shares = self.plaza_shares.clone();
        let table = self.ensure_plaza_table(window, &shares, cx);
        let table_view = crate::replay_table::plaza_table_element(&table, cx);

        let starred = self.is_course_starred(&course.key, &course.course_name);
        let star_label = if starred { "★" } else { "☆" };
        let key_for_star = course.key.clone();
        let name_for_star = course.course_name.clone();
        let mut meta = Vec::new();
        if !course.term.is_empty() {
            meta.push(course.term.clone());
        }
        if !course.teacher.is_empty() {
            meta.push(course.teacher.clone());
        }
        if !course.college.is_empty() {
            meta.push(course.college.clone());
        }
        if !course.credits.is_empty() {
            meta.push(format!("{} 学分", course.credits));
        }
        if !course.course_type.is_empty() {
            meta.push(course.course_type.clone());
        }
        if !course.schedule_type.is_empty() {
            meta.push(course.schedule_type.clone());
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
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(title(course.course_name.clone()))
                                    .child(
                                        div()
                                            .flex_none()
                                            .child(detail(format!("{} 条回放", course.count))),
                                    ),
                            )
                            .when(!meta.is_empty(), |this| {
                                this.child(
                                    detail(meta.join(" · ")).truncate().flex_shrink_0(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("plaza-detail-download-all")
                                    .label("下载全部")
                                    .primary()
                                    .disabled(self.plaza_shares.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.download_plaza_course(window, cx);
                                    })),
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
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(table_view),
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
                        .child(detail("正在加载回放…")),
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
                        .child(detail("该课程暂时没有可下载的回放。")),
                )
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlazaFooter {
    Loading,
    LoadMore,
    Done,
}

fn plaza_columns(window: &Window) -> usize {
    let width = f32::from(window.viewport_size().width);
    let available = (width - 300.0 - 48.0 - 16.0).max(0.0);
    ((available + 16.0) / 316.0).floor().max(1.0) as usize
}
