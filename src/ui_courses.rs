use crate::app::{AppState, SidebarSection, Tab};
use crate::ui::{subtitle, title, BORDER, DANGER, MUTED, PANEL, PRIMARY, PRIMARY_SOFT};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::spinner::Spinner;
use gpui_component::Disableable as _;
use gpui_component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, rgb, uniform_list, AnyElement, Context, ElementId, FontWeight, IntoElement, Window,
};

#[derive(Clone)]
enum SidebarRow {
    Group {
        term: String,
        count: usize,
        collapsed: bool,
    },
    Enrolled(usize),
    Favorite { key: String, name: String },
}

impl AppState {
    pub(crate) fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            Tab::Courses => self.courses_view(window, cx).into_any_element(),
            Tab::Plaza => self.plaza_view(window, cx).into_any_element(),
            Tab::Settings => self.settings_view(window, cx).into_any_element(),
        }
    }

    fn courses_view(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let right = if let Some(key) = &self.favorite_selected {
            self.favorite_replay_list(key.clone(), cx).into_any_element()
        } else {
            self.replay_list(window, cx).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .child(self.course_sidebar(window, cx))
            .child(right)
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
        let mut groups: Vec<(String, Vec<SidebarRow>)> = Vec::new();
        match self.sidebar_section {
            SidebarSection::Courses => {
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
            }
            SidebarSection::Favorites => {
                for (key, name, term) in self.favorite_courses() {
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
            }
        }

        let mut rows = Vec::new();
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
        rows
    }

    fn section_tab(
        &self,
        id: &'static str,
        label: &'static str,
        section: SidebarSection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.sidebar_section == section;
        div()
            .id(id)
            .h(px(36.0))
            .px_1()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_sidebar_section(section, window, cx);
            }))
            .child(
                div()
                    .text_sm()
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if selected {
                        rgb(PRIMARY)
                    } else {
                        rgb(MUTED)
                    })
                    .child(label),
            )
            .into_any_element()
    }

    fn course_sidebar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        let count = match self.sidebar_section {
            SidebarSection::Courses => self.courses.len(),
            SidebarSection::Favorites => self.favorite_courses().len(),
        };
        let empty_text = match self.sidebar_section {
            SidebarSection::Courses => "登录后会自动加载课程。",
            SidebarSection::Favorites => "在共享广场给课程点 ☆ 后，会出现在这里。",
        };

        div()
            .w(px(300.0))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .child(
                div()
                    .h(px(52.0))
                    .px_4()
                    .flex_none()
                    .flex()
                    .items_end()
                    .gap_4()
                    .child(self.section_tab(
                        "section-courses",
                        "课程",
                        SidebarSection::Courses,
                        cx,
                    ))
                    .child(self.section_tab(
                        "section-favorites",
                        "收藏",
                        SidebarSection::Favorites,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(div().pb_2().child(subtitle(format!("{count}")))),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(list)
                    .child(crate::ui_scroll::uniform_rail(
                        "course-scrollbar",
                        &self.course_scroll,
                    )),
            )
            .when(row_count == 0, |this| {
                this.child(
                    div().px_4().py_3().child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(empty_text),
                    ),
                )
            })
    }

    fn sidebar_row(&self, row: &SidebarRow, cx: &mut Context<Self>) -> AnyElement {
        match row {
            SidebarRow::Group {
                term,
                count,
                collapsed,
            } => {
                let term_for_click = term.clone();
                let arrow = if *collapsed { "▸" } else { "▾" };
                div()
                    .id(ElementId::Name(format!("term-{term}").into()))
                    .h(px(36.0))
                    .w_full()
                    .flex_none()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(rgb(0xf8fafc))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0xf1f5f9)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_term(term_for_click.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().text_color(rgb(0x64748b)).child(arrow))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_sm()
                                    .truncate()
                                    .child(term.clone()),
                            ),
                    )
                    .child(subtitle(format!("{count}")))
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
            .flex_none()
            .pl(px(28.0))
            .pr_4()
            .flex()
            .items_center()
            .cursor_pointer()
            .when(selected, |this| this.bg(rgb(PRIMARY_SOFT)))
            .when(!selected, |this| this.hover(|style| style.bg(rgb(0xf1f5f9))))
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

    fn favorite_replay_list(&self, key: String, cx: &mut Context<Self>) -> impl IntoElement {
        let replays = self.favorite_replays_for(&key);
        let count = replays.len();
        let pending_count = replays
            .iter()
            .filter(|replay| !self.replay_downloaded(replay))
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
        let replays_for_list = replays.clone();
        let view = cx.entity().downgrade();
        let list = uniform_list("favorite-replay-list", count, move |range, _, cx| {
            let replays = replays_for_list.clone();
            view.update(cx, |this, cx| {
                range
                    .map(|index| this.replay_row(&replays[index], index, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .flex_1()
        .min_h_0()
        .track_scroll(&self.favorite_scroll);

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
            title_block = title_block.child(subtitle(meta.join(" · ")));
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
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(title_block)
                            .child(subtitle(format!("{count} 条回放"))),
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
                                    .disabled(self.busy || pending_count == 0)
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
                    .child(list)
                    .child(crate::ui_scroll::uniform_rail(
                        "favorite-replay-scrollbar",
                        &self.favorite_scroll,
                    )),
            )
            .when(count == 0, |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(subtitle(if self.busy {
                            "正在加载收藏回放…"
                        } else {
                            "该收藏课程暂时没有可下载的回放。"
                        })),
                )
            })
    }

    fn replay_list(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(course) = self.selected_course() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(subtitle("请选择左侧课程"));
        };
        let replays = self.replays.clone();
        let course_name = course.name.clone();
        let term = course.term.clone();
        let pending_count = replays
            .iter()
            .filter(|replay| !self.replay_downloaded(replay))
            .count();
        let row_count = replays.len();
        let replays_for_list = replays.clone();
        let view = cx.entity().downgrade();
        let list = uniform_list("replay-virtual-list", row_count, move |range, _, cx| {
            let replays = replays_for_list.clone();
            view.update(cx, |this, cx| {
                range
                    .map(|index| this.replay_row(&replays[index], index, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .flex_1()
        .min_h_0()
        .track_scroll(&self.replay_scroll);

        let mut title_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title(course_name));
        if !term.is_empty() {
            title_block = title_block.child(subtitle(term));
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
                    .border_b_1()
                    .border_color(rgb(BORDER))
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
                                    .disabled(self.busy || pending_count == 0)
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
                    .child(list)
                    .child(crate::ui_scroll::uniform_rail(
                        "replay-scrollbar",
                        &self.replay_scroll,
                    )),
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
                        .child(subtitle(self.replays_loading_label.clone())),
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
                                .text_color(rgb(DANGER))
                                .child("没有解析到回放。"),
                        ),
                )
            })
    }

    fn replay_row(
        &self,
        replay: &crate::blackboard::Replay,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let downloaded = self.replay_downloaded(replay);
        let in_progress = self.replay_in_progress(replay);
        let has_cache = self.replay_has_cache(replay);
        let (label, class) = if downloaded {
            ("已下载", 2)
        } else if in_progress {
            ("下载中", 1)
        } else if has_cache {
            ("继续下载", 0)
        } else {
            ("下载", 0)
        };
        let button = Button::new(ElementId::Name(format!("download-{index}").into())).label(label);
        let button = match class {
            2 => button.success().disabled(true),
            1 => button.warning().disabled(true),
            _ => button.primary().disabled(self.busy),
        };
        let replay_for_download = replay.clone();
        let lesson = if replay.sub_title.is_empty() {
            replay.replay_id.clone()
        } else {
            replay.sub_title.clone()
        };
        let time = if replay.record_time.trim().is_empty() {
            "—".to_string()
        } else {
            replay.record_time.clone()
        };
        div()
            .h(px(52.0))
            .w_full()
            .flex_none()
            .px_5()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(rgb(BORDER))
            .hover(|style| style.bg(rgb(0xf8fafc)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(lesson),
            )
            .child(
                div()
                    .w(px(170.0))
                    .flex_none()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(time),
            )
            .child(button.on_click(cx.listener(move |this, _, window, cx| {
                this.download_replay(replay_for_download.clone(), window, cx);
            })))
            .into_any_element()
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
