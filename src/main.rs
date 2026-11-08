#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod auth;
mod blackboard;
mod config;
mod cookies;
mod download;
mod download_manager;
mod ffmpeg;
mod haoxue;
mod http;
mod media;
mod onlineroomse;
mod preview;
mod share;
mod ui;
mod ui_courses;
mod ui_plaza;
mod ui_scroll;
mod ui_settings;

use gpui_kit::prelude::*;
use gpui_kit::{point, px, size, Bounds, WindowBounds, WindowDecorations, WindowOptions};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(120.0), px(70.0)),
                    size(px(1400.0), px(900.0)),
                ))),
                app_id: Some("course-share".to_string()),
                window_decorations: Some(WindowDecorations::Client),
                ..gpui_component::TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| app::AppState::new(window, cx));
                view.update(cx, |this, cx| this.bootstrap(window, cx));
                view
            })
            .expect("failed to open window");
        });
}
