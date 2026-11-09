use crate::auth;
use crate::blackboard::{self, Course, Replay};
use crate::config::AppConfig;
use crate::download::{DownloadControl, DownloadEvent};
use crate::http::Http;
use crate::media::{self, MediaCandidate};
use crate::share::{CourseFacets, CourseGroup, Share, ShareClient};
use crate::replay_table::{CourseReplayDelegate, PlazaReplayDelegate};
use crate::smooth_scroll::SmoothTarget;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_component::IndexPath;
use gpui_kit::prelude::*;
use gpui_kit::{px, Context, Entity, Pixels, ScrollHandle, Task, UniformListScrollHandle, Window};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use url::Url;

const PLAZA_PAGE_SIZE: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlazaFilters {
    pub(crate) term: String,
    pub(crate) college: String,
    pub(crate) schedule_type: String,
}

pub(crate) type PlazaSelectState = Entity<SelectState<SearchableVec<String>>>;

#[derive(Clone, Copy)]
enum PlazaFilterKind {
    Term,
    College,
    Schedule,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Courses,
    Plaza,
    Settings,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NavState {
    tab: Tab,
    selected_course: Option<usize>,
    favorite_selected: Option<String>,
    plaza_selected_course: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoginMode {
    Password,
    Cookie,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    Courses,
    Favorites,
}

enum HaoxueFetch {
    Replays { http: Http, replays: Vec<Replay> },
    Verification(auth::PendingLogin),
}

#[derive(Clone)]
pub struct DownloadTask {
    pub id: String,
    pub title: String,
    pub state: DownloadTaskState,
    pub downloaded: u64,
    pub total: u64,
    pub detail: String,
    pub attempt: u64,
    pub control: Option<Arc<DownloadControl>>,
    pub replay: Replay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadTaskState {
    Queued,
    Downloading,
    Paused,
    Merging,
    Done,
    Skipped,
    Failed,
}

pub struct AppState {
    pub(crate) http: Option<Http>,
    pub(crate) config: AppConfig,
    pub(crate) tab: Tab,
    pub(crate) nav_history: Vec<NavState>,
    pub(crate) nav_index: usize,
    pub(crate) login_mode: LoginMode,
    pub(crate) username: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) cookie_input: Entity<InputState>,
    pub(crate) plaza_search: Entity<InputState>,
    pub(crate) backend_input: Entity<InputState>,
    pub(crate) download_dir_input: Entity<InputState>,
    pub(crate) ffmpeg_input: Entity<InputState>,
    pub(crate) verification_input: Entity<InputState>,
    pub(crate) pending_login: Option<auth::PendingLogin>,
    pub(crate) captcha_path: Option<PathBuf>,
    pub(crate) course_scroll: UniformListScrollHandle,
    pub(crate) replay_scroll: UniformListScrollHandle,
    pub(crate) favorite_scroll: UniformListScrollHandle,
    pub(crate) plaza_grid_scroll: gpui_kit::base::VirtualListScrollHandle,
    pub(crate) plaza_detail_scroll: UniformListScrollHandle,
    pub(crate) collapsed_terms: HashSet<String>,
    pub(crate) collapsed_sections: HashSet<String>,
    pub(crate) starred_courses: HashSet<String>,
    pub(crate) favorite_selected: Option<String>,
    pub(crate) plaza_selected_course: Option<String>,
    pub(crate) busy: bool,
    pub(crate) downloading: bool,
    pub(crate) active_downloads: HashSet<String>,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) courses: Vec<Course>,
    pub(crate) selected_course: Option<usize>,
    pub(crate) replays: Vec<Replay>,
    pub(crate) course_table: Option<Entity<gpui_component::table::TableState<CourseReplayDelegate>>>,
    pub(crate) course_table_key: String,
    pub(crate) plaza_table: Option<Entity<gpui_component::table::TableState<PlazaReplayDelegate>>>,
    pub(crate) plaza_table_key: String,
    pub(crate) replays_loading: bool,
    pub(crate) replays_loading_label: String,
    pub(crate) pending_haoxue_course: Option<Course>,
    pub(crate) plaza_courses: Vec<CourseGroup>,
    pub(crate) plaza_shares: Vec<Share>,
    pub(crate) plaza_detail_loading: bool,
    pub(crate) favorite_shares: Vec<Share>,
    pub(crate) plaza_loading_more: bool,
    pub(crate) plaza_has_more: bool,
    pub(crate) plaza_total: usize,
    pub(crate) plaza_scrolled: bool,
    pub(crate) plaza_filters: PlazaFilters,
    pub(crate) plaza_facets: CourseFacets,
    pub(crate) plaza_term_select: Option<PlazaSelectState>,
    pub(crate) plaza_college_select: Option<PlazaSelectState>,
    pub(crate) plaza_schedule_select: Option<PlazaSelectState>,
    pub(crate) download_tasks: Vec<DownloadTask>,
    pub(crate) download_scroll: ScrollHandle,
    pub(crate) download_attempt: u64,
    pub(crate) scheduler: crate::download_manager::DownloadScheduler,
    pub(crate) downloaded: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
    pub(crate) smooth_target: Option<(&'static str, SmoothTarget)>,
    pub(crate) smooth_pending: Pixels,
    pub(crate) smooth_animating: bool,
    pub(crate) smooth_scroll_task: Option<Task<()>>,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = AppConfig::load();
        let credentials = crate::config::Credentials::load();
        let starred_courses: HashSet<String> = config.starred_courses.iter().cloned().collect();
        let username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("学号")
                .default_value(credentials.username.clone())
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("密码")
                .masked(true)
                .default_value(credentials.password.clone())
        });
        let cookie_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("粘贴 Cookie：name=value; name2=value2")
        });
        let plaza_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索课程、回放、教师或 GUID")
        });
        let backend_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("后端地址")
                .default_value(config.backend_url.clone())
        });
        let download_dir_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("下载目录")
                .default_value(config.download_dir.clone())
        });
        let ffmpeg_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("留空自动（优先使用内置 ffmpeg）")
                .default_value(config.ffmpeg.clone())
        });
        let verification_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入验证码")
        });
        let (download_tx, download_rx) = std::sync::mpsc::channel::<DownloadEvent>();
        let scheduler = crate::download_manager::DownloadScheduler::new(download_tx);

        let mut app = Self {
            http: None,
            config,
            tab: Tab::Courses,
            nav_history: Vec::new(),
            nav_index: 0,
            login_mode: LoginMode::Password,
            username,
            password,
            cookie_input,
            plaza_search,
            backend_input,
            download_dir_input,
            ffmpeg_input,
            verification_input,
            pending_login: None,
            captcha_path: None,
            course_scroll: UniformListScrollHandle::new(),
            replay_scroll: UniformListScrollHandle::new(),
            favorite_scroll: UniformListScrollHandle::new(),
            plaza_grid_scroll: gpui_kit::base::VirtualListScrollHandle::new(),
            plaza_detail_scroll: UniformListScrollHandle::new(),
            collapsed_terms: HashSet::new(),
            collapsed_sections: HashSet::from(["favorites".to_string()]),
            starred_courses,
            favorite_selected: None,
            plaza_selected_course: None,
            busy: false,
            downloading: false,
            active_downloads: HashSet::new(),
            status: "请登录教学网".to_string(),
            error: None,
            courses: Vec::new(),
            selected_course: None,
            replays: Vec::new(),
            course_table: None,
            course_table_key: String::new(),
            plaza_table: None,
            plaza_table_key: String::new(),
            replays_loading: false,
            replays_loading_label: "正在加载回放…".to_string(),
            pending_haoxue_course: None,
            plaza_courses: Vec::new(),
            plaza_shares: Vec::new(),
            plaza_detail_loading: false,
            favorite_shares: Vec::new(),
            plaza_loading_more: false,
            plaza_has_more: false,
            plaza_total: 0,
            plaza_scrolled: false,
            plaza_filters: PlazaFilters::default(),
            plaza_facets: CourseFacets::default(),
            plaza_term_select: None,
            plaza_college_select: None,
            plaza_schedule_select: None,
            download_tasks: Vec::new(),
            download_scroll: ScrollHandle::new(),
            download_attempt: 0,
            scheduler,
            downloaded: 0,
            skipped: 0,
            failed: 0,
            smooth_target: None,
            smooth_pending: px(0.0),
            smooth_animating: false,
            smooth_scroll_task: None,
        };
        cx.subscribe_in(
            &app.plaza_search,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.refresh_plaza(window, cx);
                }
            },
        )
        .detach();
        app.rebuild_plaza_selects(window, cx);
        cx.spawn_in(window, async move |this, cx| loop {
            match download_rx.try_recv() {
                Ok(event) => {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.on_download_event(event);
                        cx.notify();
                    });
                }
                Err(TryRecvError::Empty) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |this, cx| this.poll_plaza_scroll(cx));
        })
        .detach();
        app
    }

    pub(crate) fn bootstrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_session(window, cx);
    }

    fn run_blocking<T, F, A>(&mut self, window: &mut Window, cx: &mut Context<Self>, work: F, apply: A)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        A: FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        cx.spawn_in(window, async move |this, cx| loop {
            match rx.try_recv() {
                Ok(value) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        apply(this, value, window, cx);
                        cx.notify();
                    });
                    break;
                }
                Err(TryRecvError::Empty) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(80))
                        .await;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        })
        .detach();
    }

    fn run_background<T, F, A>(&mut self, cx: &mut Context<Self>, work: F, apply: A)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        A: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        cx.spawn(async move |this, cx| loop {
            match rx.try_recv() {
                Ok(value) => {
                    let _ = this.update(cx, |this, cx| {
                        apply(this, value, cx);
                        cx.notify();
                    });
                    break;
                }
                Err(TryRecvError::Empty) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(80))
                        .await;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        })
        .detach();
    }

    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let http = match Http::new(AppConfig::cookie_path()) {
            Ok(http) => http,
            Err(error) => {
                self.error = Some(format!("初始化 Cookie 存储失败：{error:#}"));
                return;
            }
        };
        if http.cookie_count() == 0 {
            return;
        }
        self.status = "检查已保存的登录状态…".to_string();
        self.busy = true;
        let probe = http.clone();
        self.run_blocking(
            window,
            cx,
            move || auth::validate_blackboard(&probe),
            |this, valid, window, cx| {
                this.busy = false;
                if valid {
                    this.http = Some(http);
                    this.status = "已恢复登录状态".to_string();
                    this.load_courses(window, cx);
                } else {
                    let username = this.username.read(cx).value().trim().to_string();
                    let password = this.password.read(cx).value().to_string();
                    if !username.is_empty() && !password.is_empty() {
                        this.status = "登录状态已失效，正在自动重新登录…".to_string();
                        this.error = None;
                        this.start_login(window, cx);
                    } else {
                        this.status = "已保存的 Cookie 已失效，请重新登录".to_string();
                    }
                }
            },
        );
    }

    fn remember_credentials(&self, cx: &Context<Self>) {
        let username = self.username.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        if !username.is_empty() && !password.is_empty() {
            let _ = crate::config::Credentials { username, password }.save();
        }
    }

    pub(crate) fn start_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let username = self.username.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        if username.trim().is_empty() || password.is_empty() {
            self.error = Some("请输入学号和密码".to_string());
            cx.notify();
            return;
        }
        let cookie_path = AppConfig::cookie_path();
        self.busy = true;
        self.error = None;
        self.status = "正在登录 IAAA / 教学网…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || auth::begin_login(&username, &password, cookie_path),
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(auth::LoginOutcome::Success(http)) => {
                        this.http = Some(http);
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.remember_credentials(cx);
                        this.status = "登录成功".to_string();
                        this.load_courses(window, cx);
                    }
                    Ok(auth::LoginOutcome::Verification(pending)) => {
                        this.pending_login = Some(pending);
                        this.captcha_path = None;
                        this.error = None;
                        this.after_pending_login(window, cx);
                    }
                    Ok(auth::LoginOutcome::Failed(message)) => {
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.error = Some(message);
                        this.status = "登录失败".to_string();
                    }
                    Err(error) => {
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.error = Some(format!("{error:#}"));
                        this.status = "登录失败".to_string();
                    }
                }
            },
        );
    }

    fn after_pending_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_login.clone() else {
            return;
        };
        let kind = pending.challenge.kind.clone();
        let placeholder = kind.placeholder();
        self.verification_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        self.status = format!("需要{}：{}", kind.label(), pending.challenge.message);
        self.error = None;
        if kind == auth::VerificationKind::Captcha {
            self.fetch_captcha(window, cx);
        }
        cx.notify();
    }

    fn fetch_captcha(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_login.clone() else {
            return;
        };
        if let Some(old) = self.captcha_path.take() {
            let _ = std::fs::remove_file(old);
        }
        let path = AppConfig::data_dir().join(format!(
            "captcha-{}.jpg",
            chrono::Utc::now().timestamp_millis()
        ));
        self.run_blocking(
            window,
            cx,
            move || auth::fetch_captcha(&pending, &path).map(|_| path),
            |this, result, _window, cx| {
                match result {
                    Ok(path) => this.captcha_path = Some(path),
                    Err(error) => {
                        this.error = Some(format!("获取图形验证码失败：{error:#}"));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn submit_verification(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let code = self.verification_input.read(cx).value().trim().to_string();
        if code.is_empty() {
            self.error = Some("请输入验证码".to_string());
            cx.notify();
            return;
        }
        let Some(pending) = self.pending_login.take() else {
            return;
        };
        self.busy = true;
        self.error = None;
        self.status = "提交验证码…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || auth::resume_login(pending, code),
            |this, result, window, cx| {
                this.busy = false;
                this.verification_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                match result {
                    Ok(auth::LoginOutcome::Success(http)) => {
                        this.http = Some(http);
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.remember_credentials(cx);
                        if let Some(course) = this.pending_haoxue_course.take() {
                            this.status = "好学登录成功，正在查询回放…".to_string();
                            this.start_haoxue_fallback(course, window, cx);
                        } else {
                            this.status = "登录成功".to_string();
                            this.load_courses(window, cx);
                        }
                    }
                    Ok(auth::LoginOutcome::Verification(pending)) => {
                        this.pending_login = Some(pending);
                        this.after_pending_login(window, cx);
                    }
                    Ok(auth::LoginOutcome::Failed(message)) => {
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.error = Some(message);
                        this.status = "验证失败".to_string();
                    }
                    Err(error) => {
                        this.pending_login = None;
                        this.captcha_path = None;
                        this.error = Some(format!("{error:#}"));
                        this.status = "验证失败".to_string();
                    }
                }
            },
        );
    }

    pub(crate) fn cancel_verification(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_login = None;
        let was_haoxue = self.pending_haoxue_course.take().is_some();
        if let Some(path) = self.captcha_path.take() {
            let _ = std::fs::remove_file(path);
        }
        self.verification_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.error = None;
        self.status = if was_haoxue {
            "已取消好学登录".to_string()
        } else {
            "已取消二次验证".to_string()
        };
        cx.notify();
    }

    pub(crate) fn refresh_captcha(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fetch_captcha(window, cx);
    }

    pub(crate) fn send_sms_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(pending) = self.pending_login.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        self.status = "正在发送短信验证码…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || auth::send_sms_code(&pending),
            |this, result, _window, cx| {
                this.busy = false;
                match result {
                    Ok(mask) => this.status = format!("验证码已发送至 {mask}"),
                    Err(error) => this.error = Some(format!("发送短信验证码失败：{error:#}")),
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn start_cookie_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let cookies = self.cookie_input.read(cx).value().to_string();
        if cookies.trim().is_empty() {
            self.error = Some("请粘贴教学网 Cookie".to_string());
            cx.notify();
            return;
        }
        let cookie_path = AppConfig::cookie_path();
        self.busy = true;
        self.error = None;
        self.status = "正在验证 Cookie…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || auth::login_with_cookie(&cookies, cookie_path),
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(http) => {
                        this.http = Some(http);
                        this.status = "Cookie 登录成功".to_string();
                        this.load_courses(window, cx);
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.status = "Cookie 无效".to_string();
                    }
                }
            },
        );
    }

    pub(crate) fn logout(&mut self, cx: &mut Context<Self>) {
        if let Some(http) = &self.http {
            let _ = http.clear_cookies();
        }
        crate::config::Credentials::clear();
        self.http = None;
        self.courses.clear();
        self.replays.clear();
        self.plaza_courses.clear();
        self.plaza_shares.clear();
        self.plaza_detail_loading = false;
        self.favorite_shares.clear();
        self.plaza_loading_more = false;
        self.plaza_has_more = false;
        self.plaza_total = 0;
        self.selected_course = None;
        self.plaza_selected_course = None;
        self.plaza_scrolled = false;
        self.favorite_selected = None;
        self.collapsed_terms.clear();
        self.collapsed_sections = HashSet::from(["favorites".to_string()]);
        self.nav_history.clear();
        self.nav_index = 0;
        self.smooth_target = None;
        self.smooth_pending = px(0.0);
        self.pending_login = None;
        self.pending_haoxue_course = None;
        self.captcha_path = None;
        self.replays_loading = false;
        self.replays_loading_label = "正在加载回放…".to_string();
        self.status = "已退出登录".to_string();
        self.error = None;
        cx.notify();
    }

    fn load_courses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        self.status = "加载课程列表…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || blackboard::list_courses(&http),
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(courses) => {
                        this.status = format!("共 {} 门课程", courses.len());
                        this.courses = courses;
                        let first = this.courses.first().cloned();
                        if let Some(course) = first {
                            this.selected_course = Some(0);
                            this.load_replays_for(course, window, cx);
                        } else {
                            this.selected_course = None;
                        }
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.status = "课程加载失败".to_string();
                    }
                }
            },
        );
    }

    pub(crate) fn select_course(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_to(
            NavState {
                tab: Tab::Courses,
                selected_course: Some(index),
                favorite_selected: None,
                plaza_selected_course: None,
            },
            window,
            cx,
        );
    }

    fn show_course(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(course) = self.courses.get(index).cloned() else {
            return;
        };
        self.selected_course = Some(index);
        self.favorite_selected = None;
        self.replay_scroll = UniformListScrollHandle::new();
        self.load_replays_for(course, window, cx);
        cx.notify();
    }

    fn replay_cache_path(course: &Course) -> PathBuf {
        AppConfig::data_dir()
            .join("replay-cache")
            .join(format!("{}.json", media::sanitize_filename(&course.id)))
    }

    fn load_replay_cache(course: &Course) -> Vec<Replay> {
        let path = Self::replay_cache_path(course);
        fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<Replay>>(&text).ok())
            .map(|mut replays| {
                blackboard::sort_replays(&mut replays);
                replays
            })
            .unwrap_or_default()
    }

    fn save_replay_cache(course: &Course, replays: &[Replay]) {
        let path = Self::replay_cache_path(course);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(data) = serde_json::to_vec_pretty(replays) else {
            return;
        };
        let temporary = path.with_extension("json.tmp");
        if fs::write(&temporary, data).is_ok() {
            let _ = crate::media::replace_file(&temporary, &path);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
            }
        }
    }

    fn merge_replays(cached: Vec<Replay>, fresh: Vec<Replay>) -> Vec<Replay> {
        let mut by_identity: HashMap<String, Replay> = cached
            .into_iter()
            .map(|replay| (replay.identity(), replay))
            .collect();
        let mut merged = Vec::with_capacity(fresh.len());
        for mut replay in fresh {
            if let Some(old) = by_identity.remove(&replay.identity()) {
                if replay.media.is_empty() {
                    replay.media = old.media;
                }
                if replay.guid.is_empty() {
                    replay.guid = old.guid;
                }
                if replay.record_time.is_empty() {
                    replay.record_time = old.record_time;
                }
            }
            merged.push(replay);
        }
        blackboard::sort_replays(&mut merged);
        merged
    }

    fn load_replays_for(&mut self, course: Course, window: &mut Window, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            return;
        };
        self.replays = Self::load_replay_cache(&course);
        self.replays_loading = true;
        self.replays_loading_label = "正在加载回放…".to_string();
        self.error = None;
        self.status = format!("加载回放：{}", course.name);
        cx.notify();

        let course_for_load = course.clone();
        let course_for_apply = course.clone();
        self.run_blocking(
            window,
            cx,
            move || blackboard::list_replays(&http, &course_for_load),
            move |this, result, window, cx| {
                let current_id = this
                    .selected_course
                    .and_then(|index| this.courses.get(index))
                    .map(|current| current.id.clone());
                let is_current = current_id.as_deref() == Some(course_for_apply.id.as_str());
                if !is_current {
                    // Cache the result even if the user already switched away.
                    if let Ok(fresh) = result {
                        if !fresh.is_empty() {
                            let cached = Self::load_replay_cache(&course_for_apply);
                            Self::save_replay_cache(
                                &course_for_apply,
                                &Self::merge_replays(cached, fresh),
                            );
                        }
                    }
                    return;
                }
                match result {
                    Ok(fresh) if fresh.is_empty() => {
                        this.start_haoxue_fallback(course_for_apply.clone(), window, cx);
                    }
                    Ok(fresh) => {
                        this.replays_loading = false;
                        let cached = this.replays.clone();
                        this.replays = Self::merge_replays(cached, fresh);
                        Self::save_replay_cache(&course_for_apply, &this.replays);
                        let with_media = this
                            .replays
                            .iter()
                            .filter(|replay| !replay.media.is_empty())
                            .count();
                        this.status = format!(
                            "回放 {} 条，其中 {} 条已解析到媒体地址",
                            this.replays.len(),
                            with_media
                        );
                        cx.notify();
                    }
                    Err(error) => {
                        this.replays_loading = false;
                        this.status = "回放加载失败".to_string();
                        this.error = Some(if this.replays.is_empty() {
                            format!("{error:#}")
                        } else {
                            format!("刷新失败，显示本地缓存：{error:#}")
                        });
                        cx.notify();
                    }
                }
            },
        );
    }

    fn start_haoxue_fallback(
        &mut self,
        course: Course,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(http) = self.http.clone() else {
            return;
        };
        let username = self.username.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        self.replays_loading = true;
        self.replays_loading_label = "正在查询好学…".to_string();
        self.error = None;
        self.status = "教学网没有解析到回放，正在查询好学…".to_string();
        cx.notify();

        let course_for_load = course.clone();
        let course_for_apply = course.clone();
        self.run_blocking(
            window,
            cx,
            move || -> anyhow::Result<HaoxueFetch> {
                if crate::haoxue::session_valid(&http) {
                    crate::haoxue::normalize_cookies(&http);
                    let replays = crate::haoxue::list_replays(&http, &course_for_load)?;
                    return Ok(HaoxueFetch::Replays { http, replays });
                }
                match auth::begin_haoxue_login(&username, &password, AppConfig::cookie_path())? {
                    auth::LoginOutcome::Success(haoxue_http) => {
                        let replays =
                            crate::haoxue::list_replays(&haoxue_http, &course_for_load)?;
                        Ok(HaoxueFetch::Replays {
                            http: haoxue_http,
                            replays,
                        })
                    }
                    auth::LoginOutcome::Verification(pending) => {
                        Ok(HaoxueFetch::Verification(pending))
                    }
                    auth::LoginOutcome::Failed(message) => Err(anyhow::anyhow!(message)),
                }
            },
            move |this, result, _window, cx| {
                let current_id = this
                    .selected_course
                    .and_then(|index| this.courses.get(index))
                    .map(|current| current.id.clone());
                let is_current = current_id.as_deref() == Some(course_for_apply.id.as_str());
                match result {
                    Ok(HaoxueFetch::Replays { http, replays }) => {
                        this.http = Some(http);
                        if !is_current {
                            let cached = Self::load_replay_cache(&course_for_apply);
                            Self::save_replay_cache(
                                &course_for_apply,
                                &Self::merge_replays(cached, replays),
                            );
                            return;
                        }
                        this.replays_loading = false;
                        let cached = this.replays.clone();
                        this.replays = Self::merge_replays(cached, replays);
                        Self::save_replay_cache(&course_for_apply, &this.replays);
                        this.error = None;
                        this.status = if this.replays.is_empty() {
                            "教学网和好学都没有解析到回放".to_string()
                        } else {
                            format!("好学回放 {} 条", this.replays.len())
                        };
                    }
                    Ok(HaoxueFetch::Verification(pending)) => {
                        if !is_current {
                            return;
                        }
                        this.replays_loading = false;
                        this.pending_login = Some(pending);
                        this.pending_haoxue_course = Some(course_for_apply);
                        this.status = "好学登录需要二次验证".to_string();
                    }
                    Err(error) => {
                        if !is_current {
                            return;
                        }
                        this.replays_loading = false;
                        this.status = "好学查询失败".to_string();
                        this.error = Some(if this.replays.is_empty() {
                            format!("{error:#}")
                        } else {
                            format!("好学查询失败，保留本地缓存：{error:#}")
                        });
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn refresh_replays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected_course {
            self.select_course(index, window, cx);
        }
    }

    fn replay_output_dir(&self, replay: &Replay) -> PathBuf {
        self.config
            .download_path()
            .join(media::sanitize_filename(&replay.course_name))
            .join("_录播回放")
    }

    pub(crate) fn replay_downloaded(&self, replay: &Replay) -> bool {
        media::find_downloaded(
            &self.replay_output_dir(replay),
            &replay.sub_title,
            &replay.lecturer,
        )
        .is_some()
    }

    pub(crate) fn replay_in_progress(&self, replay: &Replay) -> bool {
        self.active_downloads.contains(&replay.identity())
    }

    pub(crate) fn replay_has_cache(&self, replay: &Replay) -> bool {
        media::find_in_progress(
            &self.replay_output_dir(replay),
            &replay.sub_title,
            &replay.lecturer,
        )
    }

    pub(crate) fn preview_replay(&mut self, replay: Replay, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            self.error = Some("请先登录".to_string());
            cx.notify();
            return;
        };
        let Some(candidate) = replay.media.first() else {
            self.error = Some("该回放没有可用媒体地址".to_string());
            cx.notify();
            return;
        };
        match crate::preview::open(http, replay.display_title(), candidate.url.clone()) {
            Ok(()) => {
                self.error = None;
                self.status = "已在系统浏览器打开预览".to_string();
            }
            Err(error) => {
                self.error = Some(format!("打开预览失败：{error:#}"));
            }
        }
        cx.notify();
    }

    pub(crate) fn preview_share(&mut self, share: Share, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            self.error = Some("请先登录".to_string());
            cx.notify();
            return;
        };
        if share.m3u8_url.is_empty() {
            self.error = Some("这条分享没有媒体地址".to_string());
            cx.notify();
            return;
        }
        let title = format!("{} · {}", share.course_name, share.replay_title);
        match crate::preview::open(http, title, share.m3u8_url.clone()) {
            Ok(()) => {
                self.error = None;
                self.status = "已在系统浏览器打开预览".to_string();
            }
            Err(error) => {
                self.error = Some(format!("打开预览失败：{error:#}"));
            }
        }
        cx.notify();
    }

    pub(crate) fn download_replay(
        &mut self,
        replay: Replay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.replay_downloaded(&replay) {
            self.status = "该回放已下载，无需重复下载".to_string();
            cx.notify();
            return;
        }
        if self.active_downloads.contains(&replay.identity()) {
            self.status = "该回放正在下载中".to_string();
            cx.notify();
            return;
        }
        self.download_many(vec![replay], window, cx);
    }

    pub(crate) fn download_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending: Vec<Replay> = self
            .replays
            .iter()
            .filter(|replay| {
                !self.replay_downloaded(replay) && !self.replay_in_progress(replay)
            })
            .cloned()
            .collect();
        if pending.is_empty() {
            self.status = "当前课程的回放已全部下载或正在下载".to_string();
            cx.notify();
            return;
        }
        self.download_many(pending, window, cx);
    }

    fn download_many(
        &mut self,
        replays: Vec<Replay>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.downloading {
            let keep_tasks = self.download_tasks.iter().any(|task| {
                matches!(
                    task.state,
                    DownloadTaskState::Paused | DownloadTaskState::Failed
                )
            });
            if !keep_tasks {
                self.download_tasks.clear();
                self.download_scroll = ScrollHandle::new();
                self.downloaded = 0;
                self.skipped = 0;
                self.failed = 0;
            }
        }
        self.enqueue_downloads(replays, cx);
    }

    fn enqueue_downloads(&mut self, replays: Vec<Replay>, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            self.error = Some("请先登录".to_string());
            cx.notify();
            return;
        };
        let replays: Vec<Replay> = replays
            .into_iter()
            .filter(|replay| {
                !self.replay_downloaded(replay)
                    && !self.active_downloads.contains(&replay.identity())
            })
            .collect();
        if replays.is_empty() {
            self.status = "没有需要下载的回放（已全部下载或正在下载）".to_string();
            cx.notify();
            return;
        }

        let output = self.config.download_path();
        let ffmpeg = crate::ffmpeg::resolve(&self.config.ffmpeg);
        self.downloading = true;
        self.error = None;
        self.status = format!("已加入下载队列：{} 条", replays.len());
        for replay in replays {
            self.download_attempt += 1;
            let attempt = self.download_attempt;
            let control = DownloadControl::new();
            let id = replay.identity();
            let title = replay.display_title();
            self.active_downloads.insert(id.clone());
            let task = DownloadTask {
                id: id.clone(),
                title,
                state: DownloadTaskState::Queued,
                downloaded: 0,
                total: 0,
                detail: "排队中".to_string(),
                attempt,
                control: Some(control.clone()),
                replay: replay.clone(),
            };
            if let Some(index) = self
                .download_tasks
                .iter()
                .position(|task| task.id == id)
            {
                self.download_tasks[index] = task;
            } else {
                self.download_tasks.push(task);
            }
            self.scheduler.enqueue(
                http.clone(),
                replay,
                output.clone(),
                ffmpeg.clone(),
                attempt,
                control,
            );
        }
        cx.notify();
    }

    fn task_for_attempt(&mut self, id: &str, attempt: u64) -> Option<&mut DownloadTask> {
        self.download_tasks
            .iter_mut()
            .find(|task| task.id == id && task.attempt == attempt)
    }

    fn finish_downloads_if_idle(&mut self) {
        if !self.active_downloads.is_empty() {
            return;
        }
        self.downloading = false;
        if self
            .download_tasks
            .iter()
            .any(|task| task.state == DownloadTaskState::Paused)
        {
            self.status = format!(
                "下载结束：新增 {}，跳过 {}，失败 {}（部分任务已暂停）",
                self.downloaded, self.skipped, self.failed
            );
        } else {
            self.status = format!(
                "下载完成：新增 {}，跳过 {}，失败 {}",
                self.downloaded, self.skipped, self.failed
            );
        }
    }

    pub(crate) fn pause_download(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(task) = self
            .download_tasks
            .iter_mut()
            .find(|task| task.id == id)
        else {
            return;
        };
        if task.state != DownloadTaskState::Downloading {
            return;
        }
        if let Some(control) = &task.control {
            control.pause();
        }
        task.state = DownloadTaskState::Paused;
        task.detail = "已暂停".to_string();
        let title = task.title.clone();
        self.active_downloads.remove(id);
        self.downloading = !self.active_downloads.is_empty();
        self.status = format!("已暂停：{title}");
        cx.notify();
    }

    pub(crate) fn resume_download(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(replay) = self
            .download_tasks
            .iter()
            .find(|task| task.id == id)
            .filter(|task| {
                matches!(
                    task.state,
                    DownloadTaskState::Paused | DownloadTaskState::Failed
                )
            })
            .map(|task| task.replay.clone())
        else {
            return;
        };
        self.enqueue_downloads(vec![replay], cx);
    }

    pub(crate) fn stop_download(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .download_tasks
            .iter()
            .position(|task| task.id == id)
        else {
            return;
        };
        let task = &self.download_tasks[index];
        if matches!(
            task.state,
            DownloadTaskState::Done | DownloadTaskState::Skipped | DownloadTaskState::Failed
        ) {
            return;
        }
        if let Some(control) = &task.control {
            control.stop();
        }
        let title = task.title.clone();
        self.download_tasks.remove(index);
        self.active_downloads.remove(id);
        self.downloading = !self.active_downloads.is_empty();
        self.status = format!("已停止并移出队列：{title}");
        cx.notify();
    }

    pub(crate) fn download_footer_visible(&self) -> bool {
        self.downloading
            || self.error.is_some()
            || self.download_tasks.iter().any(|task| {
                matches!(
                    task.state,
                    DownloadTaskState::Paused | DownloadTaskState::Failed
                )
            })
    }

    fn on_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Progress {
                attempt,
                id,
                title,
                downloaded,
                total,
                detail,
                merging,
            } => {
                let paused = match self.task_for_attempt(&id, attempt) {
                    Some(task) => match task.state {
                        DownloadTaskState::Paused => {
                            task.downloaded = downloaded;
                            task.total = total;
                            task.detail = detail.clone();
                            true
                        }
                        DownloadTaskState::Done
                        | DownloadTaskState::Skipped
                        | DownloadTaskState::Failed => return,
                        _ => {
                            task.state = if merging {
                                DownloadTaskState::Merging
                            } else {
                                DownloadTaskState::Downloading
                            };
                            task.downloaded = downloaded;
                            task.total = total;
                            task.detail = detail.clone();
                            false
                        }
                    },
                    None => return,
                };
                if !paused {
                    self.status = format!("{title}：{detail}");
                }
            }
            DownloadEvent::Done {
                attempt,
                id,
                title,
                path,
                skipped,
            } => {
                if self.task_for_attempt(&id, attempt).is_none() {
                    return;
                }
                if let Some(task) = self.task_for_attempt(&id, attempt) {
                    task.state = if skipped {
                        DownloadTaskState::Skipped
                    } else {
                        DownloadTaskState::Done
                    };
                    task.total = task.total.max(task.downloaded);
                    task.downloaded = task.total;
                    task.detail = path.display().to_string();
                }
                self.active_downloads.remove(&id);
                if skipped {
                    self.skipped += 1;
                    self.status = format!("已存在，跳过：{title}");
                } else {
                    self.downloaded += 1;
                    self.status = format!("已保存：{}", path.display());
                }
                self.finish_downloads_if_idle();
            }
            DownloadEvent::Stopped { attempt, id, title } => {
                if self.task_for_attempt(&id, attempt).is_none() {
                    return;
                }
                self.download_tasks.retain(|task| task.id != id);
                self.active_downloads.remove(&id);
                self.downloading = !self.active_downloads.is_empty();
                self.status = format!("已停止并移出队列：{title}");
            }
            DownloadEvent::Paused { attempt, id, title } => {
                if self.task_for_attempt(&id, attempt).is_none() {
                    return;
                }
                if let Some(task) = self.task_for_attempt(&id, attempt) {
                    task.state = DownloadTaskState::Paused;
                    task.detail = "已暂停".to_string();
                }
                self.active_downloads.remove(&id);
                self.downloading = !self.active_downloads.is_empty();
                self.status = format!("已暂停：{title}");
            }
            DownloadEvent::Failed {
                attempt,
                id,
                title,
                error,
            } => {
                if self.task_for_attempt(&id, attempt).is_none() {
                    return;
                }
                if let Some(task) = self.task_for_attempt(&id, attempt) {
                    task.state = DownloadTaskState::Failed;
                    task.detail = error.clone();
                }
                self.active_downloads.remove(&id);
                self.failed += 1;
                self.error = Some(format!("{title}：{error}"));
                self.status = format!("失败：{title}");
                self.finish_downloads_if_idle();
            }
        }
    }

    pub(crate) fn share_course(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(http) = self.http.clone() else {
            self.error = Some("请先登录".to_string());
            cx.notify();
            return;
        };
        let Some(course) = self.selected_course().cloned() else {
            self.error = Some("请先选择课程".to_string());
            cx.notify();
            return;
        };
        if self.replays.is_empty() {
            self.status = "当前课程没有可发布的回放".to_string();
            cx.notify();
            return;
        }
        let backend = self.config.backend_url.clone();
        let uploader = self.username.read(cx).value().trim().to_string();
        let uploader = if uploader.is_empty() {
            "anonymous".to_string()
        } else {
            uploader
        };
        let replays = self.replays.clone();
        self.busy = true;
        self.error = None;
        self.status = "正在发布本课程回放到广场…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || -> anyhow::Result<(usize, Vec<String>)> {
                let client = ShareClient::new(&backend)?;
                let mut published = 0usize;
                let mut errors = Vec::new();
                for mut replay in replays {
                    if replay.media.is_empty() {
                        if let Err(error) = blackboard::resolve_media(&http, &mut replay) {
                            errors.push(format!("{}: {error:#}", replay.sub_title));
                            continue;
                        }
                    }
                    let Some(share) = Share::from_replay(&course, &replay, &uploader) else {
                        errors.push(format!("{}: 没有可分享的媒体地址", replay.sub_title));
                        continue;
                    };
                    match client.publish(&share) {
                        Ok(_) => published += 1,
                        Err(error) => errors.push(format!("{}: {error:#}", replay.sub_title)),
                    }
                }
                Ok((published, errors))
            },
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok((published, errors)) => {
                        if published > 0 {
                            this.status = if errors.is_empty() {
                                format!("已发布 {published} 条回放到广场")
                            } else {
                                format!("已发布 {published} 条回放，{} 条失败", errors.len())
                            };
                            this.error = None;
                            this.refresh_plaza(window, cx);
                        } else {
                            this.error = Some(if errors.is_empty() {
                                "没有发布任何回放".to_string()
                            } else {
                                errors.join("\n")
                            });
                            this.status = "发布失败".to_string();
                        }
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.status = "发布失败".to_string();
                    }
                }
            },
        );
    }

    fn make_plaza_select(
        &mut self,
        options: Vec<String>,
        current_label: String,
        all_label: &'static str,
        kind: PlazaFilterKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PlazaSelectState {
        let selected_index = options
            .iter()
            .position(|option| option == &current_label)
            .map(IndexPath::new);
        let state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(options), selected_index, window, cx).searchable(true)
        });
        cx.subscribe_in(
            &state,
            window,
            move |this, _state, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                let SelectEvent::Confirm(value) = event;
                let selected = value.clone().unwrap_or_else(|| all_label.to_string());
                let mapped = if selected == all_label {
                    String::new()
                } else if matches!(kind, PlazaFilterKind::Schedule) {
                    this.plaza_facets
                        .schedule_types
                        .iter()
                        .find(|option| option.label == selected)
                        .map(|option| option.value.clone())
                        .unwrap_or_default()
                } else {
                    selected
                };
                let current = match kind {
                    PlazaFilterKind::Term => &mut this.plaza_filters.term,
                    PlazaFilterKind::College => &mut this.plaza_filters.college,
                    PlazaFilterKind::Schedule => &mut this.plaza_filters.schedule_type,
                };
                if *current != mapped {
                    *current = mapped;
                    cx.defer_in(window, |this, window, cx| this.refresh_plaza(window, cx));
                }
            },
        )
        .detach();
        state
    }

    pub(crate) fn rebuild_plaza_selects(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut term_options = vec!["全部学期".to_string()];
        term_options.extend(self.plaza_facets.terms.iter().cloned());
        let mut college_options = vec!["全部学院".to_string()];
        college_options.extend(self.plaza_facets.colleges.iter().cloned());
        let mut schedule_options = vec!["全部课表".to_string()];
        schedule_options.extend(
            self.plaza_facets
                .schedule_types
                .iter()
                .map(|option| option.label.clone()),
        );

        if !self.plaza_filters.term.is_empty()
            && !term_options
                .iter()
                .any(|option| option == &self.plaza_filters.term)
        {
            self.plaza_filters.term.clear();
        }
        if !self.plaza_filters.college.is_empty()
            && !college_options
                .iter()
                .any(|option| option == &self.plaza_filters.college)
        {
            self.plaza_filters.college.clear();
        }
        if !self.plaza_filters.schedule_type.is_empty()
            && !self
                .plaza_facets
                .schedule_types
                .iter()
                .any(|option| option.value == self.plaza_filters.schedule_type)
        {
            self.plaza_filters.schedule_type.clear();
        }

        let term_current = if self.plaza_filters.term.is_empty() {
            "全部学期".to_string()
        } else {
            self.plaza_filters.term.clone()
        };
        let college_current = if self.plaza_filters.college.is_empty() {
            "全部学院".to_string()
        } else {
            self.plaza_filters.college.clone()
        };
        let schedule_current = if self.plaza_filters.schedule_type.is_empty() {
            "全部课表".to_string()
        } else {
            self.plaza_facets
                .schedule_types
                .iter()
                .find(|option| option.value == self.plaza_filters.schedule_type)
                .map(|option| option.label.clone())
                .unwrap_or_else(|| "全部课表".to_string())
        };

        self.plaza_term_select = Some(self.make_plaza_select(
            term_options,
            term_current,
            "全部学期",
            PlazaFilterKind::Term,
            window,
            cx,
        ));
        self.plaza_college_select = Some(self.make_plaza_select(
            college_options,
            college_current,
            "全部学院",
            PlazaFilterKind::College,
            window,
            cx,
        ));
        self.plaza_schedule_select = Some(self.make_plaza_select(
            schedule_options,
            schedule_current,
            "全部课表",
            PlazaFilterKind::Schedule,
            window,
            cx,
        ));
    }

    pub(crate) fn refresh_plaza(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plaza_selected_course = None;
        self.plaza_grid_scroll = gpui_kit::base::VirtualListScrollHandle::new();
        self.plaza_detail_scroll = UniformListScrollHandle::new();
        self.plaza_scrolled = false;
        self.plaza_loading_more = false;
        let query = self.plaza_search.read(cx).value().trim().to_string();
        let filters = self.plaza_filters.clone();
        let backend = self.config.backend_url.clone();
        self.busy = true;
        self.error = None;
        self.status = "加载广场…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || -> anyhow::Result<(CourseFacets, Vec<CourseGroup>, usize)> {
                let client = ShareClient::new(&backend)?;
                let facets = client.course_facets().unwrap_or_default();
                let (courses, total) = client.list_courses_page(
                    &query,
                    &filters.term,
                    &filters.college,
                    &filters.schedule_type,
                    PLAZA_PAGE_SIZE,
                    0,
                )?;
                Ok((facets, courses, total))
            },
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok((facets, courses, total)) => {
                        this.plaza_facets = facets;
                        this.rebuild_plaza_selects(window, cx);
                        this.plaza_courses = courses;
                        this.plaza_total = total;
                        this.plaza_has_more = this.plaza_courses.len() < total;
                        this.status = format!("广场 {} 门课程", total);
                    }
                    Err(error) => {
                        this.plaza_courses.clear();
                        this.plaza_has_more = false;
                        this.error = Some(format!("{error:#}"));
                        this.status = "广场加载失败".to_string();
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn load_more_plaza(&mut self, cx: &mut Context<Self>) {
        if self.plaza_loading_more || !self.plaza_has_more || self.busy {
            return;
        }
        let query = self.plaza_search.read(cx).value().trim().to_string();
        let filters = self.plaza_filters.clone();
        let backend = self.config.backend_url.clone();
        let offset = self.plaza_courses.len();
        let query_for_work = query.clone();
        let filters_for_work = filters.clone();
        self.plaza_loading_more = true;
        cx.notify();
        self.run_background(
            cx,
            move || {
                ShareClient::new(&backend)?.list_courses_page(
                    &query_for_work,
                    &filters_for_work.term,
                    &filters_for_work.college,
                    &filters_for_work.schedule_type,
                    PLAZA_PAGE_SIZE,
                    offset,
                )
            },
            move |this, result, cx| {
                this.plaza_loading_more = false;
                let current_query = this.plaza_search.read(cx).value().trim().to_string();
                if current_query != query || this.plaza_filters != filters {
                    cx.notify();
                    return;
                }
                match result {
                    Ok((page, total)) => {
                        let before = this.plaza_courses.len();
                        for course in page {
                            if !this
                                .plaza_courses
                                .iter()
                                .any(|existing| existing.key == course.key)
                            {
                                this.plaza_courses.push(course);
                            }
                        }
                        this.plaza_total = total;
                        this.plaza_has_more =
                            this.plaza_courses.len() < total && this.plaza_courses.len() > before;
                        this.status = format!("广场 {} 门课程", total);
                    }
                    Err(error) => {
                        this.plaza_has_more = false;
                        this.error = Some(format!("加载更多失败：{error:#}"));
                    }
                }
                cx.notify();
            },
        );
    }

    fn poll_plaza_scroll(&mut self, cx: &mut Context<Self>) {
        let target = SmoothTarget::virtual_list(&self.plaza_grid_scroll);
        let (current, min) = target.offset_bounds();
        let scrolled = current < px(-4.0);
        if scrolled != self.plaza_scrolled {
            self.plaza_scrolled = scrolled;
            cx.notify();
        }
        if self.tab == Tab::Plaza
            && self.plaza_selected_course.is_none()
            && self.plaza_has_more
            && !self.plaza_loading_more
            && !self.busy
            && min < px(0.0)
            && current - min < px(240.0)
        {
            self.load_more_plaza(cx);
        }
    }

    pub(crate) fn scroll_plaza_top(&mut self, cx: &mut Context<Self>) {
        let target = SmoothTarget::virtual_list(&self.plaza_grid_scroll);
        self.scroll_target_to_top("plaza-grid", target, cx);
    }

    pub(crate) fn toggle_sidebar_section(
        &mut self,
        section: SidebarSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = match section {
            SidebarSection::Courses => "courses",
            SidebarSection::Favorites => "favorites",
        };
        let was_collapsed = self.collapsed_sections.remove(key);
        if !was_collapsed {
            self.collapsed_sections.insert(key.to_string());
        } else if section == SidebarSection::Favorites
            && self.favorite_shares.is_empty()
            && !self.busy
        {
            self.refresh_shares(window, cx);
        }
        cx.notify();
    }

    fn current_nav(&self) -> NavState {
        NavState {
            tab: self.tab,
            selected_course: self.selected_course,
            favorite_selected: self.favorite_selected.clone(),
            plaza_selected_course: self.plaza_selected_course.clone(),
        }
    }

    fn navigate_to(&mut self, next: NavState, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_nav();
        if next == current {
            return;
        }
        if self.nav_history.is_empty() {
            self.nav_history.push(current);
            self.nav_index = 0;
        }
        self.nav_history.truncate(self.nav_index + 1);
        self.nav_history.push(next.clone());
        self.nav_index = self.nav_history.len() - 1;
        self.apply_nav(next, window, cx);
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.nav_index > 0
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        !self.nav_history.is_empty() && self.nav_index + 1 < self.nav_history.len()
    }

    pub(crate) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_go_back() {
            return;
        }
        self.nav_index -= 1;
        let state = self.nav_history[self.nav_index].clone();
        self.apply_nav(state, window, cx);
    }

    pub(crate) fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_go_forward() {
            return;
        }
        self.nav_index += 1;
        let state = self.nav_history[self.nav_index].clone();
        self.apply_nav(state, window, cx);
    }

    fn apply_nav(&mut self, state: NavState, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = state.tab;
        match state.tab {
            Tab::Courses => {
                if let Some(key) = state.favorite_selected {
                    if self.favorite_selected.as_deref() != Some(key.as_str()) {
                        self.show_favorite(key, window, cx);
                    }
                } else if let Some(index) = state.selected_course {
                    if self.selected_course != Some(index) || self.favorite_selected.is_some() {
                        self.show_course(index, window, cx);
                    }
                } else {
                    self.favorite_selected = None;
                }
            }
            Tab::Plaza => {
                if let Some(key) = state.plaza_selected_course {
                    if self.plaza_selected_course.as_deref() != Some(key.as_str()) {
                        self.load_plaza_detail(key, window, cx);
                    }
                } else {
                    self.plaza_selected_course = None;
                    if self.plaza_courses.is_empty() {
                        self.refresh_plaza(window, cx);
                    }
                }
            }
            Tab::Settings => {}
        }
        cx.notify();
    }

    pub(crate) fn open_plaza(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to(
            NavState {
                tab: Tab::Plaza,
                selected_course: self.selected_course,
                favorite_selected: self.favorite_selected.clone(),
                plaza_selected_course: None,
            },
            window,
            cx,
        );
        self.refresh_plaza(window, cx);
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to(
            NavState {
                tab: Tab::Settings,
                selected_course: self.selected_course,
                favorite_selected: self.favorite_selected.clone(),
                plaza_selected_course: self.plaza_selected_course.clone(),
            },
            window,
            cx,
        );
    }

    pub(crate) fn set_theme(&mut self, mode: crate::theme::ThemeMode, cx: &mut Context<Self>) {
        self.config.theme = mode.pref().to_string();
        let _ = self.config.save();
        crate::theme::apply(mode.pref(), cx);
        cx.notify();
    }

    pub(crate) fn is_course_starred(&self, key: &str, name: &str) -> bool {
        self.starred_courses.contains(key)
            || self.starred_courses.contains(name.trim())
    }

    pub(crate) fn toggle_star_course(
        &mut self,
        key: String,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let was_starred = self.is_course_starred(&key, &name);
        self.starred_courses.remove(&key);
        self.starred_courses.remove(name.trim());
        if !was_starred {
            self.starred_courses.insert(key.clone());
        }
        if self.favorite_selected.as_deref() == Some(key.as_str()) && was_starred {
            self.favorite_selected = None;
        }
        self.config.starred_courses = self.starred_courses.iter().cloned().collect();
        let _ = self.config.save();
        cx.notify();
    }

    pub(crate) fn open_favorite_course(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_to(
            NavState {
                tab: Tab::Courses,
                selected_course: self.selected_course,
                favorite_selected: Some(key),
                plaza_selected_course: None,
            },
            window,
            cx,
        );
    }

    fn show_favorite(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let needs_shares = !self
            .favorite_shares
            .iter()
            .any(|share| share.course_key() == key);
        self.favorite_selected = Some(key);
        self.favorite_scroll = UniformListScrollHandle::new();
        if needs_shares && !self.busy {
            self.refresh_shares(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn refresh_shares(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let backend = self.config.backend_url.clone();
        self.busy = true;
        self.error = None;
        self.status = "加载分享…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || ShareClient::new(&backend)?.list(""),
            |this, result, _window, cx| {
                this.busy = false;
                match result {
                    Ok(shares) => {
                        this.favorite_shares = shares;
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.status = "分享加载失败".to_string();
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn favorite_replays_for(&self, key: &str) -> Vec<Replay> {
        let mut replays: Vec<Replay> = self
            .favorite_shares
            .iter()
            .filter(|share| share.course_key() == key)
            .map(|share| {
                let kind = media::classify_media_url(&share.m3u8_url).to_string();
                let quality = media::score_vod_url(&share.m3u8_url);
                let candidate = MediaCandidate {
                    url: share.m3u8_url.clone(),
                    kind,
                    label: "share".to_string(),
                    quality,
                    guid: share.guid.clone(),
                };
                Replay {
                    course_id: share.course_id.clone(),
                    course_name: share.course_name.clone(),
                    replay_id: share.guid.clone(),
                    sub_title: share.replay_title.clone(),
                    lecturer: share.teacher.clone(),
                    record_time: if share.record_time.is_empty() {
                        share.record_date.clone()
                    } else {
                        share.record_time.clone()
                    },
                    page_url: String::new(),
                    media: vec![candidate],
                    guid: share.guid.clone(),
                    ..Default::default()
                }
            })
            .collect();
        blackboard::sort_replays(&mut replays);
        replays
    }

    pub(crate) fn download_favorite_all(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replays = self.favorite_replays_for(key);
        self.download_many(replays, window, cx);
    }

    pub(crate) fn toggle_term(&mut self, term: String, cx: &mut Context<Self>) {
        if !self.collapsed_terms.remove(&term) {
            self.collapsed_terms.insert(term);
        }
        cx.notify();
    }

    pub(crate) fn select_plaza_course(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_to(
            NavState {
                tab: Tab::Plaza,
                selected_course: self.selected_course,
                favorite_selected: self.favorite_selected.clone(),
                plaza_selected_course: Some(key),
            },
            window,
            cx,
        );
    }

    fn load_plaza_detail(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = Tab::Plaza;
        self.plaza_selected_course = Some(key.clone());
        self.plaza_detail_scroll = UniformListScrollHandle::new();
        self.plaza_shares.clear();
        self.plaza_detail_loading = true;
        self.error = None;
        let backend = self.config.backend_url.clone();
        let group = self
            .plaza_courses
            .iter()
            .find(|course| course.key == key)
            .cloned();
        cx.notify();
        let Some(group) = group else {
            self.plaza_detail_loading = false;
            cx.notify();
            return;
        };
        self.run_blocking(
            window,
            cx,
            move || {
                ShareClient::new(&backend)?
                    .list_course_shares(&group.course_id, &group.course_name)
            },
            move |this, result, _window, cx| {
                if this.plaza_selected_course.as_deref() != Some(key.as_str()) {
                    return;
                }
                this.plaza_detail_loading = false;
                match result {
                    Ok(mut shares) => {
                        shares.sort_by(|a, b| {
                            let time = |share: &Share| {
                                let raw = if share.record_time.is_empty() {
                                    share.record_date.as_str()
                                } else {
                                    share.record_time.as_str()
                                };
                                crate::blackboard::sortable_time(raw)
                            };
                            time(b)
                                .cmp(&time(a))
                                .then_with(|| b.replay_title.cmp(&a.replay_title))
                        });
                        this.plaza_shares = shares;
                    }
                    Err(error) => {
                        this.error = Some(format!("加载课程回放失败：{error:#}"));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn download_share(
        &mut self,
        share: Share,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if share.m3u8_url.is_empty() {
            self.error = Some("这条分享没有媒体地址".to_string());
            cx.notify();
            return;
        }
        let replay = Self::replay_from_share(&share);
        let backend = self.config.backend_url.clone();
        let id = share.id.clone();
        self.run_blocking(
            window,
            cx,
            move || ShareClient::new(&backend)?.mark_download(&id),
            |_this, _result, _window, _cx| {},
        );
        self.download_many(vec![replay], window, cx);
    }

    pub(crate) fn download_plaza_course(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.plaza_shares.is_empty() {
            self.status = "该课程暂时没有可下载的回放".to_string();
            cx.notify();
            return;
        }
        let replays: Vec<Replay> = self.plaza_shares.iter().map(Self::replay_from_share).collect();
        let backend = self.config.backend_url.clone();
        let ids: Vec<String> = self
            .plaza_shares
            .iter()
            .filter(|share| {
                let replay = Self::replay_from_share(share);
                !self.replay_downloaded(&replay)
                    && !self.active_downloads.contains(&replay.identity())
            })
            .map(|share| share.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        if !ids.is_empty() {
            self.run_background(
                cx,
                move || {
                    let client = ShareClient::new(&backend)?;
                    for id in ids {
                        let _ = client.mark_download(&id);
                    }
                    Ok::<(), anyhow::Error>(())
                },
                |_this, _result, _cx| {},
            );
        }
        self.download_many(replays, window, cx);
    }

    pub(crate) fn save_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend_input.read(cx).value().trim().to_string();
        let download_dir = self.download_dir_input.read(cx).value().trim().to_string();
        let ffmpeg = self.ffmpeg_input.read(cx).value().trim().to_string();
        match Url::parse(&backend) {
            Ok(url)
                if (url.scheme() == "http" || url.scheme() == "https")
                    && url.host_str().is_some() => {}
            Ok(_) => {
                self.error = Some("后端地址必须是 http(s)://host[:port]".to_string());
                cx.notify();
                return;
            }
            Err(error) => {
                self.error = Some(format!("后端地址无效：{error}"));
                cx.notify();
                return;
            }
        }
        self.config.backend_url = backend;
        self.config.download_dir = download_dir;
        self.config.ffmpeg = ffmpeg;
        match self.config.save() {
            Ok(()) => {
                self.status = "设置已保存".to_string();
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!("保存设置失败：{error:#}"));
            }
        }
        cx.notify();
    }

    pub(crate) fn test_backend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend_input.read(cx).value().trim().to_string();
        self.busy = true;
        self.status = "测试后端…".to_string();
        self.error = None;
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || ShareClient::new(&backend)?.health(),
            |this, result, _window, cx| {
                this.busy = false;
                match result {
                    Ok(value) => {
                        let count = value.get("shares").and_then(|v| v.as_u64()).unwrap_or(0);
                        this.status = format!("后端正常，已有 {count} 条分享");
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(format!("后端不可用：{error:#}"));
                        this.status = "后端测试失败".to_string();
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn copy_text(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.status = "已复制到剪贴板".to_string();
        cx.notify();
    }

    fn replay_from_share(share: &Share) -> Replay {
        let kind = media::classify_media_url(&share.m3u8_url).to_string();
        let candidate = MediaCandidate {
            url: share.m3u8_url.clone(),
            kind,
            label: "广场".to_string(),
            quality: media::score_vod_url(&share.m3u8_url),
            guid: share.guid.clone(),
        };
        Replay {
            course_id: share.course_id.clone(),
            course_name: share.course_name.clone(),
            replay_id: share.guid.clone(),
            sub_title: share.replay_title.clone(),
            lecturer: share.teacher.clone(),
            record_time: share.record_date.clone(),
            page_url: String::new(),
            media: vec![candidate],
            guid: share.guid.clone(),
            ..Default::default()
        }
    }

    pub(crate) fn selected_course(&self) -> Option<&Course> {
        self.selected_course.and_then(|index| self.courses.get(index))
    }

    pub(crate) fn cookie_summary(&self) -> String {
        self.http
            .as_ref()
            .map(|http| format!("Cookie {} 条", http.cookie_count()))
            .unwrap_or_else(|| "未登录".to_string())
    }


}
