use crate::app::{AppState, SidebarSection, Tab};
use crate::smooth_scroll::SmoothTarget;
use crate::theme::tone;
use crate::ui::{detail, title, BORDER, DANGER, MUTED, PANEL, PRIMARY, PRIMARY_SOFT, SUCCESS, TEXT};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::spinner::Spinner;
use gpui_component::Disableable as _;
use gpui_component::{IconName, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, uniform_list, AnyElement, Context, ElementId, FontWeight, IntoElement, Window,
};

#[derive(Clone)]
enum SidebarRow {
    Section {
        section: SidebarSection,
        count: usize,
        collapsed: bool,
    },
    Group {
        term: String,
        count: usize,
        collapsed: bool,
    },
    Enrolled(usize),
    Favorite { key: String, name: String },
}

impl AppState {
    pub(crate) fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            Tab::Courses => self.courses_view(window, cx).into_any_element(),
            Tab::Plaza => self.plaza_view(window, cx).into_any_element(),
            Tab::Settings => self.settings_view(window, cx).into_any_element(),
        }
    }

    fn courses_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let right = if let Some(key) = &self.favorite_selected {
            self.favorite_replay_list(key.clone(), window, cx).into_any_element()
        } else {
            self.replay_list(window, cx).into_any_element()
        };
        div().size_full().flex().child(right)
    }

    fn favorite_courses(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<(String, String, String)> = Vec::new();
        for share in &self.favorite_shares {
            let key = share.course_key();
            if self.is_course_starred(&key, &share.course_name)
                && !out.iter().any(|(existing, _, _)| *existing == key)
            {
                out.push((key, share.course_name.clone(), share.term.clone()));
            }
        }
        for course in &self.courses {
            let key = course.key();
            if self.is_course_starred(&key, &course.name)
                && !out.iter().any(|(existing, _, _)| *existing == key)
            {
                out.push((key, course.name.clone(), course.term.clone()));
            }
        }
        // Legacy shares without course_id use name:<name>; prefer the enrolled
        // course's id-keyed entry when both refer to the same course.
        let id_keyed_names: Vec<String> = out
            .iter()
            .filter(|(key, _, _)| key.starts_with("id:"))
            .map(|(_, name, _)| name.clone())
            .collect();
        out.retain(|(key, name, _)| {
            !key.starts_with("name:") || !id_keyed_names.contains(name)
        });
        out
    }

    fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let mut rows = Vec::new();

        let courses_collapsed = self.collapsed_sections.contains("courses");
        rows.push(SidebarRow::Section {
            section: SidebarSection::Courses,
            count: self.courses.len(),
            collapsed: courses_collapsed,
        });
        if !courses_collapsed {
            let mut groups: Vec<(String, Vec<SidebarRow>)> = Vec::new();
            for (index, course) in self.courses.iter().enumerate() {
                let term = if course.term.trim().is_empty() {
                    "未知学期".to_string()
                } else {
                    course.term.clone()
                };
                if let Some((_, rows)) = groups.iter_mut().find(|(name, _)| *name == term) {
                    rows.push(SidebarRow::Enrolled(index));
                } else {
                    groups.push((term, vec![SidebarRow::Enrolled(index)]));
                }
            }
            groups.sort_by(|(a, _), (b, _)| term_order(b).cmp(&term_order(a)));
            for (term, courses) in groups {
                let collapsed = self.collapsed_terms.contains(&term);
                rows.push(SidebarRow::Group {
                    term: term.clone(),
                    count: courses.len(),
                    collapsed,
                });
                if !collapsed {
                    rows.extend(courses);
                }
            }
        }

        let favorites = self.favorite_courses();
        let favorites_collapsed = self.collapsed_sections.contains("favorites");
        rows.push(SidebarRow::Section {
            section: SidebarSection::Favorites,
            count: favorites.len(),
            collapsed: favorites_collapsed,
        });
        if !favorites_collapsed {
            let mut groups: Vec<(String, Vec<SidebarRow>)> = Vec::new();
            for (key, name, term) in favorites {
                let term = if term.trim().is_empty() {
                    "未知学期".to_string()
                } else {
                    term
                };
                let row = SidebarRow::Favorite { key, name };
                if let Some((_, rows)) = groups.iter_mut().find(|(t, _)| *t == term) {
                    rows.push(row);
                } else {
                    groups.push((term, vec![row]));
                }
            }
            groups.sort_by(|(a, _), (b, _)| term_order(b).cmp(&term_order(a)));
            for (term, courses) in groups {
                let collapsed = self.collapsed_terms.contains(&term);
                rows.push(SidebarRow::Group {
                    term: term.clone(),
                    count: courses.len(),
                    collapsed,
                });
                if !collapsed {
                    rows.extend(courses);
                }
            }
        }
        rows
    }

    pub(crate) fn course_sidebar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.sidebar_rows();
        let row_count = rows.len();
        let rows_for_list = rows.clone();
        let view = cx.entity().downgrade();
        let list = uniform_list("course-virtual-list", row_count, move |range, _, cx| {
            let rows = rows_for_list.clone();
            view.update(cx, |this, cx| {
                rows[range]
                    .iter()
                    .map(|row| this.sidebar_row(row, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .flex_1()
        .min_h_0()
        .track_scroll(&self.course_scroll);
        let viewport = self.smooth_viewport(
            "sidebar-courses",
            SmoothTarget::uniform(&self.course_scroll),
            list,
            cx,
        );

        div()
            .w(px(300.0))
            .h_full()
            .flex()
            .flex_col()
            .flex_none()
            .bg(tone(PANEL))
            .border_r_1()
            .border_color(tone(BORDER))
            .child(self.plaza_entry(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .ml_2()
                    .overflow_hidden()
                    .child(viewport)
                    .child(crate::ui_scroll::uniform_rail(
                        "course-scrollbar",
                        &self.course_scroll,
                    )),
            )
            .child(self.sidebar_footer(cx))
    }

    fn plaza_entry(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.tab == Tab::Plaza;
        div()
            .id("sidebar-plaza")
            .h(px(40.0))
            .flex_1()
            .flex_none()
            .px_4()
            .m_2()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .rounded_sm()
            .when(selected, |this| this.bg(tone(PRIMARY_SOFT)))
            .when(!selected, |this| this.hover(|style| style.bg(tone(0xf1f5f9))))
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_plaza(window, cx);
            }))
            .child(
                div()
                    .text_sm()
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if selected { tone(PRIMARY) } else { tone(TEXT) })
                    .child("广场"),
            )
            .into_any_element()
    }

    fn sidebar_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let logged_in = self.http.is_some();
        let settings_selected = self.tab == Tab::Settings;
        div()
            .flex_none()
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .border_t_1()
            .border_color(tone(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .bg(tone(if logged_in { SUCCESS } else { MUTED })),
                    )
                    .child(detail(if logged_in { "已登录" } else { "未登录" })),
            )
            .child(
                {
                    let button = Button::new("sidebar-settings")
                        .icon(IconName::Settings)
                        .accessibility_label("设置");
                    let button = if settings_selected {
                        button.primary()
                    } else {
                        button.ghost()
                    };
                    button.on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                    }))
                },
            )
            .into_any_element()
    }

    fn sidebar_row(&self, row: &SidebarRow, cx: &mut Context<Self>) -> AnyElement {
        match row {
            SidebarRow::Section {
                section,
                count,
                collapsed,
            } => {
                let section = *section;
                let label = match section {
                    SidebarSection::Courses => "我的课程",
                    SidebarSection::Favorites => "关注",
                };
                div()
                    .id(ElementId::Name(format!("section-{label}").into()))
                    .h(px(40.0))
                    .w_full()
                    .m_2()
                    .rounded_sm()
                    .flex_none()
                    .pl_3()
                    .pr_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|style| style.bg(tone(0xf1f5f9)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_sidebar_section(section, window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .m_2()
                            .rounded_sm()
                            .text_sm()
                            .text_color(tone(MUTED))
                            .child(label),
                    )
                    .child(detail(format!("{count}")))
                    .child(
                        div().pl_2().text_color(tone(MUTED)).child(if *collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        }),
                    )
                    .into_any_element()
            }
            SidebarRow::Group {
                term,
                count,
                collapsed,
            } => {
                let term_for_click = term.clone();
                div()
                    .id(ElementId::Name(format!("term-{term}").into()))
                    .h(px(36.0))
                    .w_full()
                    .m_2()
                    .rounded_sm()
                    .flex_none()
                    .pl_6()
                    .pr_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|style| style.bg(tone(0xf1f5f9)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_term(term_for_click.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(tone(TEXT))
                            .child(term.clone()),
                    )
                    .child(detail(format!("{count}")))
                    .child(
                        div().pl_2().text_color(tone(MUTED)).child(if *collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        }),
                    )
                    .into_any_element()
            }
            SidebarRow::Enrolled(index) => {
                let course_index = *index;
                let Some(course) = self.courses.get(course_index) else {
                    return div().into_any_element();
                };
                let selected = self.selected_course == Some(course_index);
                self.course_row(
                    ElementId::Name(format!("course-{course_index}").into()),
                    course.name.clone(),
                    selected,
                    cx.listener(move |this, _, window, cx| {
                        this.select_course(course_index, window, cx);
                    }),
                )
            }
            SidebarRow::Favorite { key, name } => {
                let favorite_key = key.clone();
                let favorite_name = name.clone();
                let selected = self.favorite_selected.as_deref() == Some(key.as_str());
                self.course_row(
                    ElementId::Name(format!("favorite-{key}").into()),
                    favorite_name,
                    selected,
                    cx.listener(move |this, _, window, cx| {
                        this.open_favorite_course(favorite_key.clone(), window, cx);
                    }),
                )
            }
        }
    }

    fn course_row(
        &self,
        id: ElementId,
        name: String,
        selected: bool,
        on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .h(px(36.0))
            .w_full()
            .m_2()
            .rounded_sm()
            .flex_none()
            .pl(px(40.0))
            .pr_4()
            .flex()
            .items_center()
            .cursor_pointer()
            .when(selected, |this| this.bg(tone(PRIMARY_SOFT)))
            .when(!selected, |this| this.hover(|style| style.bg(tone(0xf1f5f9))))
            .on_click(on_click)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .truncate()
                    .child(name),
            )
            .into_any_element()
    }

    fn favorite_replay_list(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let replays = self.favorite_replays_for(&key);
        let count = replays.len();
        let pending_count = replays
            .iter()
            .filter(|replay| {
                !self.replay_downloaded(replay) && !self.replay_in_progress(replay)
            })
            .count();
        let name = self
            .favorite_shares
            .iter()
            .find(|share| share.course_key() == key)
            .map(|share| share.course_name.clone())
            .unwrap_or_else(|| {
                key.trim_start_matches("id:")
                    .trim_start_matches("name:")
                    .to_string()
            });
        let term = self
            .favorite_shares
            .iter()
            .find(|share| share.course_key() == key && !share.term.is_empty())
            .map(|share| share.term.clone())
            .unwrap_or_default();
        let teacher = self
            .favorite_shares
            .iter()
            .find(|share| share.course_key() == key && !share.teacher.is_empty())
            .map(|share| share.teacher.clone())
            .unwrap_or_default();
        let table = self.ensure_course_table(
            window,
            &format!("favorite:{key}"),
            &replays,
            cx,
        );
        let table_view = crate::replay_table::course_table_element(&table, cx);

        let mut meta = Vec::new();
        if !term.is_empty() {
            meta.push(term);
        }
        if !teacher.is_empty() {
            meta.push(teacher);
        }
        let mut title_block = div()
            .flex()
            .items_center()
            .gap_3()
            .child(title(name));
        if !meta.is_empty() {
            title_block = title_block.child(detail(meta.join(" · ")));
        }
        let name_for_download = self.favorite_selected.clone().unwrap_or_default();

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(80.0))
                    .flex_none()
                    .px_5()
                    .py_4()
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
                            .child(title_block)
                            .child(detail(format!("{count} 条回放"))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("refresh-favorite")
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_shares(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("download-favorite-all")
                                    .label("下载全部")
                                    .primary()
                                    .disabled(pending_count == 0)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.download_favorite_all(&name_for_download, window, cx);
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
                    .pt_4()
                    .child(table_view),
            )
            .when(count == 0, |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(detail(if self.busy {
                            "正在加载收藏回放…"
                        } else {
                            "该收藏课程暂时没有可下载的回放。"
                        })),
                )
            })
    }

    fn replay_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(course) = self.selected_course().cloned() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(detail("请选择左侧课程"));
        };
        let replays = self.replays.clone();
        let course_name = course.name.clone();
        let term = course.term.clone();
        let pending_count = replays
            .iter()
            .filter(|replay| {
                !self.replay_downloaded(replay) && !self.replay_in_progress(replay)
            })
            .count();
        let table = self.ensure_course_table(
            window,
            &format!("course:{}", course.id),
            &replays,
            cx,
        );
        let table_view = crate::replay_table::course_table_element(&table, cx);

        let mut title_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title(course_name));
        if !term.is_empty() {
            title_block = title_block.child(detail(term));
        }

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(80.0))
                    .flex_none()
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(title_block)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("refresh-replays")
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_replays(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("download-all")
                                    .label("下载全部")
                                    .primary()
                                    .disabled(pending_count == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.download_all(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("share-course")
                                    .label("发布到广场")
                                    .disabled(self.busy || replays.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.share_course(window, cx);
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
                    .pt_4()
                    .child(table_view),
            )
            .when(self.replays_loading, |this| {
                this.child(
                    div()
                        .h(px(44.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Spinner::new().small())
                        .child(detail(self.replays_loading_label.clone())),
                )
            })
            .when(!self.replays_loading && self.replays.is_empty(), |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_sm()
                                .text_color(tone(DANGER))
                                .child("没有解析到回放。"),
                        ),
                )
            })
    }
}

