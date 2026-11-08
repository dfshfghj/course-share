use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookieRecord {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub host_only: bool,
    #[serde(default)]
    pub expires: Option<i64>,
}

fn default_path() -> String {
    "/".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CookieJar {
    #[serde(default)]
    pub records: Vec<CookieRecord>,
}

impl CookieJar {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<CookieJar>(&text).ok())
            .map(|mut jar| {
                jar.prune();
                jar
            })
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let data = serde_json::to_vec_pretty(self)?;
        fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.records.len()
    }

    pub fn value(&self, name: &str) -> Option<String> {
        self.records
            .iter()
            .rev()
            .find(|cookie| cookie.name == name && !cookie.value.is_empty())
            .map(|cookie| cookie.value.clone())
    }

    /// Replace every cookie with this name by one shared-domain record. Used for
    /// Haoxue tokens that must also be sent to `resourcese.pku.edu.cn`.
    pub fn set_shared(&mut self, name: &str, value: &str, domain: &str) {
        self.records.retain(|cookie| cookie.name != name);
        self.records.push(CookieRecord {
            name: name.to_string(),
            value: value.to_string(),
            domain: domain.trim_start_matches('.').to_ascii_lowercase(),
            path: "/".to_string(),
            secure: true,
            host_only: false,
            expires: None,
        });
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn cookie_header(&mut self, url: &Url) -> Option<String> {
        self.prune();
        let host = url.host_str()?.to_ascii_lowercase();
        let path = if url.path().is_empty() { "/" } else { url.path() };
        let secure_ok = url.scheme() == "https";
        let mut parts = Vec::new();
        for cookie in &self.records {
            if cookie.secure && !secure_ok {
                continue;
            }
            if !domain_matches(&cookie.domain, cookie.host_only, &host) {
                continue;
            }
            if !path_matches(&cookie.path, path) {
                continue;
            }
            parts.push(format!("{}={}", cookie.name, cookie.value));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }

    pub fn capture_set_cookie(&mut self, url: &Url, raw: &str) {
        if let Some(cookie) = parse_set_cookie(raw, url) {
            if cookie.expires.map(|ts| ts <= now()).unwrap_or(false) {
                self.records.retain(|c| !same_cookie(c, &cookie));
                return;
            }
            self.records.retain(|c| !same_cookie(c, &cookie));
            self.records.push(cookie);
        }
    }

    /// Import a `name=value; name2=value2` Cookie header for every host under
    /// the given domain suffix (for example `pku.edu.cn`).
    pub fn import_header(&mut self, header: &str, domain: &str) {
        let header = header.trim().trim_start_matches("Cookie:").trim();
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        for part in header.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let record = CookieRecord {
                name: name.to_string(),
                value: value.trim().to_string(),
                domain: domain.clone(),
                path: "/".to_string(),
                secure: true,
                host_only: false,
                expires: None,
            };
            self.records.retain(|c| !same_cookie(c, &record));
            self.records.push(record);
        }
    }

    fn prune(&mut self) {
        let now = now();
        self.records
            .retain(|c| c.expires.map(|ts| ts > now).unwrap_or(true));
    }
}

fn same_cookie(a: &CookieRecord, b: &CookieRecord) -> bool {
    a.name == b.name
        && a.domain.eq_ignore_ascii_case(&b.domain)
        && a.path == b.path
        && a.host_only == b.host_only
}

fn domain_matches(domain: &str, host_only: bool, host: &str) -> bool {
    if host_only {
        host.eq_ignore_ascii_case(domain)
    } else {
        host.eq_ignore_ascii_case(domain)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", domain.to_ascii_lowercase()))
    }
}

fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    cookie_path == "/"
        || request_path == cookie_path
        || request_path.starts_with(&format!("{}/", cookie_path.trim_end_matches('/')))
}

fn parse_set_cookie(raw: &str, url: &Url) -> Option<CookieRecord> {
    let mut parts = raw.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut cookie = CookieRecord {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain: host,
        path: "/".to_string(),
        secure: false,
        host_only: true,
        expires: None,
    };
    for attr in parts {
        let (key, value) = attr.split_once('=').unwrap_or((attr, ""));
        match key.trim().to_ascii_lowercase().as_str() {
            "domain" => {
                let domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
                if !domain.is_empty() {
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
            }
            "path" => {
                let path = value.trim();
                if path.starts_with('/') {
                    cookie.path = path.to_string();
                }
            }
            "secure" => cookie.secure = true,
            "max-age" => {
                if let Ok(seconds) = value.trim().parse::<i64>() {
                    cookie.expires = Some(now() + seconds);
                }
            }
            "expires" => {
                if let Some(ts) = parse_http_date(value.trim()) {
                    cookie.expires = Some(ts);
                }
            }
            _ => {}
        }
    }
    Some(cookie)
}

fn parse_http_date(value: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc2822(value) {
        return Some(dt.timestamp());
    }
    for format in [
        "%a, %d %b %Y %H:%M:%S GMT",
        "%a, %d-%b-%Y %H:%M:%S GMT",
        "%A, %d-%b-%y %H:%M:%S GMT",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, format) {
            return Some(dt.and_utc().timestamp());
        }
    }
    None
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_matches_domain_and_path() {
        let url = Url::parse("https://course.pku.edu.cn/webapps/x").unwrap();
        let mut jar = CookieJar::default();
        jar.capture_set_cookie(&url, "JSESSIONID=abc; Path=/; Domain=.pku.edu.cn; Secure");
        let media = Url::parse("https://resourcese.pku.edu.cn/play/x.m3u8").unwrap();
        assert_eq!(jar.cookie_header(&media).unwrap(), "JSESSIONID=abc");
        let http = Url::parse("http://resourcese.pku.edu.cn/play/x.m3u8").unwrap();
        assert!(jar.cookie_header(&http).is_none());
        let other = Url::parse("https://example.com/").unwrap();
        assert!(jar.cookie_header(&other).is_none());
    }

    #[test]
    fn imports_cookie_header_for_domain_suffix() {
        let mut jar = CookieJar::default();
        jar.import_header("_token=media; _token2=api", "pku.edu.cn");
        let url = Url::parse("https://onlineroomse.pku.edu.cn/player").unwrap();
        let header = jar.cookie_header(&url).unwrap();
        assert!(header.contains("_token=media"));
        assert!(header.contains("_token2=api"));
    }

    #[test]
    fn expired_cookie_is_removed() {
        let url = Url::parse("https://course.pku.edu.cn/").unwrap();
        let mut jar = CookieJar::default();
        jar.capture_set_cookie(&url, "old=1; Max-Age=0; Path=/");
        assert!(jar.cookie_header(&url).is_none());
    }
}
