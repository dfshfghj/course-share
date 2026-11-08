use crate::config::AppConfig;
use crate::http::Http;
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

const HLS_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/hls.min.js"));
const SESSION_TTL: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone)]
struct PreviewSession {
    id: String,
    http: Http,
    url: String,
    title: String,
    is_hls: bool,
    created: Instant,
}

struct PreviewServer {
    base: String,
    sessions: Arc<Mutex<HashMap<String, PreviewSession>>>,
}

static SERVER: OnceLock<Result<PreviewServer, String>> = OnceLock::new();

pub fn open(http: Http, title: String, url: String) -> Result<()> {
    let parsed = Url::parse(&url).context("invalid preview URL")?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(anyhow!("预览地址必须是 http(s)"));
    }
    let server = server()?;
    let id = format!("{:032x}", rand::random::<u128>());
    let is_hls = parsed.path().to_ascii_lowercase().ends_with(".m3u8");
    {
        let mut sessions = server
            .sessions
            .lock()
            .map_err(|_| anyhow!("preview session store poisoned"))?;
        sessions.retain(|_, session| session.created.elapsed() < SESSION_TTL);
        if sessions.len() >= 64 {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.created)
                .map(|(id, _)| id.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(
            id.clone(),
            PreviewSession {
                id: id.clone(),
                http,
                url,
                title,
                is_hls,
                created: Instant::now(),
            },
        );
    }
    open_browser(&format!("{}/preview/{id}", server.base))
}

fn server() -> Result<&'static PreviewServer> {
    let result = SERVER.get_or_init(|| {
        start_server().map_err(|error| format!("{error:#}"))
    });
    result
        .as_ref()
        .map_err(|error| anyhow!("预览代理启动失败：{error}"))
}

fn start_server() -> Result<PreviewServer> {
    let server = Server::http("127.0.0.1:0").map_err(|error| anyhow!("绑定预览代理失败：{error}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("预览代理没有 IP 地址"))?
        .port();
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let sessions_for_thread = sessions.clone();
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let sessions = sessions_for_thread.clone();
            thread::spawn(move || {
                let _ = handle_request(request, &sessions);
            });
        }
    });
    Ok(PreviewServer {
        base: format!("http://127.0.0.1:{port}"),
        sessions,
    })
}

fn handle_request(request: Request, sessions: &Arc<Mutex<HashMap<String, PreviewSession>>>) -> Result<()> {
    if request.method() != &Method::Get {
        return respond_text(request, 405, "method not allowed");
    }
    let local = Url::parse(&format!("http://127.0.0.1{}", request.url()))
        .context("parse preview request URL")?;
    let parts: Vec<&str> = local
        .path_segments()
        .map(|segments| segments.collect())
        .unwrap_or_default();
    if parts.len() < 2 || parts[0] != "preview" {
        return respond_text(request, 404, "not found");
    }
    let id = parts[1];
    if parts.len() == 2 {
        let session = session(sessions, id)?;
        return respond_html(request, &render_player(id, &session));
    }
    if parts.len() == 3 && parts[2] == "hls.min.js" {
        return respond_javascript(request);
    }
    if parts.len() == 3 && parts[2] == "fetch" {
        let session = session(sessions, id)?;
        let encoded = local
            .query_pairs()
            .find(|(key, _)| key == "u")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| anyhow!("missing u parameter"))?;
        let remote = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded.as_bytes())
                .context("decode media URL")?,
        )
        .context("media URL is not UTF-8")?;
        let remote = Url::parse(&remote).context("invalid media URL")?;
        if !allowed_media_url(&remote, &session.url) {
            return respond_text(request, 403, "media host not allowed");
        }
        return proxy_media(request, &session, &remote);
    }
    respond_text(request, 404, "not found")
}

fn session(
    sessions: &Arc<Mutex<HashMap<String, PreviewSession>>>,
    id: &str,
) -> Result<PreviewSession> {
    let sessions = sessions
        .lock()
        .map_err(|_| anyhow!("preview session store poisoned"))?;
    sessions
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow!("preview session not found"))
}

