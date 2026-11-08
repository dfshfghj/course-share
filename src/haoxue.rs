use crate::blackboard::{Course, Replay};
use crate::http::Http;
use crate::media::{self, MediaCandidate};
use anyhow::{anyhow, Context, Result};
use md5::{Digest, Md5};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;
use url::Url;

const COURSEAPI: &str = "https://yjapise.pku.edu.cn/courseapi";
const TENANT_ID: &str = "d438cafb4c34b3b49b3a523bbef50203";
const API_VERSION: &str = "3.12.0";
const APP_ID: &str = "50";
const SIGN_KEY: &str = "8225ca5fa4d56d6fde540db6024fba39";
const SUB_ID_STEP: i64 = 4;
const HIDDEN_SCAN_STEPS: i64 = 200;
pub const LOGIN_ENTRY: &str =
    "https://passport.pku.edu.cn/auth/login?redirect=https://passport.pku.edu.cn/auth/app-login";

static URL_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"https?://[^\s"'<>\\]+"#).unwrap());

pub fn session_valid(http: &Http) -> bool {
    let Some(token) = http.cookie_value("_token2") else {
        return false;
    };
    course_api(
        http,
        "v3/course/get-use-member",
        &[("page", "1")],
        &token,
    )
    .is_ok()
}

/// The media cookie is normally set on `passport.pku.edu.cn`; media files live
/// on `resourcese.pku.edu.cn`. Normalize both tokens onto the shared PKU
/// domain and remove duplicate values so requests send one consistent token.
pub fn normalize_cookies(http: &Http) {
    for name in ["_token", "_token2"] {
        if let Some(value) = http.cookie_value(name) {
            let _ = http.set_shared_cookie(name, &value, "pku.edu.cn");
        }
    }
}

pub fn list_replays(http: &Http, course: &Course) -> Result<Vec<Replay>> {
    let token = http
        .cookie_value("_token2")
        .ok_or_else(|| anyhow!("好学未登录（缺少 _token2）"))?;
    let records = fetch_courses(http, &token, course.name.trim())?;
    let mut exact = Vec::new();
    let mut fuzzy = Vec::new();
    for record in records {
        let name = value_string(&record, &["course_name", "name"]);
        if name.trim() == course.name.trim() {
            exact.push(record);
        } else if !name.is_empty()
            && (name.contains(course.name.trim()) || course.name.trim().contains(&name))
        {
            fuzzy.push(record);
        }
    }
    let candidates = if exact.is_empty() { fuzzy } else { exact };

    let mut ranked = Vec::new();
    for item in candidates {
        let Some(course_id) = non_empty(value_string(&item, &["course_id", "id"])) else {
            continue;
        };
        let subjects = match fetch_subjects(http, &token, &course_id) {
            Ok(subjects) => subjects,
            Err(_) => continue,
        };
        let media_count = subjects
            .iter()
            .filter(|subject| !media_candidates(subject).is_empty())
            .count();
        let address = subjects
            .first()
            .map(|subject| value_string(subject, &["address"]))
            .unwrap_or_default();
        // Old placeholder records use a default classroom and never have media.
        let empty_shell = address.contains("默认校区默认教学楼") && media_count == 0;
        let rank = (
            media_count,
            match_score(course, &item),
            if empty_shell { 0 } else { 1 },
            subjects.len(),
        );
        ranked.push((rank, course_id, subjects));
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0));

    // Several Haoxue course records can share the same name. Prefer records
    // whose visible lessons already have media, then try the rest.
    for (_, course_id, subjects) in ranked {
        let replays = collect_course_replays(http, &token, &course_id, course, &subjects);
        if !replays.is_empty() {
            return Ok(replays);
        }
    }
    Ok(Vec::new())
}

