use crate::blackboard::Replay;
use crate::http::Http;
use crate::media::{self, format_bytes, format_time, sanitize_filename};
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use url::Url;

#[derive(Clone, Debug)]
pub enum DownloadEvent {
    Status(String),
    Progress {
        title: String,
        downloaded: u64,
        total: u64,
        detail: String,
    },
    Done {
        title: String,
        path: PathBuf,
        skipped: bool,
    },
    Failed {
        title: String,
        error: String,
    },
    BatchDone,
}

pub fn download_replay(
    http: &Http,
    replay: &Replay,
    output_root: &Path,
    ffmpeg: &str,
    tx: &Sender<DownloadEvent>,
) -> Result<()> {
    let candidate = replay
        .media
        .first()
        .ok_or_else(|| anyhow!("该回放没有可用媒体地址"))?
        .clone();
    let dir = output_root
        .join(sanitize_filename(&replay.course_name))
        .join("_录播回放");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let sub_title = if replay.sub_title.trim().is_empty() {
        replay.replay_id.as_str()
    } else {
        replay.sub_title.as_str()
    };
    let extension = if candidate.is_hls() {
        "mp4".to_string()
    } else {
        Url::parse(&candidate.url)
            .ok()
            .and_then(|url| {
                Path::new(url.path())
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_lowercase())
            })
            .filter(|ext| {
                ["mp4", "m4v", "mov", "mkv", "flv", "webm", "ts"].contains(&ext.as_str())
            })
            .unwrap_or_else(|| "mp4".to_string())
    };
    let output = media::output_path(&dir, "", sub_title, &replay.lecturer)
        .with_extension(extension);
    let title = replay.display_title();

    tx.send(DownloadEvent::Status(format!("准备下载：{title}"))).ok();

    if let Some(existing) = media::find_downloaded(&dir, sub_title, &replay.lecturer) {
        tx.send(DownloadEvent::Done {
            title,
            path: existing,
            skipped: true,
        })
        .ok();
        return Ok(());
    }

    if candidate.is_hls() {
        download_hls(http, &candidate.url, &output, ffmpeg, &title, tx)?;
    } else {
        download_file(http, &candidate.url, &output, &title, tx)?;
    }

    tx.send(DownloadEvent::Done {
        title,
        path: output.clone(),
        skipped: false,
    })
    .ok();
    Ok(())
}