fn proxy_media(request: Request, session: &PreviewSession, remote: &Url) -> Result<()> {
    if remote.path().to_ascii_lowercase().ends_with(".m3u8") {
        let fetched = session
            .http
            .get_follow(remote.as_str(), 10)
            .with_context(|| format!("fetch playlist {}", redacted_host(remote)))?;
        if !(200..300).contains(&fetched.status) {
            return respond_text(request, fetched.status, "playlist fetch failed");
        }
        let body = rewrite_playlist(&fetched.body, &fetched.final_url, session)?;
        return respond_playlist(request, &body);
    }

    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str().to_string());
    let headers: Vec<(&str, &str)> = range
        .as_deref()
        .map(|value| vec![("Range", value)])
        .unwrap_or_default();
    let response = session
        .http
        .get_response_follow(remote.as_str(), &headers)
        .with_context(|| format!("fetch media {}", redacted_host(remote)))?;
    let status = StatusCode(response.status().as_u16());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut response_headers = vec![
        header("Content-Type", &content_type),
        header("Cache-Control", "no-store"),
    ];
    if let Some(value) = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        response_headers.push(header("Content-Range", value));
    }
    if let Some(value) = response
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
    {
        response_headers.push(header("Accept-Ranges", value));
    }
    let length = response.content_length().map(|value| value as usize);
    let tiny = Response::new(status, response_headers, response, length, None);
    request.respond(tiny).ok();
    Ok(())
}

fn rewrite_playlist(body: &str, base: &Url, session: &PreviewSession) -> Result<String> {
    let mut out = String::with_capacity(body.len() + 256);
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.contains("URI=\"") {
            out.push_str(&rewrite_uri_attributes(trimmed, base, session)?);
        } else if trimmed.starts_with('#') || trimmed.is_empty() {
            out.push_str(line);
        } else {
            let remote = base.join(trimmed).context("resolve playlist URL")?;
            out.push_str(&local_fetch_url(session, &remote));
        }
        out.push('\n');
    }
    Ok(out)
}

fn rewrite_uri_attributes(line: &str, base: &Url, session: &PreviewSession) -> Result<String> {
    static URI: OnceLock<regex::Regex> = OnceLock::new();
    let regex = URI.get_or_init(|| regex::Regex::new(r#"URI="([^"]+)""#).unwrap());
    let mut out = String::with_capacity(line.len() + 128);
    let mut last = 0;
    for captures in regex.captures_iter(line) {
        let whole = captures.get(0).unwrap();
        let value = captures.get(1).unwrap();
        out.push_str(&line[last..whole.start()]);
        let remote = base.join(value.as_str()).context("resolve URI attribute")?;
        out.push_str(&format!("URI=\"{}\"", local_fetch_url(session, &remote)));
        last = whole.end();
    }
    out.push_str(&line[last..]);
    Ok(out)
}

fn local_fetch_url(session: &PreviewSession, remote: &Url) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(remote.as_str());
    format!("/preview/{}/fetch?u={encoded}", session.id)
}

fn render_player(id: &str, session: &PreviewSession) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session.url.as_str());
    let src = format!("/preview/{id}/fetch?u={encoded}");
    let title = html_escape(&session.title);
    PLAYER_HTML
        .replace("__TITLE__", &title)
        .replace("__SRC__", &src)
        .replace("__HLS__", if session.is_hls { "true" } else { "false" })
        .replace("__JS__", &format!("/preview/{id}/hls.min.js"))
}

fn allowed_media_url(remote: &Url, session_url: &str) -> bool {
    let Some(host) = remote.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let session_host = Url::parse(session_url)
        .ok()
        .and_then(|url| url.host_str().map(|value| value.to_ascii_lowercase()))
        .unwrap_or_default();
    host == session_host || host == "pku.edu.cn" || host.ends_with(".pku.edu.cn")
}

