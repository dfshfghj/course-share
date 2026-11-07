use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaCandidate {
    pub url: String,
    pub kind: String,
    pub label: String,
    pub quality: i32,
    pub guid: String,
}

impl MediaCandidate {
    pub fn identity(&self) -> String {
        Url::parse(&self.url)
            .map(|u| format!("{}://{}{}", u.scheme(), u.host_str().unwrap_or(""), u.path()))
            .unwrap_or_else(|_| self.url.clone())
    }

    pub fn is_hls(&self) -> bool {
        self.kind == "hls"
    }
}

pub fn classify_media_url(url: &str) -> &'static str {
    if url.contains("downloadVideo.action") {
        return "invalid";
    }
    let path = Url::parse(url)
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_else(|_| url.to_ascii_lowercase());
    if path.ends_with(".m3u8") || path.contains(".m3u8") {
        return "hls";
    }
    for suffix in [".mp4", ".m4v", ".mov", ".mkv", ".flv", ".webm", ".ts"] {
        if path.ends_with(suffix) {
            return "file";
        }
    }
    "unknown"
}

pub fn score_vod_url(url: &str) -> i32 {
    let lower = url.to_ascii_lowercase();
    for (keyword, score) in [
        ("newhighvideo", 50),
        ("4000k", 48),
        ("_1080", 46),
        ("2000k", 34),
        ("1280_720", 32),
        ("newmidvideo", 30),
        ("1000k", 24),
        ("854_480", 22),
        ("_480", 20),
        ("newlowvideo", 12),
        ("500k", 10),
        ("640_360", 8),
        ("_360", 6),
    ] {
        if lower.contains(keyword) {
            return score;
        }
    }
    if lower.contains(".m3u8") {
        26
    } else {
        5
    }
}

pub fn guid_from_url(url: &str) -> String {
    static GUID: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"harpocrates/[0-9]+/[0-9]+/[0-9]+/([A-Za-z0-9_-]+)/").unwrap()
    });
    if let Some(captured) = GUID
        .captures(url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    {
        return captured;
    }
    let Ok(parsed) = Url::parse(url) else {
        return String::new();
    };
    let parts: Vec<&str> = parsed.path().split('/').filter(|part| !part.is_empty()).collect();
    parts
        .iter()
        .rev()
        .skip(1)
        .find(|part| {
            (16..=64).contains(&part.len())
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
                && !part.chars().all(|ch| ch.is_ascii_digit())
        })
        .map(|part| part.to_string())
        .unwrap_or_default()
}

pub fn extract_media_candidates(sub_content: &str) -> Vec<MediaCandidate> {
    let mut candidates = Vec::new();
    let parsed = serde_json::from_str::<serde_json::Value>(sub_content).ok();

    if let Some(value) = &parsed {
        if let Some(saved) = value.get("save_playback") {
            if let Some(url) = saved.get("contents").and_then(|v| v.as_str()) {
                add_candidate(&mut candidates, url, "saved playback");
            }
        }
        for (key, label) in [
            ("newHighVideo", "high"),
            ("newMidVideo", "mid"),
            ("newLowVideo", "low"),
            ("highVideo", "high"),
            ("midVideo", "mid"),
            ("lowVideo", "low"),
        ] {
            if let Some(url) = value.get(key).and_then(|v| v.as_str()) {
                add_candidate(&mut candidates, url, label);
            }
        }
        if let Some(playback) = value.get("playback").and_then(|v| v.get("url")).and_then(|v| v.as_str()) {
            add_candidate(&mut candidates, playback, "playback");
        }
        if let Some(files) = value.get("file_list").and_then(|v| v.as_array()) {
            for item in files {
                if let Some(url) = item.get("file_name").and_then(|v| v.as_str()) {
                    add_candidate(&mut candidates, url, "file");
                }
            }
        }
        if let Some(firm) = value.get("firm_source").and_then(|v| v.get("contents")).and_then(|v| v.as_str()) {
            add_candidate(&mut candidates, firm, "cloud source");
        }
    }

    let mut raw = String::new();
    collect_strings(parsed.as_ref(), &mut raw);
    raw.push_str(&sub_content.replace("\\/", "/"));
    static URL_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"https?://resourcese\.pku\.edu\.cn/[^"'\s<>\\)]+"#).unwrap()
    });
    for found in URL_RE.find_iter(&raw) {
        add_candidate(&mut candidates, found.as_str(), "resource");
    }

    dedupe_and_sort(candidates)
}