fn download_file(
    http: &Http,
    url: &str,
    output: &Path,
    title: &str,
    tx: &Sender<DownloadEvent>,
) -> Result<()> {
    let temporary = output.with_extension("part.mp4");
    let mut offset = temporary.metadata().map(|m| m.len()).unwrap_or(0);
    let mut range = Vec::new();
    if offset > 0 {
        range.push(("Range", format!("bytes={offset}-")));
    }
    let range_refs: Vec<(&str, &str)> = range
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let response = http.get_response_follow(url, &range_refs)?;
    let status = response.status().as_u16();
    if status == 416 && offset > 0 {
        let total = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.trim().parse::<u64>().ok());
        if total.map(|total| offset >= total).unwrap_or(false) {
            media::replace_file(&temporary, output)?;
            return Ok(());
        }
    }
    if !(200..300).contains(&status) {
        return Err(anyhow!("媒体返回 HTTP {status}"));
    }
    let append = status == 206 && offset > 0;
    if status == 200 && offset > 0 {
        offset = 0;
    }
    let content_length = response.content_length().unwrap_or(0);
    let total = if append { offset + content_length } else { content_length };
    let mut response = response;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&temporary)
        .with_context(|| format!("open {}", temporary.display()))?;
    let mut downloaded = offset;
    let started = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut buffer = [0u8; 256 * 1024];
    loop {
        let read = response.read(&mut buffer).context("read media stream")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        downloaded += read as u64;
        if last_emit.elapsed() >= Duration::from_millis(250) {
            emit_progress(tx, title, downloaded, total, started, "下载", true);
            last_emit = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    if total > 0 && downloaded != total {
        return Err(anyhow!(
            "下载不完整：期望 {}，实际 {}",
            format_bytes(total),
            format_bytes(downloaded)
        ));
    }
    media::replace_file(&temporary, output).with_context(|| format!("rename to {}", output.display()))?;
    emit_progress(tx, title, downloaded, total.max(downloaded), started, "完成", true);
    Ok(())
}

fn download_hls(
    http: &Http,
    url: &str,
    output: &Path,
    ffmpeg: &str,
    title: &str,
    tx: &Sender<DownloadEvent>,
) -> Result<()> {
    tx.send(DownloadEvent::Status(format!("解析 HLS：{title}"))).ok();
    let (playlist_url, playlist) = fetch_media_playlist(http, url, 0)?;
    let duration = playlist_duration(&playlist);
    let cache_dir = output.with_file_name(format!(
        "{}.segments",
        output
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("replay")
    ));
    fs::create_dir_all(&cache_dir).with_context(|| format!("create {}", cache_dir.display()))?;
    let identity = playlist_identity(&playlist, &playlist_url);
    let identity_file = cache_dir.join("playlist.id");
    if identity_file.exists() {
        let old = fs::read_to_string(&identity_file).unwrap_or_default();
        if old.trim() != identity {
            fs::remove_dir_all(&cache_dir)?;
            fs::create_dir_all(&cache_dir)?;
        }
    }
    fs::write(&identity_file, &identity)?;

    let lines: Vec<String> = playlist
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    let mut segments: Vec<(Url, f64)> = Vec::new();
    let mut pending_duration = 0.0f64;
    for line in &lines {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            pending_duration = value
                .split(',')
                .next()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let remote = playlist_url
            .join(line)
            .with_context(|| format!("bad segment URL {line}"))?;
        segments.push((remote, pending_duration));
        pending_duration = 0.0;
    }
    if segments.is_empty() {
        return Err(anyhow!("HLS playlist 没有分片"));
    }

    let total = segments.len();
    let started = Instant::now();
    let mut local_names = Vec::with_capacity(total);
    let mut ready_duration = 0.0f64;
    for (index, (remote, segment_duration)) in segments.iter().enumerate() {
        let suffix = Path::new(remote.path())
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!(".{ext}"))
            .unwrap_or_else(|| ".ts".to_string());
        let name = format!("segment_{:06}{suffix}", index + 1);
        let local_path = cache_dir.join(&name);
        if !local_path.is_file() || local_path.metadata().map(|m| m.len() == 0).unwrap_or(true) {
            let bytes = http
                .get_bytes(remote.as_str())
                .with_context(|| format!("下载分片 {}/{}", index + 1, total))?;
            write_atomic(&local_path, &bytes)?;
        }
        local_names.push(name);
        ready_duration += segment_duration;
        let detail = format!(
            "分片 {}/{}，已就绪 {}",
            index + 1,
            total,
            format_time(ready_duration)
        );
        emit_progress(tx, title, (index + 1) as u64, total as u64, started, &detail, false);
    }

    let mut local_lines = Vec::new();
    let mut auxiliary: HashMap<String, String> = HashMap::new();
    let mut segment_index = 0usize;
    for line in &lines {
        if line.starts_with("#EXTINF:") {
            local_lines.push(line.clone());
        } else if line.starts_with("#EXT-X-KEY:") || line.starts_with("#EXT-X-MAP:") {
            local_lines.push(rewrite_uri_attribute(
                http,
                &playlist_url,
                line,
                &cache_dir,
                &mut auxiliary,
            )?);
        } else if line.starts_with('#') {
            local_lines.push(line.clone());
        } else if segment_index < local_names.len() {
            local_lines.push(local_names[segment_index].clone());
            segment_index += 1;
        }
    }

    let local_playlist = cache_dir.join("local.m3u8");
    fs::write(&local_playlist, local_lines.join("\n") + "\n")?;

    let temporary = output.with_extension("part.mp4");
    let status = run_ffmpeg(ffmpeg, &local_playlist, &temporary, duration, title, tx)
        .context("调用 ffmpeg 合并 HLS")?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(anyhow!(
            "ffmpeg 合并失败（退出码 {:?}）；分片缓存保留在 {}",
            status.code(),
            cache_dir.display()
        ));
    }
    media::replace_file(&temporary, output).with_context(|| format!("rename to {}", output.display()))?;
    let _ = fs::remove_dir_all(&cache_dir);
    Ok(())
}

fn emit_progress(
    tx: &Sender<DownloadEvent>,
    title: &str,
    downloaded: u64,
    total: u64,
    started: Instant,
    detail: &str,
    bytes_based: bool,
) {
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let speed = if bytes_based && total > 0 {
        format!("{}/s", format_bytes((downloaded as f64 / elapsed) as u64))
    } else {
        String::new()
    };
    let detail = if speed.is_empty() {
        detail.to_string()
    } else {
        format!("{detail} · {speed}")
    };
    tx.send(DownloadEvent::Progress {
        title: title.to_string(),
        downloaded,
        total,
        detail,
    })
    .ok();
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("part");
    fs::write(&temporary, bytes)?;
    media::replace_file(&temporary, path)?;
    Ok(())
}