fn collect_course_replays(
    http: &Http,
    token: &str,
    course_id: &str,
    course: &Course,
    subjects: &[Value],
) -> Vec<Replay> {
    let wanted = course.name.trim();
    let wanted_id = course_id.trim();
    let mut details: HashMap<String, Value> = HashMap::new();
    let mut visible_ids = Vec::new();

    // Most courses are fully covered by sub_list. Resolve media for those
    // entries first and only fall back to id scanning when needed.
    for subject in subjects {
        let Some(sub_id) = non_empty(value_string(subject, &["id", "sub_id", "subId"])) else {
            continue;
        };
        if let Ok(id) = sub_id.parse::<i64>() {
            visible_ids.push(id);
        }
        let value = if media_candidates(subject).is_empty() {
            match fetch_sub_detail(http, token, course_id, &sub_id) {
                Ok(value) => value,
                Err(_) => continue,
            }
        } else {
            subject.clone()
        };
        if detail_belongs(&value, wanted_id, wanted) {
            details.insert(sub_id, value);
        }
    }

    let default = fetch_sub_detail(http, token, course_id, "").ok();
    let default_id = default
        .as_ref()
        .and_then(|value| non_empty(value_string(value, &["sub_id", "id"])))
        .and_then(|value| value.parse::<i64>().ok());
    let anchor_covered = match (default_id, visible_ids.iter().min(), visible_ids.iter().max()) {
        (Some(id), Some(min), Some(max)) => id >= *min && id <= *max,
        (None, _, _) | (_, None, _) | (_, _, None) => true,
    };

    let replays = replays_from_details(course, &details);
    if !replays.is_empty() && anchor_covered {
        return replays;
    }

    // Either sub_list has no playable replay, or the user's last-watched
    // lesson lies outside sub_list (the list is truncated). Walk forward from
    // that anchor (step 4) until five consecutive ids miss.
    let Some(anchor) = default else {
        return replays;
    };
    if !detail_belongs(&anchor, wanted_id, wanted) {
        return replays;
    }
    let Some(anchor_id) = default_id else {
        return replays;
    };
    details.insert(anchor_id.to_string(), anchor);
    let mut misses = 0usize;
    let mut id = anchor_id + SUB_ID_STEP;
    for _ in 0..HIDDEN_SCAN_STEPS {
        match fetch_sub_detail(http, token, course_id, &id.to_string()) {
            Ok(value)
                if detail_belongs(&value, wanted_id, wanted)
                    && !value_string(&value, &["title"]).trim().is_empty() =>
            {
                misses = 0;
                details.insert(id.to_string(), value);
            }
            _ => {
                misses += 1;
                if misses >= 5 {
                    break;
                }
            }
        }
        id += SUB_ID_STEP;
    }
    replays_from_details(course, &details)
}

fn detail_belongs(detail: &Value, course_id: &str, name: &str) -> bool {
    let detail_course_id = value_string(detail, &["course_id"]);
    if !detail_course_id.is_empty() && detail_course_id.trim() != course_id.trim() {
        return false;
    }
    let detail_name = value_string(detail, &["course_name"]);
    if !detail_name.is_empty() && detail_name.trim() != name.trim() {
        return false;
    }
    true
}

fn replays_from_details(course: &Course, details: &HashMap<String, Value>) -> Vec<Replay> {
    let mut replays = Vec::new();
    let mut seen = HashSet::new();
    for (fallback_id, detail) in details {
        let Some(replay) = replay_from_detail(course, detail, fallback_id) else {
            continue;
        };
        if seen.insert(replay.replay_id.clone()) {
            replays.push(replay);
        }
    }
    replays.sort_by(|a, b| {
        let left = a.replay_id.parse::<i64>().unwrap_or(0);
        let right = b.replay_id.parse::<i64>().unwrap_or(0);
        left.cmp(&right)
    });
    replays
}

fn fetch_courses(http: &Http, token: &str, search: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for page in 1..=20usize {
        let page_text = page.to_string();
        let value = course_api(
            http,
            "v3/course/get-use-member",
            &[("page", page_text.as_str()), ("search", search)],
            token,
        )?;
        let data = value.get("data").unwrap_or(&value);
        let lists = data
            .get("lists")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let page_len = lists.len();
        let mut added = 0usize;
        for item in lists {
            let id = value_string(&item, &["course_id", "id"]);
            if !id.is_empty() && seen.insert(id) {
                out.push(item);
                added += 1;
            }
        }
        let has_next = truthy(data.get("hasNextPage"));
        let page_size = data
            .get("pageSize")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|text| text.trim().parse().ok()))
            })
            .unwrap_or(10) as usize;
        let full_page = page_len >= page_size.max(10);
        if !(has_next || full_page) || added == 0 {
            break;
        }
    }
    Ok(out)
}

fn fetch_course_detail(http: &Http, token: &str, course_id: &str) -> Result<Value> {
    let value = course_api(
        http,
        "v3/course/get-course-detail",
        &[("course_id", course_id)],
        token,
    )?;
    let mut data = value.get("data").cloned().unwrap_or(value);
    if data.get("title").is_none() && data.get("sub_list").is_none() {
        if let Some(inner) = data.get("data").cloned() {
            data = inner;
        }
    }
    Ok(data)
}