fn redacted_host(url: &Url) -> String {
    format!("{}://{}", url.scheme(), url.host_str().unwrap_or("unknown"))
}

fn respond_html(request: Request, body: &str) -> Result<()> {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "text/html; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    request.respond(response).ok();
    Ok(())
}

fn respond_javascript(request: Request) -> Result<()> {
    let response = Response::from_string(HLS_JS)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "application/javascript; charset=utf-8"))
        .with_header(header("Cache-Control", "public, max-age=86400"));
    request.respond(response).ok();
    Ok(())
}

fn respond_playlist(request: Request, body: &str) -> Result<()> {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "application/vnd.apple.mpegurl"))
        .with_header(header("Cache-Control", "no-store"));
    request.respond(response).ok();
    Ok(())
}

fn respond_text(request: Request, status: u16, body: &str) -> Result<()> {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "text/plain; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    request.respond(response).ok();
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // A default Firefox often refuses to start when another
        // non-responding instance owns its profile. Prefer Chromium with an
        // isolated profile; fall back to Firefox with its own profile.
        for browser in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ] {
            if command_exists(browser) {
                let profile = AppConfig::data_dir().join("preview-chrome");
                let _ = fs::create_dir_all(&profile);
                let mut command = Command::new(browser);
                command
                    .arg(format!("--user-data-dir={}", profile.display()))
                    .arg("--no-first-run")
                    .arg("--no-default-browser-check")
                    .arg("--new-window")
                    .arg(url);
                return spawn_browser(command);
            }
        }
        if command_exists("firefox") {
            let profile = AppConfig::data_dir().join("preview-firefox");
            let _ = fs::create_dir_all(&profile);
            let mut command = Command::new("firefox");
            command
                .arg("--no-remote")
                .arg("--new-instance")
                .arg("--profile")
                .arg(&profile)
                .arg(url);
            return spawn_browser(command);
        }
        let mut command = Command::new("xdg-open");
        command.arg(url);
        return spawn_browser(command);
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        return spawn_browser(command);
    }

    #[cfg(windows)]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        return spawn_browser(command);
    }

    #[allow(unreachable_code)]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        spawn_browser(command)
    }
}