pub fn extract_media_from_html(html: &str) -> Vec<MediaCandidate> {
    let mut candidates = Vec::new();
    static URL_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"https?://resourcese\.pku\.edu\.cn/[^"'\s<>\\)]+"#).unwrap()
    });
    let html = html.replace("\\/", "/").replace("&amp;", "&");
    for found in URL_RE.find_iter(&html) {
        add_candidate(&mut candidates, found.as_str(), "page");
    }
    dedupe_and_sort(candidates)
}

fn add_candidate(out: &mut Vec<MediaCandidate>, url: &str, label: &str) {
    let url = url
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | ',' | ';' | ')' | ']' | '}'))
        .replace("&amp;", "&");
    if !url.starts_with("http") || !url.contains("resourcese.pku.edu.cn") {
        return;
    }
    let kind = classify_media_url(&url);
    if kind == "invalid" || kind == "unknown" {
        return;
    }
    let quality = score_vod_url(&url);
    out.push(MediaCandidate {
        guid: guid_from_url(&url),
        url,
        kind: kind.to_string(),
        label: label.to_string(),
        quality,
    });
}

fn collect_strings(value: Option<&serde_json::Value>, out: &mut String) {
    let Some(value) = value else {
        return;
    };
    match value {
        serde_json::Value::String(text) => {
            out.push_str(text);
            out.push('\n');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(Some(item), out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_strings(Some(item), out);
            }
        }
        _ => {}
    }
}

pub fn dedupe_candidates(candidates: Vec<MediaCandidate>) -> Vec<MediaCandidate> {
    dedupe_and_sort(candidates)
}

fn dedupe_and_sort(mut candidates: Vec<MediaCandidate>) -> Vec<MediaCandidate> {
    let fallback_guid = candidates
        .iter()
        .find(|candidate| !candidate.guid.is_empty())
        .map(|candidate| candidate.guid.clone())
        .unwrap_or_default();
    for candidate in &mut candidates {
        if candidate.guid.is_empty() && !fallback_guid.is_empty() {
            candidate.guid = fallback_guid.clone();
        }
    }
    let mut by_identity: HashMap<String, MediaCandidate> = HashMap::new();
    for candidate in candidates {
        let key = candidate.identity();
        match by_identity.get(&key) {
            Some(existing) if existing.quality >= candidate.quality => {}
            _ => {
                by_identity.insert(key, candidate);
            }
        }
    }
    let mut out: Vec<_> = by_identity.into_values().collect();
    out.sort_by(|a, b| {
        b.quality
            .cmp(&a.quality)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.url.cmp(&b.url))
    });
    out
}

pub fn sanitize_filename(value: &str) -> String {
    let mut clean = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control() {
            clean.push('_');
        } else {
            clean.push(ch);
        }
    }
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = clean.chars().take(180).collect::<String>();
    let clean = clean.trim_end_matches([' ', '.']);
    let clean = if clean.is_empty() { "replay" } else { clean };
    if is_reserved_windows_name(clean) {
        format!("_{clean}")
    } else {
        clean.to_string()
    }
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim();
    let stem = stem.to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit()
        && stem.as_bytes()[3] != b'0')
}

pub fn replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        match std::fs::remove_file(to) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    std::fs::rename(from, to)
}

pub fn output_path(dir: &std::path::Path, course: &str, sub_title: &str, lecturer: &str) -> PathBuf {
    let mut parts = vec![course.to_string()];
    if !sub_title.is_empty() {
        parts.push(sub_title.to_string());
    }
    if !lecturer.is_empty() {
        parts.push(lecturer.to_string());
    }
    let parts: Vec<String> = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect();
    let name = sanitize_filename(&parts.join(" - "));
    dir.join(format!("{name}.mp4"))
}