fn fetch_subjects(http: &Http, token: &str, course_id: &str) -> Result<Vec<Value>> {
    let data = fetch_course_detail(http, token, course_id)?;
    Ok(data
        .get("sub_list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

fn fetch_sub_detail(
    http: &Http,
    token: &str,
    course_id: &str,
    sub_id: &str,
) -> Result<Value> {
    let value = course_api(
        http,
        "v3/course/get-course-sub-detail",
        &[("course_id", course_id), ("sub_id", sub_id)],
        token,
    )?;
    // Responses are wrapped as {"success":..., "data": {...}}; some versions
    // add another data layer. Unwrap until we see the detail object itself.
    let mut data = value.get("data").cloned().unwrap_or(value);
    if data.get("title").is_none()
        && data.get("sub_id").is_none()
        && data.get("videoArr").is_none()
    {
        if let Some(inner) = data.get("data").cloned() {
            data = inner;
        }
    }
    Ok(data)
}

fn replay_from_detail(
    course: &Course,
    detail: &Value,
    fallback_id: &str,
) -> Option<Replay> {
    let title = value_string(detail, &["title", "sub_title", "subTitle"]);
    if title.is_empty() {
        return None;
    }
    let media = media_candidates(detail);
    if media.is_empty() {
        return None;
    }
    let sub_id = non_empty(value_string(detail, &["sub_id", "id", "subId"]))
        .unwrap_or_else(|| fallback_id.to_string());
    if sub_id.is_empty() {
        return None;
    }
    let lecturer = match non_empty(value_string(
        detail,
        &["lecturer_name", "teacher_name", "lecturer"],
    )) {
        Some(name) => name,
        None => course.teacher.clone(),
    };
    let mut guid = value_string(detail, &["resource_guid", "resourceGuid"]);
    if guid.is_empty() {
        guid = media
            .first()
            .map(|candidate| candidate.guid.clone())
            .unwrap_or_default();
    }
    let mut record_time = format_timestamp(&value_string(
        detail,
        &["class_begin", "start_at", "record_time"],
    ));
    if record_time.is_empty() {
        record_time = date_from_title(&title);
    }
    Some(Replay {
        course_id: course.id.clone(),
        course_name: course.name.clone(),
        replay_id: sub_id,
        sub_title: title,
        lecturer,
        record_time,
        page_url: String::new(),
        media,
        guid,
    })
}

fn date_from_title(title: &str) -> String {
    static DATE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(\d{4}-\d{2}-\d{2})").unwrap());
    DATE.captures(title)
        .and_then(|captured| captured.get(1))
        .map(|value| value.as_str().to_string())
        .unwrap_or_default()
}

fn media_candidates(value: &Value) -> Vec<MediaCandidate> {
    let mut urls = Vec::new();
    collect_media_urls(value, &mut urls);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for url in urls {
        let Ok(parsed) = Url::parse(&url) else {
            continue;
        };
        let identity = format!(
            "{}://{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or(""),
            parsed.path()
        );
        if !seen.insert(identity) {
            continue;
        }
        let kind = media::classify_media_url(&url);
        if kind == "invalid" || kind == "unknown" {
            continue;
        }
        candidates.push(MediaCandidate {
            guid: media::guid_from_url(&url),
            kind: kind.to_string(),
            label: "haoxue".to_string(),
            quality: media::score_vod_url(&url),
            url,
        });
    }
    candidates.sort_by(|a, b| b.quality.cmp(&a.quality).then_with(|| a.url.cmp(&b.url)));
    candidates
}

fn collect_media_urls(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let unescaped = text.replace("\\/", "/").replace("&amp;", "&");
            if let Ok(nested) = serde_json::from_str::<Value>(&unescaped) {
                collect_media_urls(&nested, out);
            }
            for found in URL_RE.find_iter(&unescaped) {
                out.push(found.as_str().to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_media_urls(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_media_urls(item, out);
            }
        }
        _ => {}
    }
}

fn format_timestamp(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return value.to_string();
    }
    let Ok(mut seconds) = value.parse::<i64>() else {
        return value.to_string();
    };
    if seconds > 10_000_000_000 {
        seconds /= 1000;
    }
    let Some(time) = chrono::DateTime::from_timestamp(seconds, 0) else {
        return value.to_string();
    };
    let Some(offset) = chrono::FixedOffset::east_opt(8 * 3600) else {
        return value.to_string();
    };
    time.with_timezone(&offset)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn match_score(course: &Course, item: &Value) -> i32 {
    let name = value_string(item, &["course_name", "name"]);
    let teacher = value_string(item, &["course_teacher", "teacher"]);
    let term = value_string(item, &["course_term", "term"]);
    let mut score = 0;
    if name.trim() == course.name.trim() {
        score += 3;
    }
    if !teacher.is_empty()
        && !course.teacher.is_empty()
        && (teacher.contains(&course.teacher) || course.teacher.contains(&teacher))
    {
        score += 2;
    }
    if !term.is_empty() && !course.term.is_empty() && term.trim() == course.term.trim() {
        score += 2;
    }
    score
}

fn course_api(
    http: &Http,
    path: &str,
    params: &[(&str, &str)],
    token: &str,
) -> Result<Value> {
    let mut url = Url::parse(&format!("{COURSEAPI}/{path}"))
        .with_context(|| format!("build courseapi URL {path}"))?;
    url.query_pairs_mut().extend_pairs(signed_params(params));
    let value = http
        .get_json(url.as_str(), &[("token", token), ("urlname", "courseapi")])
        .with_context(|| format!("好学 API {path}"))?;
    if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let err = value
            .get("err")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let message = value
            .get("errMsg")
            .or_else(|| value.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(anyhow!("好学 API 返回错误 {err}: {message}"));
    }
    Ok(value)
}

fn signed_params(params: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut map = BTreeMap::new();
    for (key, value) in params {
        if !value.is_empty() {
            map.insert((*key).to_string(), (*value).to_string());
        }
    }
    for (key, value) in [
        ("tenantid", TENANT_ID),
        ("tenantId", TENANT_ID),
        ("api_version", API_VERSION),
        ("app_id", APP_ID),
        ("client", "android"),
    ] {
        map.insert(key.to_string(), value.to_string());
    }
    let material = map
        .iter()
        .map(|(key, value)| format!("{key}{value}"))
        .collect::<String>();
    let digest = Md5::digest(format!("{material}{SIGN_KEY}").as_bytes());
    let sign = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    map.insert("sign".to_string(), sign);
    map.into_iter().collect()
}

fn value_string(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(found) = value.get(*key) {
            if let Some(text) = found.as_str() {
                if !text.is_empty() {
                    return text.to_string();
                }
            }
            if let Some(number) = found.as_u64() {
                return number.to_string();
            }
            if let Some(number) = found.as_i64() {
                return number.to_string();
            }
        }
    }
    String::new()
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        Some(Value::String(value)) => {
            matches!(value.as_str(), "1" | "true" | "TRUE" | "True")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_courseapi_params() {
        let params = signed_params(&[("page", "1")]);
        let sign = params
            .iter()
            .find(|(key, _)| key == "sign")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(sign, "9992eb7fac3d38197045218fd386ff8b");
    }

    #[test]
    fn extracts_media_from_synthetic_sub_detail() {
        let value = serde_json::json!({
            "data": {
                "videoArr": [
                    {"path": "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/01/02/0123456789abcdef0123456789abcdef/0/playlist.m3u8", "tag": "hd"},
                    {"path": "https://resourcese.pku.edu.cn/play/video/vod/newLowVideo/2026/01/02/FEDCBA9876543210FEDCBA9876543210/low.mp4", "tag": "sd"}
                ],
                "sub_content": "{\"save_playback\":{\"contents\":\"https:\\/\\/resourcese.pku.edu.cn\\/play\\/video\\/vod\\/newLowVideo\\/2026\\/01\\/02\\/FEDCBA9876543210FEDCBA9876543210\\/low.mp4\"}}"
            }
        });
        let media = media_candidates(&value);
        assert!(!media.is_empty());
        assert!(media.iter().any(|item| item.kind == "hls"));
        assert!(media.iter().any(|item| item.kind == "file"));
    }

    #[test]
    fn formats_beijing_time() {
        assert_eq!(format_timestamp("1735084800"), "2024-12-25 08:00:00");
        assert_eq!(format_timestamp("2024-12-25 08:00:00"), "2024-12-25 08:00:00");
    }

    #[test]
    fn builds_replay_from_detail_with_title_date() {
        let course = Course {
            id: "10001_1".to_string(),
            name: "测试课程".to_string(),
            teacher: "测试教师".to_string(),
            term: "2026秋".to_string(),
            url: String::new(),
        };
        let detail = serde_json::json!({
            "sub_id": "900001",
            "course_name": "测试课程",
            "title": "2026-09-07第10-12节",
            "resource_guid": "0123456789abcdef0123456789abcdef",
            "videoArr": [{
                "path": "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/09/08/0123456789abcdef0123456789abcdef/0/playlist.m3u8"
            }]
        });
        let replay = replay_from_detail(&course, &detail, "900001").unwrap();
        assert_eq!(replay.replay_id, "900001");
        assert_eq!(replay.sub_title, "2026-09-07第10-12节");
        assert_eq!(replay.record_time, "2026-09-07");
        assert_eq!(replay.guid, "0123456789abcdef0123456789abcdef");
        assert!(!replay.media.is_empty());
    }

    #[test]
    fn skips_detail_without_media() {
        let course = Course {
            id: "10001_1".to_string(),
            name: "测试课程".to_string(),
            teacher: "测试教师".to_string(),
            term: "2026秋".to_string(),
            url: String::new(),
        };
        let detail = serde_json::json!({
            "sub_id": "900002",
            "title": "2026-09-14第10-12节",
            "sub_content": {"save_playback": {"is_m3u8": "no"}}
        });
        assert!(replay_from_detail(&course, &detail, "900002").is_none());
    }
}
