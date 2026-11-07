use crate::auth;
use crate::blackboard::{self, Course, Replay};
use crate::config::AppConfig;
use crate::download::{self, DownloadEvent};
use crate::http::Http;
use crate::media::{self, MediaCandidate};
use crate::share::{Share, ShareClient};
use gpui_component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, ScrollHandle, UniformListScrollHandle, Window};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use url::Url;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Courses,
    Plaza,
    Settings,
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

#[derive(Clone, Debug)]
pub struct ProgressState {
    pub title: String,
    pub downloaded: u64,
    pub total: u64,
    pub detail: String,
}

pub struct AppState {
    pub(crate) http: Option<Http>,
    pub(crate) config: AppConfig,
    pub(crate) tab: Tab,
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
    pub(crate) plaza_grid_scroll: ScrollHandle,
    pub(crate) plaza_detail_scroll: UniformListScrollHandle,
    pub(crate) collapsed_terms: HashSet<String>,
    pub(crate) sidebar_section: SidebarSection,
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
    pub(crate) replays_loading: bool,
    pub(crate) shares: Vec<Share>,
    pub(crate) progress: Option<ProgressState>,
    pub(crate) downloaded: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = AppConfig::load();
        let starred_courses: HashSet<String> = config.starred_courses.iter().cloned().collect();
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("学号"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("密码")
                .masked(true)
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

        let app = Self {
            http: None,
            config,
            tab: Tab::Courses,
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
            plaza_grid_scroll: ScrollHandle::new(),
            plaza_detail_scroll: UniformListScrollHandle::new(),
            collapsed_terms: HashSet::new(),
            sidebar_section: SidebarSection::Courses,
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
            replays_loading: false,
            shares: Vec::new(),
            progress: None,
            downloaded: 0,
            skipped: 0,
            failed: 0,
        };
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
                    this.status = "已保存的 Cookie 已失效，请重新登录".to_string();
                }
            },
        );
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
                        this.status = "登录成功".to_string();
                        this.load_courses(window, cx);
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
        if let Some(path) = self.captcha_path.take() {
            let _ = std::fs::remove_file(path);
        }
        self.verification_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.error = None;
        self.status = "已取消二次验证".to_string();
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
        self.http = None;
        self.courses.clear();
        self.replays.clear();
        self.shares.clear();
        self.selected_course = None;
        self.plaza_selected_course = None;
        self.favorite_selected = None;
        self.collapsed_terms.clear();
        self.sidebar_section = SidebarSection::Courses;
        self.pending_login = None;
        self.captcha_path = None;
        self.progress = None;
        self.replays_loading = false;
        self.downloading = false;
        self.active_downloads.clear();
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
        merged
    }

    fn load_replays_for(&mut self, course: Course, window: &mut Window, cx: &mut Context<Self>) {
        let Some(http) = self.http.clone() else {
            return;
        };
        self.replays = Self::load_replay_cache(&course);
        self.replays_loading = true;
        self.error = None;
        self.status = format!("加载回放：{}", course.name);
        cx.notify();

        let course_for_load = course.clone();
        let course_for_apply = course.clone();
        self.run_blocking(
            window,
            cx,
            move || blackboard::list_replays(&http, &course_for_load),
            move |this, result, _window, cx| {
                let current_id = this
                    .selected_course
                    .and_then(|index| this.courses.get(index))
                    .map(|current| current.id.clone());
                let is_current = current_id.as_deref() == Some(course_for_apply.id.as_str());
                if !is_current {
                    // Cache the result even if the user already switched away.
                    if let Ok(fresh) = result {
                        let cached = Self::load_replay_cache(&course_for_apply);
                        Self::save_replay_cache(
                            &course_for_apply,
                            &Self::merge_replays(cached, fresh),
                        );
                    }
                    return;
                }
                this.replays_loading = false;
                match result {
                    Ok(fresh) => {
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
                    }
                    Err(error) => {
                        this.status = "回放加载失败".to_string();
                        this.error = Some(if this.replays.is_empty() {
                            format!("{error:#}")
                        } else {
                            format!("刷新失败，显示本地缓存：{error:#}")
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
            .filter(|replay| !self.replay_downloaded(replay))
            .cloned()
            .collect();
        if pending.is_empty() {
            self.status = "当前课程的回放已全部下载".to_string();
            cx.notify();
            return;
        }
        self.download_many(pending, window, cx);
    }

    fn download_many(
        &mut self,
        replays: Vec<Replay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            self.status = "当前有任务在运行，请稍候".to_string();
            cx.notify();
            return;
        }
        let Some(http) = self.http.clone() else {
            self.error = Some("请先登录".to_string());
            cx.notify();
            return;
        };
        let replays: Vec<Replay> = replays
            .into_iter()
            .filter(|replay| !self.replay_downloaded(replay))
            .collect();
        if replays.is_empty() {
            self.status = "没有需要下载的回放（已全部下载）".to_string();
            cx.notify();
            return;
        }
        self.downloading = true;
        self.active_downloads = replays.iter().map(|replay| replay.identity()).collect();
        let output = self.config.download_path();
        let ffmpeg = crate::ffmpeg::resolve(&self.config.ffmpeg);
        self.busy = true;
        self.error = None;
        self.progress = None;
        self.downloaded = 0;
        self.skipped = 0;
        self.failed = 0;
        self.status = format!("开始处理 {} 条回放…", replays.len());
        cx.notify();

        let (tx, rx) = std::sync::mpsc::channel::<DownloadEvent>();
        std::thread::spawn(move || {
            for mut replay in replays {
                let title = replay.display_title();
                if replay.media.is_empty() {
                    tx.send(DownloadEvent::Status(format!("解析媒体：{title}"))).ok();
                    if let Err(error) = blackboard::resolve_media(&http, &mut replay) {
                        tx.send(DownloadEvent::Failed {
                            title,
                            error: format!("{error:#}"),
                        })
                        .ok();
                        continue;
                    }
                }
                match download::download_replay(&http, &replay, &output, &ffmpeg, &tx) {
                    Ok(_) => {}
                    Err(error) => {
                        tx.send(DownloadEvent::Failed {
                            title,
                            error: format!("{error:#}"),
                        })
                        .ok();
                    }
                }
            }
            tx.send(DownloadEvent::BatchDone).ok();
        });

        cx.spawn_in(window, async move |this, cx| {
            let mut done = false;
            while !done {
                while let Ok(event) = rx.try_recv() {
                    let batch_done = matches!(event, DownloadEvent::BatchDone);
                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.on_download_event(event);
                        cx.notify();
                    });
                    if batch_done {
                        done = true;
                        break;
                    }
                }
                if !done {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                }
            }
        })
        .detach();
    }

    fn on_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Status(status) => self.status = status,
            DownloadEvent::Progress {
                title,
                downloaded,
                total,
                detail,
            } => {
                self.status = title.clone();
                self.progress = Some(ProgressState {
                    title,
                    downloaded,
                    total,
                    detail,
                });
            }
            DownloadEvent::Done {
                title,
                path,
                skipped,
            } => {
                if skipped {
                    self.skipped += 1;
                    self.status = format!("已存在，跳过：{}", path.display());
                } else {
                    self.downloaded += 1;
                    self.status = format!("已保存：{}", path.display());
                }
                self.progress = None;
                let _ = title;
            }
            DownloadEvent::Failed { title, error } => {
                self.failed += 1;
                self.error = Some(format!("{title}：{error}"));
                self.status = format!("失败：{title}");
                self.progress = None;
            }
            DownloadEvent::BatchDone => {
                self.busy = false;
                self.downloading = false;
                self.active_downloads.clear();
                self.progress = None;
                self.status = format!(
                    "下载完成：新增 {}，跳过 {}，失败 {}",
                    self.downloaded, self.skipped, self.failed
                );
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
                            eprintln!("share_course published=0 errors={errors:?}");
                            this.error = Some(if errors.is_empty() {
                                "没有发布任何回放".to_string()
                            } else {
                                errors.join("\n")
                            });
                            this.status = "发布失败".to_string();
                        }
                    }
                    Err(error) => {
                        eprintln!("share_course error: {error:#}");
                        this.error = Some(format!("{error:#}"));
                        this.status = "发布失败".to_string();
                    }
                }
            },
        );
    }

    pub(crate) fn refresh_plaza(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plaza_selected_course = None;
        self.plaza_grid_scroll = ScrollHandle::new();
        self.plaza_detail_scroll = UniformListScrollHandle::new();
        let query = self.plaza_search.read(cx).value().trim().to_string();
        let backend = self.config.backend_url.clone();
        self.busy = true;
        self.error = None;
        self.status = "加载广场…".to_string();
        cx.notify();
        self.run_blocking(
            window,
            cx,
            move || ShareClient::new(&backend)?.list(&query),
            |this, result, _window, cx| {
                this.busy = false;
                match result {
                    Ok(shares) => {
                        this.shares = shares;
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.status = "广场加载失败".to_string();
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn set_sidebar_section(
        &mut self,
        section: SidebarSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_section = section;
        // Switching sections only changes the sidebar list. The right panel
        // keeps the current course/favorite until the user clicks an item.
        if section == SidebarSection::Favorites && self.shares.is_empty() && !self.busy {
            self.refresh_shares(window, cx);
        }
        self.course_scroll = UniformListScrollHandle::new();
        cx.notify();
    }

    pub(crate) fn toggle_star_course(&mut self, name: String, cx: &mut Context<Self>) {
        let removed = self.starred_courses.remove(&name);
        if !removed {
            self.starred_courses.insert(name.clone());
        }
        if self.favorite_selected.as_deref() == Some(name.as_str()) && removed {
            self.favorite_selected = None;
        }
        self.config.starred_courses = self.starred_courses.iter().cloned().collect();
        let _ = self.config.save();
        cx.notify();
    }

    pub(crate) fn open_favorite_course(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab = Tab::Courses;
        let needs_shares = !self.shares.iter().any(|share| share.course_name == name);
        self.favorite_selected = Some(name);
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
                        this.shares = shares;
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

    pub(crate) fn favorite_replays_for(&self, name: &str) -> Vec<Replay> {
        self.shares
            .iter()
            .filter(|share| share.course_name == name)
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
                }
            })
            .collect()
    }

    pub(crate) fn download_favorite_all(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replays = self.favorite_replays_for(name);
        self.download_many(replays, window, cx);
    }

    pub(crate) fn toggle_term(&mut self, term: String, cx: &mut Context<Self>) {
        if !self.collapsed_terms.remove(&term) {
            self.collapsed_terms.insert(term);
        }
        cx.notify();
    }

    pub(crate) fn select_plaza_course(&mut self, name: String, cx: &mut Context<Self>) {
        self.plaza_selected_course = Some(name);
        self.plaza_detail_scroll = UniformListScrollHandle::new();
        cx.notify();
    }

    pub(crate) fn back_to_plaza_courses(&mut self, cx: &mut Context<Self>) {
        self.plaza_selected_course = None;
        cx.notify();
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
        let kind = media::classify_media_url(&share.m3u8_url).to_string();
        let candidate = MediaCandidate {
            url: share.m3u8_url.clone(),
            kind,
            label: "广场".to_string(),
            quality: media::score_vod_url(&share.m3u8_url),
            guid: share.guid.clone(),
        };
        let replay = Replay {
            course_id: share.course_id.clone(),
            course_name: share.course_name.clone(),
            replay_id: share.guid.clone(),
            sub_title: share.replay_title.clone(),
            lecturer: share.teacher.clone(),
            record_time: share.record_date.clone(),
            page_url: String::new(),
            media: vec![candidate],
            guid: share.guid.clone(),
        };
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