fn term_order(term: &str) -> (i32, i32) {
    static RANGE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(\d{2,4})\s*[-–—~至]\s*(\d{2,4})").unwrap()
    });
    static YEAR: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?:19|20)\d{2}").unwrap());
    static SHORT_YEAR: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\s*(\d{2})").unwrap());
    static SEMESTER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"第\s*([1-3一二三])\s*学期").unwrap()
    });

    let semester = || {
        if term.contains('秋') {
            1
        } else if term.contains('春') {
            2
        } else if term.contains('夏') {
            3
        } else if let Some(captured) = SEMESTER.captures(term) {
            match &captured[1] {
                "1" | "一" => 1,
                "2" | "二" => 2,
                "3" | "三" => 3,
                _ => 0,
            }
        } else {
            0
        }
    };

    if let Some(captured) = RANGE.captures(term) {
        let start = normalize_year(&captured[1]);
        let mut end = normalize_year(&captured[2]);
        if end <= start {
            end += 100;
        }
        return (end, semester());
    }

    if let Some(years) = YEAR.find(term) {
        let year = years.as_str().parse::<i32>().unwrap_or(0);
        if term.contains('秋') {
            return (year + 1, 1);
        }
        return (year, semester());
    }

    if let Some(captured) = SHORT_YEAR.captures(term) {
        let year = normalize_year(&captured[1]);
        if term.contains('秋') {
            return (year + 1, 1);
        }
        return (year, semester());
    }

    (0, 0)
}

fn normalize_year(value: &str) -> i32 {
    let year = value.trim().parse::<i32>().unwrap_or(0);
    if year < 100 {
        2000 + year
    } else {
        year
    }
}

#[cfg(test)]
mod tests {
    use super::term_order;

    #[test]
    fn orders_terms_from_newest_to_oldest() {
        let mut terms = vec![
            "24-25学年第2学期",
            "25-26学年第1学期",
            "2026春",
            "2025秋",
            "未知学期",
        ];
        terms.sort_by(|a, b| term_order(b).cmp(&term_order(a)));
        assert_eq!(terms[0], "2026春");
        assert_eq!(terms[terms.len() - 1], "未知学期");
        let old = terms.iter().position(|t| *t == "24-25学年第2学期").unwrap();
        let unknown = terms.iter().position(|t| *t == "未知学期").unwrap();
        assert!(old < unknown);
    }

    #[test]
    fn treats_spring_as_newer_than_fall_in_same_academic_year() {
        assert!(term_order("2026春") > term_order("2025秋"));
        assert!(term_order("25-26学年第2学期") > term_order("25-26学年第1学期"));
        assert!(term_order("25-26学年第1学期") > term_order("24-25学年第2学期"));
    }
}
