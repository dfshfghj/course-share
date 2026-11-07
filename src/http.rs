use crate::cookies::CookieJar;
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE, COOKIE, USER_AGENT};
use reqwest::{Method, StatusCode};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct Http {
    client: Client,
    download_client: Client,
    jar: Arc<Mutex<CookieJar>>,
    cookie_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub final_url: Url,
    pub body: String,
}

impl Fetched {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl Http {
    pub fn new(cookie_path: impl Into<PathBuf>) -> Result<Self> {
        let cookie_path = cookie_path.into();
        let jar = CookieJar::load(&cookie_path);
        let base = || {
            Client::builder()
                .user_agent(DEFAULT_UA)
                .timeout(Duration::from_secs(90))
                .connect_timeout(Duration::from_secs(15))
                .gzip(true)
                .brotli(true)
                .deflate(true)
                // PKU hosts and the share backend are intranet endpoints; a
                // desktop proxy (often a localhost Clash/SOCKS listener) can
                // reset these connections even when it works for the internet.
                .no_proxy()
        };
        let client = base()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build HTTP client")?;
        let download_client = base()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .context("build media HTTP client")?;
        Ok(Self {
            client,
            download_client,
            jar: Arc::new(Mutex::new(jar)),
            cookie_path,
        })
    }

    pub fn cookie_count(&self) -> usize {
        self.jar.lock().map(|j| j.count()).unwrap_or(0)
    }

    pub fn import_cookie_header(&self, header: &str, domain: &str) -> Result<()> {
        let mut jar = self.jar.lock().map_err(|_| anyhow!("cookie store poisoned"))?;
        jar.import_header(header, domain);
        jar.save(&self.cookie_path)
    }

    pub fn clear_cookies(&self) -> Result<()> {
        let mut jar = self.jar.lock().map_err(|_| anyhow!("cookie store poisoned"))?;
        jar.clear();
        jar.save(&self.cookie_path)
    }

    pub fn save_cookies(&self) -> Result<()> {
        let jar = self.jar.lock().map_err(|_| anyhow!("cookie store poisoned"))?;
        jar.save(&self.cookie_path)
    }

    pub fn request(
        &self,
        method: Method,
        url: &str,
        form: Option<Vec<(String, String)>>,
        extra_headers: &[(&str, &str)],
    ) -> Result<Response> {
        self.request_with_client(&self.client, method, url, form, extra_headers)
    }

    fn request_with_client(
        &self,
        client: &Client,
        method: Method,
        url: &str,
        form: Option<Vec<(String, String)>>,
        extra_headers: &[(&str, &str)],
    ) -> Result<Response> {
        let parsed = Url::parse(url).with_context(|| format!("invalid URL {url}"))?;
        let mut request = client
            .request(method.clone(), parsed.clone())
            .header(USER_AGENT, DEFAULT_UA)
            .header(ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7");
        if let Some(form) = form {
            request = request.form(&form);
        }
        for (key, value) in extra_headers {
            if let Ok(value) = HeaderValue::from_str(value) {
                request = request.header(*key, value);
            }
        }
        if let Some(cookie) = self.jar.lock().ok().and_then(|mut j| j.cookie_header(&parsed)) {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                request = request.header(COOKIE, value);
            }
        }
        let response = request.send().with_context(|| {
            format!("request {} {}", method.as_str(), redact_url(&parsed))
        })?;
        self.capture_cookies(&parsed, response.headers());
        Ok(response)
    }

    pub fn get(&self, url: &str) -> Result<Response> {
        self.request(Method::GET, url, None, &[])
    }

    pub fn get_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        self.request(Method::GET, url, None, headers)
    }

    pub fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Response> {
        let form: Vec<(String, String)> = form
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        self.request(Method::POST, url, Some(form), &[])
    }

    /// Follow redirects with GET and return a small, owned response snapshot.
    pub fn get_follow(&self, url: &str, max_hops: usize) -> Result<Fetched> {
        self.get_follow_with_headers(url, &[], max_hops)
    }