pub fn find_downloaded(dir: &std::path::Path, sub_title: &str, lecturer: &str) -> Option<PathBuf> {
    let path = output_path(dir, "", sub_title, lecturer);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let legacy_stem = format!("- {stem}");
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".part.mp4") || name.contains(".segments") {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["mp4", "mkv", "m4v", "mov", "flv", "webm", "ts"].contains(&extension.as_str()) {
            continue;
        }
        let file_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if (file_stem == stem || file_stem == legacy_stem)
            && path.metadata().map(|meta| meta.len() > 0).unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

pub fn find_in_progress(dir: &std::path::Path, sub_title: &str, lecturer: &str) -> bool {
    let path = output_path(dir, "", sub_title, lecturer);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let legacy_stem = format!("- {stem}");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() {
            if name.ends_with(".segments") && (name.contains(&stem) || name.contains(&legacy_stem)) {
                return true;
            }
        } else if name.ends_with(".part.mp4")
            && (name.contains(&stem) || name.contains(&legacy_stem))
        {
            return true;
        }
    }
    false
}

pub fn format_bytes(value: u64) -> String {
    let mut value = value as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if value < 1024.0 || unit == "TiB" {
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    format!("{value:.1} TiB")
}

pub fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/0123456789abcdef0123456789abcdef/0/playlist.m3u8";

    #[test]
    fn extracts_guid_from_harpocrates_path() {
        assert_eq!(
            guid_from_url(URL),
            "0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn extracts_guid_from_legacy_mp4_path() {
        let url = "http://resourcese.pku.edu.cn/play/video/vod/newLowVideo/2024/01/02/FEDCBA9876543210FEDCBA9876543210/h264_500k_mp4_640_360.mp4";
        assert_eq!(guid_from_url(url), "FEDCBA9876543210FEDCBA9876543210");
    }

    #[test]
    fn extracts_hls_and_mp4_but_skips_download_video() {
        let sub_content = r#"{
            "save_playback": {"is_m3u8": "yes", "contents": "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/0123456789abcdef0123456789abcdef/0/playlist.m3u8"},
            "newHighVideo": "https://resourcese.pku.edu.cn/play/video/vod/2026/11/06/newHighVideo.mp4",
            "newLowVideo": "https://resourcese.pku.edu.cn/play/video/vod/2026/11/06/newLowVideo.mp4",
            "downloadVideo": "https://course.pku.edu.cn/webapps/bb-streammedia-hqy-BBLEARN/downloadVideo.action?resourceId=0123456789abcdef0123456789abcdef"
        }"#;
        let candidates = extract_media_candidates(sub_content);
        assert!(candidates.iter().any(|candidate| candidate.kind == "hls"));
        assert!(candidates.iter().any(|candidate| candidate.label == "high"));
        assert!(candidates
            .iter()
            .all(|candidate| !candidate.url.contains("downloadVideo")));
        assert_eq!(candidates[0].label, "high");
        assert_eq!(candidates[0].guid, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn deduplicates_query_variants() {
        let sub_content = r#"{
            "save_playback": {"contents": "https://resourcese.pku.edu.cn/a/playlist.m3u8?sig=1"},
            "playback": {"url": "https://resourcese.pku.edu.cn/a/playlist.m3u8?sig=2"}
        }"#;
        let candidates = extract_media_candidates(sub_content);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn extracts_media_from_player_html() {
        let html = r#"<script>var url = "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/abc123/0/playlist.m3u8";</script>"#;
        let candidates = extract_media_from_html(html);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].kind, "hls");
        assert_eq!(candidates[0].guid, "abc123");
    }

    #[test]
    fn output_path_skips_empty_course_segment() {
        let path = output_path(std::path::Path::new("/tmp"), "", "第7-9节", "测试教师");
        assert_eq!(path.file_name().unwrap(), "第7-9节 - 测试教师.mp4");
    }

    #[test]
    fn sanitizes_windows_filename() {
        assert_eq!(sanitize_filename("a/b:c*?.mp4"), "a_b_c__.mp4");
        assert_eq!(sanitize_filename("   "), "replay");
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("com1.txt"), "_com1.txt");
        assert_eq!(sanitize_filename("trailing. "), "trailing");
    }
}
