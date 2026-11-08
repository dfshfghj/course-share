use crate::blackboard::{self, Replay};
use crate::download::{self, DownloadControl, DownloadEvent, MergeOutcome, Prepared, PreparedHls};
use crate::http::Http;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

/// Replays downloading at the same time. Each of them additionally fans out
/// HLS segments, so keep this modest to avoid saturating the remote host.
pub const MAX_PARALLEL_DOWNLOADS: usize = 3;
/// ffmpeg merges are CPU/disk bound and do not consume network slots, so they
/// get their own small pool while new downloads keep starting.
pub const MAX_PARALLEL_MERGES: usize = 2;

pub struct DownloadScheduler {
    events: Sender<DownloadEvent>,
    jobs: Sender<Job>,
    merges: Sender<MergeJob>,
}

struct Job {
    attempt: u64,
    id: String,
    http: Http,
    replay: Replay,
    output: PathBuf,
    ffmpeg: String,
    control: Arc<DownloadControl>,
    events: Sender<DownloadEvent>,
    merges: Sender<MergeJob>,
}

struct MergeJob {
    attempt: u64,
    id: String,
    title: String,
    ffmpeg: String,
    prepared: PreparedHls,
    control: Arc<DownloadControl>,
    events: Sender<DownloadEvent>,
}

impl DownloadScheduler {
    pub fn new(events: Sender<DownloadEvent>) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (merge_tx, merge_rx) = mpsc::channel::<MergeJob>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let merge_rx = Arc::new(Mutex::new(merge_rx));

        for _ in 0..MAX_PARALLEL_MERGES {
            let rx = merge_rx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let receiver = match rx.lock() {
                        Ok(receiver) => receiver,
                        Err(_) => return,
                    };
                    match receiver.recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    }
                };
                run_merge(job);
            });
        }
        for _ in 0..MAX_PARALLEL_DOWNLOADS {
            let rx = job_rx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let receiver = match rx.lock() {
                        Ok(receiver) => receiver,
                        Err(_) => return,
                    };
                    match receiver.recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    }
                };
                run_download(job);
            });
        }

        Self {
            events,
            jobs: job_tx,
            merges: merge_tx,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &self,
        http: Http,
        replay: Replay,
        output: PathBuf,
        ffmpeg: String,
        attempt: u64,
        control: Arc<DownloadControl>,
    ) -> bool {
        let id = replay.identity();
        self.jobs
            .send(Job {
                attempt,
                id,
                http,
                replay,
                output,
                ffmpeg,
                control,
                events: self.events.clone(),
                merges: self.merges.clone(),
            })
            .is_ok()
    }
}

fn run_download(job: Job) {
    let Job {
        attempt,
        id,
        http,
        mut replay,
        output,
        ffmpeg,
        control,
        events,
        merges,
    } = job;
    let title = replay.display_title();

    if control.aborted() {
        send_abort(&events, attempt, &id, &title, &control);
        return;
    }
    send_progress(&events, attempt, &id, &title, 0, 0, "准备下载…", false);

    if replay.media.is_empty() {
        send_progress(&events, attempt, &id, &title, 0, 0, "解析媒体地址…", false);
        if let Err(error) = blackboard::resolve_media(&http, &mut replay) {
            if control.aborted() {
                send_abort(&events, attempt, &id, &title, &control);
            } else {
                send_failed(&events, attempt, &id, &title, error);
            }
            return;
        }
    }

    match download::download_network(
        &http,
        &replay,
        &output,
        attempt,
        &id,
        &control,
        &events,
    ) {
        Ok(Prepared::Stopped) => send_abort(&events, attempt, &id, &title, &control),
        Ok(Prepared::Skipped { path }) => send_done(events, attempt, id, title, path, true),
        Ok(Prepared::Complete { path }) => send_done(events, attempt, id, title, path, false),
        Ok(Prepared::Hls(prepared)) => {
            send_progress(&events, attempt, &id, &title, 0, 0, "等待合并…", true);
            let merge = MergeJob {
                attempt,
                id: id.clone(),
                title: title.clone(),
                ffmpeg,
                prepared,
                control: control.clone(),
                events: events.clone(),
            };
            if merges.send(merge).is_err() {
                send_failed(
                    &events,
                    attempt,
                    &id,
                    &title,
                    anyhow::anyhow!("合并线程不可用"),
                );
            }
        }
        Err(error) => {
            if control.aborted() {
                send_abort(&events, attempt, &id, &title, &control);
            } else {
                send_failed(&events, attempt, &id, &title, error);
            }
        }
    }
}

fn run_merge(job: MergeJob) {
    let MergeJob {
        attempt,
        id,
        title,
        ffmpeg,
        prepared,
        control,
        events,
    } = job;
    match download::merge_hls(
        prepared,
        &ffmpeg,
        attempt,
        &id,
        &title,
        &control,
        &events,
    ) {
        Ok(MergeOutcome::Completed(path)) => send_done(events, attempt, id, title, path, false),
        Ok(MergeOutcome::Stopped) => send_abort(&events, attempt, &id, &title, &control),
        Err(error) => {
            if control.aborted() {
                send_abort(&events, attempt, &id, &title, &control);
            } else {
                send_failed(&events, attempt, &id, &title, error);
            }
        }
    }
}

fn send_progress(
    events: &Sender<DownloadEvent>,
    attempt: u64,
    id: &str,
    title: &str,
    downloaded: u64,
    total: u64,
    detail: &str,
    merging: bool,
) {
    events
        .send(DownloadEvent::Progress {
            attempt,
            id: id.to_string(),
            title: title.to_string(),
            downloaded,
            total,
            detail: detail.to_string(),
            merging,
        })
        .ok();
}

fn send_done(
    events: Sender<DownloadEvent>,
    attempt: u64,
    id: String,
    title: String,
    path: PathBuf,
    skipped: bool,
) {
    events
        .send(DownloadEvent::Done {
            attempt,
            id,
            title,
            path,
            skipped,
        })
        .ok();
}

fn send_abort(
    events: &Sender<DownloadEvent>,
    attempt: u64,
    id: &str,
    title: &str,
    control: &DownloadControl,
) {
    let event = if control.is_stopped() {
        DownloadEvent::Stopped {
            attempt,
            id: id.to_string(),
            title: title.to_string(),
        }
    } else {
        DownloadEvent::Paused {
            attempt,
            id: id.to_string(),
            title: title.to_string(),
        }
    };
    events.send(event).ok();
}

fn send_failed(
    events: &Sender<DownloadEvent>,
    attempt: u64,
    id: &str,
    title: &str,
    error: anyhow::Error,
) {
    events
        .send(DownloadEvent::Failed {
            attempt,
            id: id.to_string(),
            title: title.to_string(),
            error: format!("{error:#}"),
        })
        .ok();
}