fn rewrite_uri_attribute(
    http: &Http,
    base: &Url,
    line: &str,
    cache_dir: &Path,
    auxiliary: &mut HashMap<String, String>,
) -> Result<String> {
    static URI_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"URI="([^"]+)""#).unwrap());
    let Some(captures) = URI_RE.captures(line) else {
        return Ok(line.to_string());
    };
    let Some(uri) = captures.get(1) else {
        return Ok(line.to_string());
    };
    let remote = base
        .join(uri.as_str())
        .with_context(|| format!("bad HLS URI {}", uri.as_str()))?;
    let key = remote.to_string();
    let name = if let Some(name) = auxiliary.get(&key) {
        name.clone()
    } else {
        let kind = if line.starts_with("#EXT-X-KEY") { "key" } else { "init" };
        let suffix = Path::new(remote.path())
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!(".{ext}"))
            .unwrap_or_else(|| ".bin".to_string());
        let name = format!("{kind}_{:03}{suffix}", auxiliary.len() + 1);
        let bytes = http.get_bytes_follow(remote.as_str())?;
        write_atomic(&cache_dir.join(&name), &bytes)?;
        auxiliary.insert(key, name.clone());
        name
    };
    Ok(URI_RE
        .replace(line, format!("URI=\"{name}\""))
        .to_string())
}

fn run_ffmpeg(
    ffmpeg: &str,
    local_playlist: &Path,
    output: &Path,
    duration: f64,
    title: &str,
    tx: &Sender<DownloadEvent>,
) -> Result<std::process::ExitStatus> {
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-y",
            "-protocol_whitelist",
            "file,crypto,data",
            "-allowed_extensions",
            "ALL",
            "-i",
        ])
        .arg(local_playlist)
        .args(["-c", "copy", "-progress", "pipe:1", "-nostats"])
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .with_context(|| {
            format!("启动 ffmpeg ({ffmpeg})，请确认内置 ffmpeg 存在或在设置中指定路径")
        })?;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let started = Instant::now();
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        for line in reader.lines().map_while(Result::ok) {
            let Some(value) = line.strip_prefix("out_time_ms=") else {
                continue;
            };
            let seconds = value.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
            if last_emit.elapsed() >= Duration::from_millis(250) {
                let detail = if duration > 0.0 {
                    format!(
                        "ffmpeg {} / {}",
                        format_time(seconds),
                        format_time(duration)
                    )
                } else {
                    format!("ffmpeg {}", format_time(seconds))
                };
                emit_progress(
                    tx,
                    title,
                    (seconds * 1000.0) as u64,
                    (duration * 1000.0) as u64,
                    started,
                    &detail,
                    false,
                );
                last_emit = Instant::now();
            }
        }
    }
    Ok(child.wait()?)
}

fn fetch_media_playlist(http: &Http, url: &str, depth: usize) -> Result<(Url, String)> {
    if depth > 5 {
        return Err(anyhow!("HLS playlist 嵌套过深"));
    }
    let fetched = http.get_follow(url, 10)?;
    if !fetched.is_success() {
        return Err(anyhow!("HLS playlist 返回 HTTP {}", fetched.status));
    }
    let base = fetched.final_url.clone();
    let body = fetched.body;
    if !body.contains("#EXTM3U") {
        return Err(anyhow!("媒体地址不是 HLS playlist"));
    }
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let mut variants: Vec<(u64, String)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.starts_with("#EXT-X-STREAM-INF:") {
            continue;
        }
        let bandwidth = line
            .split(',')
            .find_map(|part| part.trim().strip_prefix("BANDWIDTH="))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        if let Some(next) = lines.get(index + 1) {
            if !next.starts_with('#') {
                variants.push((bandwidth, (*next).to_string()));
            }
        }
    }
    if variants.is_empty() {
        return Ok((base, body));
    }
    variants.sort_by_key(|(bandwidth, _)| *bandwidth);
    let (_, chosen) = variants.pop().unwrap();
    let chosen_url = base.join(&chosen).context("resolve HLS variant")?;
    fetch_media_playlist(http, chosen_url.as_str(), depth + 1)
}

