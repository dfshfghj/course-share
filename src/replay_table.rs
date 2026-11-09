use crate::app::AppState;
use crate::blackboard::Replay;
use crate::share::Share;
use crate::ui::measured_text_width;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::{Scrollbar, ScrollbarMode};
use gpui_component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, App, Context, Edges, ElementId, Entity, IntoElement, Pixels, WeakEntity,
    Window,
};

pub(crate) fn cell_padding() -> Edges<Pixels> {
    Edges {
        top: px(4.0),
        bottom: px(4.0),
        left: px(8.0),
        right: px(8.0),
    }
}

fn text_width(window: &Window, text: &str) -> f32 {
    measured_text_width(window, text, 16.0) + 40.0
}

fn button_width(window: &Window, labels: &[&str]) -> f32 {
    labels
        .iter()
        .map(|label| measured_text_width(window, label, 14.0))
        .fold(0.0f32, f32::max)
        + 44.0
}

fn plain_width(window: &Window, text: &str) -> f32 {
    measured_text_width(window, text, 14.0) + 8.0
}

pub(crate) struct CourseReplayDelegate {
    rows: Vec<Replay>,
    title_width: f32,
    time_width: f32,
    room_width: f32,
    button_width: f32,
    action_width: f32,
    view: WeakEntity<AppState>,
}

impl CourseReplayDelegate {
    pub(crate) fn new(
        window: &Window,
        rows: Vec<Replay>,
        view: WeakEntity<AppState>,
    ) -> Self {
        let time_width = rows
            .iter()
            .map(|row| text_width(window, &time_text(row)))
            .fold(0.0f32, f32::max);
        let room_width = rows
            .iter()
            .map(|row| text_width(window, &room_text(&row.room)))
            .fold(0.0f32, f32::max);
        let button_width = button_width(window, &["预览", "继续下载"]);
        let action_width = button_width * 2.0 + 8.0 + 40.0;
        let viewport = f32::from(window.viewport_size().width);
        let title_width =
            (viewport - 300.0 - time_width - room_width - action_width - 64.0).max(160.0);
        Self {
            rows,
            title_width,
            time_width,
            room_width,
            button_width,
            action_width,
            view,
        }
    }
}

impl TableDelegate for CourseReplayDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        4
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        match col_ix {
            0 => Column::new("title", "标题").width(px(self.title_width)),
            1 => Column::new("time", "时间").width(px(self.time_width)),
            2 => Column::new("room", "教室").width(px(self.room_width)),
            _ => Column::new("actions", "操作").width(px(self.action_width)),
        }
        .paddings(cell_padding())
        .resizable(true)
        .movable(false)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.column(col_ix, cx).name.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(name)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(replay) = self.rows.get(row_ix).cloned() else {
            return div().into_any_element();
        };
        match col_ix {
            0 => div()
                .w_full()
                .truncate()
                .child(replay.sub_title.clone())
                .into_any_element(),
            1 => div()
                .w_full()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(time_text(&replay))
                .into_any_element(),
            2 => div()
                .w_full()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(room_text(&replay.room))
                .into_any_element(),
            _ => {
                let view = self.view.clone();
                let (label, tone) = match view.upgrade() {
                    Some(entity) => {
                        let app = entity.read(cx);
                        if app.replay_downloaded(&replay) {
                            ("已下载", "success")
                        } else if app.replay_in_progress(&replay) {
                            ("下载中", "warning")
                        } else if app.replay_has_cache(&replay) {
                            ("继续下载", "primary")
                        } else {
                            ("下载", "primary")
                        }
                    }
                    None => ("下载", "primary"),
                };
                let preview_view = view.clone();
                let preview_replay = replay.clone();
                let preview = Button::new(ElementId::Name(
                    format!("preview-{}", replay.identity()).into(),
                ))
                .label("预览")
                .w(px(self.button_width))
                .ghost()
                .on_click(cx.listener(move |_table, _, _, cx| {
                    let replay = preview_replay.clone();
                    let _ = preview_view.update(cx, |app, cx| app.preview_replay(replay, cx));
                }));
                let download_view = view.clone();
                let download_replay = replay.clone();
                let mut download = Button::new(ElementId::Name(
                    format!("download-{}", replay.identity()).into(),
                ))
                .label(label)
                .w(px(self.button_width));
                download = match tone {
                    "success" => download.success().disabled(true),
                    "warning" => download.warning().disabled(true),
                    _ => download.primary(),
                };
                let download = download.on_click(cx.listener(move |_table, _, window, cx| {
                    let replay = download_replay.clone();
                    let _ = download_view.update(cx, |app, cx| {
                        app.download_replay(replay, window, cx);
                    });
                }));
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(preview)
                    .child(download)
                    .into_any_element()
            }
        }
    }
}