fn spawn_browser(mut command: Command) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("打开系统浏览器失败")?;
    Ok(())
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths).any(|dir| {
                dir.join(name)
                    .metadata()
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

const PLAYER_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__</title>
<style>
  html,body{margin:0;height:100%;background:#0b0f17;color:#e5e7eb;font:14px/1.5 system-ui,sans-serif}
  body{display:flex;flex-direction:column}
  header{padding:10px 16px;background:#111827;border-bottom:1px solid #1f2937;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  main{flex:1;display:flex;align-items:center;justify-content:center;min-height:0;padding:12px}
  video{width:100%;height:100%;max-height:calc(100vh - 120px);background:#000;outline:0}
  #error{display:none;color:#fca5a5;padding:12px;white-space:pre-wrap}
</style>
</head>
<body>
<header>__TITLE__</header>
<main>
  <video id="video" controls autoplay playsinline></video>
  <div id="error"></div>
</main>
<script src="__JS__"></script>
<script>
const video = document.getElementById('video');
const errorBox = document.getElementById('error');
const source = "__SRC__";
const isHls = __HLS__;
function fail(message) {
  errorBox.textContent = message;
  errorBox.style.display = 'block';
}
if (!isHls) {
  video.src = source;
} else if (video.canPlayType('application/vnd.apple.mpegurl')) {
  video.src = source;
} else if (window.Hls && Hls.isSupported()) {
  const hls = new Hls({ enableWorker: true });
  hls.loadSource(source);
  hls.attachMedia(video);
  hls.on(Hls.Events.ERROR, (_event, data) => {
    if (data && data.fatal) {
      fail('HLS 播放失败：' + (data.details || data.type || 'unknown'));
    }
  });
} else {
  fail('当前浏览器不支持 HLS 播放。');
}
video.addEventListener('error', () => fail('视频加载失败。'));
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> PreviewSession {
        let path = std::env::temp_dir().join(format!("preview-test-{}.json", std::process::id()));
        PreviewSession {
            id: "abc".to_string(),
            http: Http::new(&path).unwrap(),
            url: "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/01/02/0123456789abcdef0123456789abcdef/0/master.m3u8".to_string(),
            title: "测试".to_string(),
            is_hls: true,
            created: Instant::now(),
        }
    }

    #[test]
    fn rewrites_playlist_uris_to_local_proxy() {
        let base = Url::parse(
            "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/01/02/0123456789abcdef0123456789abcdef/0/master.m3u8",
        )
        .unwrap();
        let playlist = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:1.0,\nseg.ts\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmedia.m3u8\n";
        let rewritten = rewrite_playlist(playlist, &base, &test_session()).unwrap();
        assert!(rewritten.contains("/preview/abc/fetch?u="));
        assert!(rewritten.contains("URI=\"/preview/abc/fetch?u="));
        assert!(!rewritten.contains("URI=\"key.bin\""));
        assert!(!rewritten.lines().any(|line| line == "seg.ts"));
        assert!(!rewritten.lines().any(|line| line == "media.m3u8"));
    }

    #[test]
    fn only_proxies_pku_media_hosts() {
        let session = "https://resourcese.pku.edu.cn/play/a.m3u8";
        assert!(allowed_media_url(
            &Url::parse("https://resourcese.pku.edu.cn/key").unwrap(),
            session
        ));
        assert!(allowed_media_url(
            &Url::parse("https://vodyy.pku.edu.cn/file.mp4").unwrap(),
            session
        ));
        assert!(!allowed_media_url(
            &Url::parse("http://127.0.0.1/secret").unwrap(),
            session
        ));
    }

    #[test]
    fn serves_rewritten_playlist_and_key() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let remote = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().take(8) {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let Ok(read) = stream.read(&mut buffer) else { continue };
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body: Vec<u8> = match path {
                    "/master.m3u8" => b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmedia.m3u8\n".to_vec(),
                    "/media.m3u8" => b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:1.0,\nseg.ts\n#EXT-X-ENDLIST\n".to_vec(),
                    "/key.bin" => vec![1; 16],
                    "/seg.ts" => vec![0x47; 188],
                    _ => b"not found".to_vec(),
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });

        let preview_server = start_server().unwrap();
        let id = format!("test-{:x}", rand::random::<u32>());
        let remote_url = format!("http://{remote}/master.m3u8");
        let path = std::env::temp_dir().join(format!("preview-proxy-{}.json", std::process::id()));
        preview_server.sessions.lock().unwrap().insert(
            id.clone(),
            PreviewSession {
                id: id.clone(),
                http: Http::new(&path).unwrap(),
                url: remote_url.clone(),
                title: "代理测试".to_string(),
                is_hls: true,
                created: Instant::now(),
            },
        );
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let html = client
            .get(format!("{}/preview/{id}", preview_server.base))
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert!(html.contains(&format!("/preview/{id}/fetch?u=")));
        assert!(html.contains("hls.min.js"));

        let encode = |url: &str| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url.as_bytes())
        };
        let master = client
            .get(format!(
                "{}/preview/{id}/fetch?u={}",
                preview_server.base,
                encode(&remote_url)
            ))
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert!(master.contains(&format!("/preview/{id}/fetch?u=")));
        assert!(!master.lines().any(|line| line == "media.m3u8"));

        let media = client
            .get(format!(
                "{}/preview/{id}/fetch?u={}",
                preview_server.base,
                encode(&format!("http://{remote}/media.m3u8"))
            ))
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert!(media.contains("URI=\"/preview/"));
        assert!(!media.contains("URI=\"key.bin\""));

        let key = client
            .get(format!(
                "{}/preview/{id}/fetch?u={}",
                preview_server.base,
                encode(&format!("http://{remote}/key.bin"))
            ))
            .send()
            .unwrap()
            .bytes()
            .unwrap();
        assert_eq!(key.len(), 16);
    }
}