fn playlist_identity(body: &str, base: &Url) -> String {
    use std::hash::{Hash, Hasher};
    static URI: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"URI="([^"]+)""#).unwrap());
    let mut normalized = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            let rewritten = URI.replace_all(line, |captures: &regex::Captures<'_>| {
                let target = captures.get(1).map(|m| m.as_str()).unwrap_or("");
                let clean = base
                    .join(target)
                    .map(|url| strip_query(&url))
                    .unwrap_or_else(|_| target.to_string());
                format!(r#"URI="{}""#, clean)
            });
            normalized.push(rewritten.to_string());
        } else {
            let clean = base
                .join(line)
                .map(|url| strip_query(&url))
                .unwrap_or_else(|_| line.to_string());
            normalized.push(clean);
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.join("\n").hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn strip_query(url: &Url) -> String {
    let mut clean = url.clone();
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

fn playlist_duration(body: &str) -> f64 {
    static EXTINF: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"#EXTINF:([0-9]+(?:\.[0-9]+)?)").unwrap());
    EXTINF
        .captures_iter(body)
        .filter_map(|captures| captures.get(1))
        .filter_map(|value| value.as_str().parse::<f64>().ok())
        .sum()
}

#[cfg(all(test, unix))]
mod hls_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    fn serve(listener: TcpListener) {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let Ok(read) = stream.read(&mut request) else { continue };
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let body: Vec<u8> = match path {
                "/master.m3u8" => b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900\nmedia.m3u8\n".to_vec(),
                "/media.m3u8" => b"#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x00000000000000000000000000000001\n#EXTINF:1.0,\nseg0.ts\n#EXTINF:1.0,\nseg1.ts\n#EXT-X-ENDLIST\n".to_vec(),
                "/seg0.ts" => vec![0x47; 188],
                "/seg1.ts" => vec![0x47; 188],
                "/key.bin" => vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
                "/init.mp4" => b"fake-init".to_vec(),
                _ => b"not found".to_vec(),
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    }

    #[test]
    fn playlist_identity_ignores_expiring_query() {
        let base = Url::parse("https://resourcese.pku.edu.cn/play/media.m3u8").unwrap();
        let first = "#EXTM3U\n#EXTINF:1.0,\nseg.ts?sig=1\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin?sig=1\"\n";
        let second = "#EXTM3U\n#EXTINF:1.0,\nseg.ts?sig=2\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin?sig=2\"\n";
        assert_eq!(playlist_identity(first, &base), playlist_identity(second, &base));
    }

    #[test]
    fn downloads_master_playlist_segments_and_auxiliary_files() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || serve(listener));

        let root = std::env::temp_dir().join(format!("course-share-hls-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = root.join("out.mp4");
        let dump = root.join("local.m3u8.dump");
        let fake_ffmpeg = root.join("fake-ffmpeg.sh");
        fs::write(
            &fake_ffmpeg,
            "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"-i\" ]; then shift; cp \"$1\" \"$TEST_FFMPEG_DUMP\"; fi\n  out=\"$1\"; shift\ndone\nprintf fake > \"$out\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake_ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("TEST_FFMPEG_DUMP", &dump);

        let http = Http::new(root.join("cookies.json")).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        download_hls(
            &http,
            &format!("http://{address}/master.m3u8"),
            &output,
            fake_ffmpeg.to_str().unwrap(),
            "test",
            &tx,
        )
        .unwrap();

        assert!(output.is_file());
        assert_eq!(fs::read_to_string(&output).unwrap(), "fake");
        let playlist = fs::read_to_string(&dump).unwrap();
        assert!(playlist.contains("segment_000001.ts"));
        assert!(playlist.contains("segment_000002.ts"));
        assert!(playlist.contains("URI=\"key_002.bin\""));
        assert!(playlist.contains("URI=\"init_001.mp4\""));
        assert!(!playlist.contains("seg0.ts"));
        assert!(!output.with_file_name("out.mp4.segments").exists());
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(all(test, unix))]
mod file_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(listener: TcpListener) {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let Ok(read) = stream.read(&mut request) else { continue };
            let request = String::from_utf8_lossy(&request[..read]);
            let start = request
                .lines()
                .find_map(|line| line.strip_prefix("Range: bytes="))
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let body = vec![7u8; 1000];
            let (status, payload, content_range) = if start > 0 {
                (
                    "206 Partial Content",
                    body[start..].to_vec(),
                    Some(format!("bytes {start}-999/1000")),
                )
            } else {
                ("200 OK", body, None)
            };
            let mut header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: video/mp4\r\nConnection: close\r\n",
                payload.len()
            );
            if let Some(range) = content_range {
                header.push_str(&format!("Content-Range: {range}\r\n"));
            }
            header.push_str("\r\n");
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&payload);
        }
    }

    #[test]
    fn resumes_partial_mp4_with_range() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || serve(listener));

        let root = std::env::temp_dir().join(format!("course-share-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = root.join("video.mp4");
        let partial = output.with_extension("part.mp4");
        fs::write(&partial, vec![7u8; 300]).unwrap();

        let http = Http::new(root.join("cookies.json")).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        download_file(
            &http,
            &format!("http://{address}/video.mp4"),
            &output,
            "test",
            &tx,
        )
        .unwrap();

        assert_eq!(fs::read(&output).unwrap(), vec![7u8; 1000]);
        assert!(!partial.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