pub(crate) struct PlazaReplayDelegate {
    rows: Vec<Share>,
    title_width: f32,
    time_width: f32,
    room_width: f32,
    button_width: f32,
    action_width: f32,
    view: WeakEntity<AppState>,
}

impl PlazaReplayDelegate {
    pub(crate) fn new(
        window: &Window,
        rows: Vec<Share>,
        view: WeakEntity<AppState>,
    ) -> Self {
        let time_width = rows
            .iter()
            .map(|row| text_width(window, &share_time_text(row)))
            .fold(0.0f32, f32::max);
        let room_width = rows
            .iter()
            .map(|row| text_width(window, &room_text(&row.room)))
            .fold(0.0f32, f32::max);
        let button_width = button_width(window, &["预览", "下载", "复制链接"]);
        let downloads_width = rows
            .iter()
            .map(|row| plain_width(window, &format!("下载 {}", row.downloads)))
            .fold(0.0f32, f32::max);
        let action_width =
            downloads_width + button_width * 3.0 + 8.0 * 3.0 + 40.0;
        let viewport = f32::from(window.viewport_size().width);
        let title_width =
            (viewport - 300.0 - time_width - room_width - action_width - 64.0).max(160.0);
        Self {
            rows,
            title_width,
            time_width,
            room_width,
            button_width,
            action_width,
            view,
        }
    }
}

impl TableDelegate for PlazaReplayDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        4
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        match col_ix {
            0 => Column::new("title", "标题").width(px(self.title_width)),
            1 => Column::new("time", "时间").width(px(self.time_width)),
            2 => Column::new("room", "教室").width(px(self.room_width)),
            _ => Column::new("actions", "操作").width(px(self.action_width)),
        }
        .paddings(cell_padding())
        .resizable(true)
        .movable(false)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let name = self.column(col_ix, cx).name.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(name)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(share) = self.rows.get(row_ix).cloned() else {
            return div().into_any_element();
        };
        match col_ix {
            0 => div()
                .w_full()
                .truncate()
                .child(share.replay_title.clone())
                .into_any_element(),
            1 => div()
                .w_full()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(share_time_text(&share))
                .into_any_element(),
            2 => div()
                .w_full()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(room_text(&share.room))
                .into_any_element(),
            _ => {
                let view = self.view.clone();
                let preview_view = view.clone();
                let preview_share = share.clone();
                let preview = Button::new(ElementId::Name(
                    format!("plaza-preview-{}", share.id).into(),
                ))
                .label("预览")
                .w(px(self.button_width))
                .ghost()
                .on_click(cx.listener(move |_table, _, _, cx| {
                    let share = preview_share.clone();
                    let _ = preview_view.update(cx, |app, cx| app.preview_share(share, cx));
                }));
                let download_view = view.clone();
                let download_share = share.clone();
                let download_disabled = share.m3u8_url.is_empty();
                let download = Button::new(ElementId::Name(
                    format!("plaza-download-{}", share.id).into(),
                ))
                .label("下载")
                .w(px(self.button_width))
                .primary()
                .disabled(download_disabled)
                .on_click(cx.listener(move |_table, _, window, cx| {
                    let share = download_share.clone();
                    let _ = download_view.update(cx, |app, cx| {
                        app.download_share(share, window, cx);
                    });
                }));
                let copy_view = view.clone();
                let copy_url = share.m3u8_url.clone();
                let copy = Button::new(ElementId::Name(
                    format!("plaza-copy-{}", share.id).into(),
                ))
                .label("复制链接")
                .w(px(self.button_width))
                .ghost()
                .disabled(copy_url.is_empty())
                .on_click(cx.listener(move |_table, _, _, cx| {
                    let url = copy_url.clone();
                    let _ = copy_view.update(cx, |app, cx| app.copy_text(url, cx));
                }));
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("下载 {}", share.downloads)),
                    )
                    .child(preview)
                    .child(download)
                    .child(copy)
                    .into_any_element()
            }
        }
    }
}