    pub fn get_follow_with_headers(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_hops: usize,
    ) -> Result<Fetched> {
        let mut current = Url::parse(url).with_context(|| format!("invalid URL {url}"))?;
        upgrade_pku_https(&mut current);
        let mut visited = std::collections::HashSet::new();
        for _ in 0..max_hops {
            if !visited.insert(current.to_string()) {
                return Err(anyhow!("redirect loop at {}", redact_url(&current)));
            }
            let response = self
                .request(Method::GET, current.as_str(), None, headers)
                .with_context(|| format!("GET {}", redact_url(&current)))?;
            let status = response.status();
            if status.is_redirection() {
                if let Some(location) = response.headers().get(reqwest::header::LOCATION) {
                    if let Ok(location) = location.to_str() {
                        current = current
                            .join(location)
                            .with_context(|| format!("bad redirect {location}"))?;
                        upgrade_pku_https(&mut current);
                        continue;
                    }
                }
                return snapshot(response, current);
            }
            if status == StatusCode::OK {
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let body = response.text().context("read response body")?;
                if content_type.to_ascii_lowercase().contains("html")
                    && body.len() < 64 * 1024
                {
                    if let Some(next) = html_redirect(&body) {
                        if let Ok(next_url) = current.join(&next) {
                            current = next_url;
                            continue;
                        }
                    }
                }
                return Ok(Fetched {
                    status: status.as_u16(),
                    final_url: current,
                    body,
                });
            }
            return snapshot(response, current);
        }
        Err(anyhow!("too many redirects starting at {}", redact_url(&current)))
    }

    pub fn get_text_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Result<String> {
        let response = self.get_with_headers(url, headers)?;
        ensure_success(&response, url)?;
        response.text().context("read response body")
    }

    pub fn get_json(&self, url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value> {
        let text = self.get_text_with_headers(url, headers)?;
        serde_json::from_str(&text).context("parse JSON response")
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.get(url)?;
        ensure_success(&response, url)?;
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .context("read binary response")
    }

    /// GET a media resource following redirects (segments, keys, init maps,
    /// direct MP4). Login/API calls still use the non-redirecting client so
    /// every Set-Cookie header can be captured.
    pub fn get_response_follow(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<Response> {
        self.request_with_client(&self.download_client, Method::GET, url, None, headers)
    }

    pub fn get_bytes_follow(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.get_response_follow(url, &[])?;
        ensure_success(&response, url)?;
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .context("read binary response")
    }

    fn capture_cookies(&self, url: &Url, headers: &HeaderMap) {
        let values: Vec<String> = headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(ToOwned::to_owned))
            .collect();
        if values.is_empty() {
            return;
        }
        if let Ok(mut jar) = self.jar.lock() {
            for value in values {
                jar.capture_set_cookie(url, &value);
            }
            let _ = jar.save(&self.cookie_path);
        }
    }
}

fn snapshot(response: Response, url: Url) -> Result<Fetched> {
    let status = response.status();
    let body = response.text().unwrap_or_default();
    Ok(Fetched {
        status: status.as_u16(),
        final_url: url,
        body,
    })
}

fn upgrade_pku_https(url: &mut Url) {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if url.scheme() == "http" && host.ends_with("pku.edu.cn") {
        let _ = url.set_scheme("https");
        if url.port() == Some(80) {
            let _ = url.set_port(None);
        }
    }
}

fn ensure_success(response: &Response, url: &str) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let clean = Url::parse(url)
        .map(|u| redact_url(&u))
        .unwrap_or_else(|_| url.to_string());
    Err(anyhow!("HTTP {} for {}", status.as_u16(), clean))
}

pub fn redact_url(url: &Url) -> String {
    let mut clean = url.clone();
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

fn html_redirect(body: &str) -> Option<String> {
    static META: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?i)http-equiv=["']refresh["'][^>]*content=["'][^"']*url=([^"'>\s]+)"#)
            .unwrap()
    });
    static JS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"window\.location(?:\.href)?\s*=\s*["']([^"']+)["']"#).unwrap()
    });
    META.captures(body)
        .and_then(|c| c.get(1).map(|m| html_unescape(m.as_str())))
        .or_else(|| {
            JS.captures(body)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        })
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn live_pku_http_redirect_upgrades_to_https() {
        let path = std::env::temp_dir().join(format!(
            "course-share-https-upgrade-{}.json",
            std::process::id()
        ));
        let http = Http::new(&path).unwrap();
        let fetched = http
            .get_follow("http://course.pku.edu.cn/", 10)
            .expect("PKU HTTPS upgrade");
        assert_eq!(fetched.final_url.scheme(), "https");
        assert_eq!(fetched.status, 200);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore]
    fn live_course_home_handshake_uses_system_tls() {
        let path = std::env::temp_dir().join(format!(
            "course-share-tls-test-{}.json",
            std::process::id()
        ));
        let http = Http::new(&path).unwrap();
        let fetched = http
            .get_follow("https://course.pku.edu.cn/", 5)
            .expect("course.pku.edu.cn handshake");
        assert_eq!(fetched.status, 200);
        let _ = std::fs::remove_file(path);
    }
}