pub(crate) fn course_table_element(
    state: &Entity<TableState<CourseReplayDelegate>>,
    cx: &App,
) -> AnyElement {
    let horizontal = state.read(cx).horizontal_scroll_handle.clone();
    let table = DataTable::new(state)
        .bordered(false)
        .stripe(false)
        .header_row_height(px(26.0))
        .scrollbar_visible(true, false)
        .large();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(table),
        )
        .child(
            div()
                .h(Scrollbar::width())
                .flex_none()
                .child(
                    Scrollbar::horizontal(&horizontal)
                        .viewport_from_layout()
                        .mode(ScrollbarMode::Always),
                ),
        )
        .into_any_element()
}

pub(crate) fn plaza_table_element(
    state: &Entity<TableState<PlazaReplayDelegate>>,
    cx: &App,
) -> AnyElement {
    let horizontal = state.read(cx).horizontal_scroll_handle.clone();
    let table = DataTable::new(state)
        .bordered(false)
        .stripe(false)
        .header_row_height(px(26.0))
        .scrollbar_visible(true, false)
        .large();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(table),
        )
        .child(
            div()
                .h(Scrollbar::width())
                .flex_none()
                .child(
                    Scrollbar::horizontal(&horizontal)
                        .viewport_from_layout()
                        .mode(ScrollbarMode::Always),
                ),
        )
        .into_any_element()
}

fn time_text(replay: &Replay) -> String {
    if replay.record_time.trim().is_empty() {
        "—".to_string()
    } else {
        replay.record_time.clone()
    }
}

fn share_time_text(share: &Share) -> String {
    if !share.record_time.trim().is_empty() {
        share.record_time.clone()
    } else if !share.record_date.trim().is_empty() {
        share.record_date.clone()
    } else {
        "—".to_string()
    }
}

fn room_text(room: &str) -> String {
    if room.trim().is_empty() {
        "—".to_string()
    } else {
        room.to_string()
    }
}

pub(crate) fn course_table_key(
    prefix: &str,
    window: &Window,
    rows: &[Replay],
) -> String {
    let mut key = format!("{prefix}|{}", f32::from(window.viewport_size().width) as i32);
    for row in rows {
        key.push_str(&format!(
            "|{}:{}:{}:{}",
            row.identity(),
            row.record_time,
            row.room,
            row.sub_title
        ));
    }
    key
}

pub(crate) fn plaza_table_key(window: &Window, rows: &[Share]) -> String {
    let mut key = format!("plaza|{}", f32::from(window.viewport_size().width) as i32);
    for row in rows {
        key.push_str(&format!(
            "|{}:{}:{}:{}:{}",
            row.id, row.record_time, row.record_date, row.room, row.replay_title
        ));
    }
    key
}

impl AppState {
    pub(crate) fn ensure_course_table(
        &mut self,
        window: &mut Window,
        prefix: &str,
        rows: &[Replay],
        cx: &mut Context<Self>,
    ) -> Entity<TableState<CourseReplayDelegate>> {
        let key = course_table_key(prefix, window, rows);
        if self.course_table_key != key || self.course_table.is_none() {
            let rows = rows.to_vec();
            let view = cx.entity().downgrade();
            self.course_table = Some(cx.new(|cx| {
                TableState::new(CourseReplayDelegate::new(window, rows, view), window, cx)
            }));
            self.course_table_key = key;
        }
        self.course_table.clone().unwrap()
    }

    pub(crate) fn ensure_plaza_table(
        &mut self,
        window: &mut Window,
        rows: &[Share],
        cx: &mut Context<Self>,
    ) -> Entity<TableState<PlazaReplayDelegate>> {
        let key = plaza_table_key(window, rows);
        if self.plaza_table_key != key || self.plaza_table.is_none() {
            let rows = rows.to_vec();
            let view = cx.entity().downgrade();
            self.plaza_table = Some(cx.new(|cx| {
                TableState::new(PlazaReplayDelegate::new(window, rows, view), window, cx)
            }));
            self.plaza_table_key = key;
        }
        self.plaza_table.clone().unwrap()
    }
}
