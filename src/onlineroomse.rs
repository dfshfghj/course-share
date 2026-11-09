use crate::blackboard::{Course, Replay, RoomRef};
use crate::http::Http;
use crate::media;
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use regex::Regex;
use serde_json::Value;
use url::Url;

pub const API_URL: &str = "https://onlineroomse.pku.edu.cn/live/get-sub-info-by-auth-data";
pub const PLAYER_API_URL: &str =
    "https://onlineroomse.pku.edu.cn/courseapi/v2/schedule/get-sub-info-by-auth-data";
pub const PLAYER_URL: &str = "https://onlineroomse.pku.edu.cn/player";
pub const YJLOGINSE_URL: &str = "https://yjloginse.pku.edu.cn/";
pub const YJLOGINSE_TENANT_CODE: &str = "226";

pub fn token_from_play_url(play_url: &str) -> Option<String> {
    if let Ok(url) = Url::parse(play_url) {
        for (key, value) in url.query_pairs() {
            if key == "token" {
                return Some(value.to_string());
            }
        }
    }
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r#"[?&]token=([^&\s'"]+)"#).unwrap());
    RE.captures(play_url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

pub fn decode_jwt(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| {
            let mut padded = payload.to_string();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            base64::engine::general_purpose::URL_SAFE.decode(padded)
        })
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

#[allow(dead_code)]
pub fn fetch_list(http: &Http, play_url: &str) -> Result<Value> {
    let token = token_from_play_url(play_url)
        .ok_or_else(|| anyhow!("playVideo URL 中没有 token"))?;
    let payload = decode_jwt(&token).ok_or_else(|| anyhow!("无法解析 playVideo JWT"))?;
    let course_id = value_string(&payload, &["hqyCourseId", "courseId", "course_id"]);
    if course_id.is_empty() {
        return Err(anyhow!("JWT 中没有 hqyCourseId"));
    }

    let referer = format!("{PLAYER_URL}?course_id={course_id}&token={token}");
    let headers = api_headers(&referer, Some(&token));
    let header_refs = as_header_refs(&headers);

    match http.get_json(API_URL, &header_refs) {
        Ok(value) => {
            if let Some(list) = extract_list(&value) {
                if !list.is_empty() {
                    return Ok(value);
                }
            }
        }
        Err(_) => {}
    }

    // Some sessions accept cookie-only auth after the player has been opened.
    let no_auth_headers = api_headers(&referer, None);
    let no_auth_refs = as_header_refs(&no_auth_headers);
    if let Ok(value) = http.get_json(API_URL, &no_auth_refs) {
        if let Some(list) = extract_list(&value) {
            if !list.is_empty() {
                return Ok(value);
            }
        }
    }

    // Last resort: establish an onlineroomse cookie via yjloginse SSO and retry.
    establish_room_cookie(http, &course_id, &token);
    let value = http
        .get_json(API_URL, &header_refs)
        .or_else(|_| http.get_json(API_URL, &no_auth_refs))
        .context("调用 onlineroomse 回放列表接口失败")?;
    if extract_list(&value).map(|list| list.is_empty()).unwrap_or(true) {
        return Err(anyhow!(
            "onlineroomse 返回了空回放列表；可能需要重新登录教学网或在浏览器中打开一次播放器"
        ));
    }
    Ok(value)
}

pub fn fetch_list_for_room(http: &Http, room: &RoomRef) -> Result<Value> {
    let mut token = String::new();
    if !room.launch_url.is_empty() {
        if let Ok(page) = http.get_follow(&room.launch_url, 15) {
            token = token_from_play_url(page.final_url.as_str()).unwrap_or_default();
            if token.is_empty() {
                token = token_from_final_page(&page.body).unwrap_or_default();
            }
        }
    }

    let mut referer = format!("{PLAYER_URL}?course_id={}", room.course_id);
    if !room.sub_id.is_empty() {
        referer.push_str(&format!("&sub_id={}", room.sub_id));
    }
    if !token.is_empty() {
        referer.push_str(&format!("&token={token}"));
    }
    let headers = api_headers(&referer, if token.is_empty() { None } else { Some(&token) });
    let header_refs = as_header_refs(&headers);
    if let Ok(value) = http.get_json(API_URL, &header_refs) {
        if extract_list(&value).map(|list| !list.is_empty()).unwrap_or(false) {
            return Ok(value);
        }
    }
    let no_auth_headers = api_headers(&referer, None);
    let no_auth_refs = as_header_refs(&no_auth_headers);
    if let Ok(value) = http.get_json(API_URL, &no_auth_refs) {
        if extract_list(&value).map(|list| !list.is_empty()).unwrap_or(false) {
            return Ok(value);
        }
    }
    Err(anyhow!(
        "无法从 onlineroomse 获取回放列表（course_id={}）。\
         如果页面只有签名 iframe，请先在浏览器中打开一次播放器，或使用 streammedia 列表页。",
        room.course_id
    ))
}

fn token_from_final_page(body: &str) -> Option<String> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"(?i)["']?token["']?\s*[:=]\s*["']([A-Za-z0-9._\-]+)["']"#).unwrap()
    });
    RE.captures(body)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn api_headers(referer: &str, token: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".to_string(), "application/json, text/plain, */*".to_string()),
        ("Accept-Language".to_string(), "zh-CN,zh;q=0.9".to_string()),
        ("Origin".to_string(), "https://onlineroomse.pku.edu.cn".to_string()),
        ("Referer".to_string(), referer.to_string()),
        ("X-Requested-With".to_string(), "XMLHttpRequest".to_string()),
    ];
    if let Some(token) = token {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

#[allow(dead_code)]
fn establish_room_cookie(http: &Http, course_id: &str, token: &str) {
    let forward = format!("{PLAYER_URL}?course_id={course_id}&token={token}");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("tenant_code", YJLOGINSE_TENANT_CODE)
        .append_pair("forward", &forward)
        .finish();
    let url = format!("{YJLOGINSE_URL}?{query}");
    let _ = http.get_follow(&url, 15);
    let _ = http.save_cookies();
}

pub fn extract_list(value: &Value) -> Option<Vec<Value>> {
    if let Some(list) = value.get("list").and_then(|v| v.as_array()) {
        return Some(list.clone());
    }
    if let Some(list) = value
        .get("data")
        .and_then(|data| data.get("list"))
        .and_then(|v| v.as_array())
    {
        return Some(list.clone());
    }
    if let Some(list) = value.as_array() {
        return Some(list.clone());
    }
    None
}

/// Resolve a playVideo page through the real player flow:
/// playVideo -> signed casapi iframe -> onlineroomse player (auth_data)
/// -> courseapi schedule detail -> sub_content.save_playback.
pub fn resolve_via_player(http: &Http, replay: &mut Replay) -> Result<()> {
    if !replay.media.is_empty() {
        return Ok(());
    }
    let page_url = replay.page_url.trim().to_string();
    if page_url.is_empty() {
        return Err(anyhow!("回放缺少播放页 URL"));
    }
    let page = http
        .get_follow(&page_url, 15)
        .with_context(|| format!("打开播放页 {}", replay.sub_title))?;
    if !page.is_success() {
        return Err(anyhow!("播放页返回 HTTP {}", page.status));
    }
    let launch = match extract_casapi_url(&page.body) {
        Some(url) => page
            .final_url
            .join(&url)
            .or_else(|_| Url::parse(&url))
            .unwrap_or_else(|_| page.final_url.clone()),
        None => {
            if page
                .final_url
                .host_str()
                .map(|host| host.contains("onlineroomse.pku.edu.cn"))
                .unwrap_or(false)
            {
                page.final_url.clone()
            } else {
                return Err(anyhow!("播放页没有 casapi / onlineroomse 播放器入口"));
            }
        }
    };
    let player = http
        .get_follow(launch.as_str(), 20)
        .context("跟随 casapi 到 onlineroomse 播放器")?;
    if !player.is_success() {
        return Err(anyhow!("onlineroomse 播放器返回 HTTP {}", player.status));
    }
    let query: std::collections::HashMap<String, String> = player
        .final_url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let course_id = query
        .get("course_id")
        .cloned()
        .filter(|value| !value.is_empty())
        .or_else(|| course_id_from_play_url(&page_url))
        .ok_or_else(|| anyhow!("播放器 URL 没有 course_id"))?;
    let sub_id = query
        .get("sub_id")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| replay.replay_id.clone());
    let app_id = query
        .get("app_id")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "blackboard".to_string());
    let auth_data = query
        .get("auth_data")
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("播放器 URL 缺少 auth_data"))?;

    let mut api = Url::parse(PLAYER_API_URL).context("build player API URL")?;
    api.query_pairs_mut()
        .append_pair("all", "1")
        .append_pair("course_id", &course_id)
        .append_pair("sub_id", &sub_id)
        .append_pair("with_sub_data", "1")
        .append_pair("with_room_data", "1")
        .append_pair("show_all", "1")
        .append_pair("show_delete", "2")
        .append_pair("app_id", &app_id)
        .append_pair("auth_data", &auth_data);
    let referer = player.final_url.to_string();
    let headers = [
        ("Referer", referer.as_str()),
        ("Origin", "https://onlineroomse.pku.edu.cn"),
        ("X-Requested-With", "XMLHttpRequest"),
        ("Accept", "application/json, text/plain, */*"),
    ];
    let value = http
        .get_json(api.as_str(), &headers)
        .context("调用播放器回放详情 API")?;
    let code = value
        .get("code")
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|n| n as i64)))
        .unwrap_or(-1);
    if code != 0 {
        return Err(anyhow!(
            "播放器 API 返回 code {code}: {}",
            value.get("msg").and_then(|v| v.as_str()).unwrap_or("")
        ));
    }
    let item = value
        .get("list")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .or_else(|| {
            value
                .get("data")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
        })
        .ok_or_else(|| anyhow!("播放器 API 没有返回回放条目"))?;
    let sub_content = item
        .get("sub_content")
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let mut candidates = media::extract_media_candidates(&sub_content);
    candidates.extend(media::extract_media_candidates(&item.to_string()));
    let candidates = media::dedupe_candidates(candidates);
    if candidates.is_empty() {
        return Err(anyhow!(
            "回放没有 resourcese 媒体地址（sub_content={}）",
            body_snippet(&sub_content)
        ));
    }
    replay.media = candidates;
    let resource_guid = value_string(item, &["resource_guid", "resourceGuid"]);
    replay.guid = if resource_guid.is_empty() {
        replay
            .media
            .first()
            .map(|candidate| candidate.guid.clone())
            .unwrap_or_default()
    } else {
        resource_guid
    };
    if replay.lecturer.is_empty() {
        replay.lecturer = value_string(item, &["teacher_name", "lecturer_name", "lecturer"]);
    }
    if replay.record_time.is_empty() {
        replay.record_time = format_timestamp_value(&value_string(
            item,
            &["course_begin", "start_at", "record_time"],
        ));
    }
    if replay.sub_title.is_empty() {
        replay.sub_title = value_string(item, &["sub_title", "title"]);
    }
    if replay.room.is_empty() {
        replay.room = value_string(item, &["room_name", "room"]);
    }
    if let Some(term) = rich_term(item) {
        replay.term = term;
    }
    if replay.kcwybm.is_empty() {
        replay.kcwybm = rich_field(item, "kcwybm");
    }
    if replay.kcdm.is_empty() {
        replay.kcdm = rich_field(item, "kcdm");
    }
    if replay.college.is_empty() {
        replay.college = value_string(item, &["structure_name", "course_college", "college"]);
    }
    Ok(())
}

fn rich_information(item: &Value) -> Value {
    item.get("information")
        .and_then(|value| value.as_str())
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or(Value::Null)
}

fn rich_field(item: &Value, key: &str) -> String {
    let direct = value_string(item, &[key]);
    if !direct.is_empty() {
        return direct;
    }
    value_string(&rich_information(item), &[key])
}

fn rich_term(item: &Value) -> Option<String> {
    let info = rich_information(item);
    let year = value_string(&info, &["kkxn"]);
    let semester = value_string(&info, &["kkxq"]);
    if !year.is_empty() && !semester.is_empty() {
        return Some(format!("{year}学年第{semester}学期"));
    }
    non_empty(value_string(item, &["course_term", "term_name"]))
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

pub(crate) fn format_timestamp_value(value: &str) -> String {
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

fn extract_casapi_url(html: &str) -> Option<String> {
    static CASAPI: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"https?://yjapise\.pku\.edu\.cn/casapi/[^\s"'<>]+"#).unwrap()
    });
    let html = html.replace("\\/", "/").replace("&amp;", "&");
    CASAPI.find(&html).map(|m| m.as_str().to_string())
}

fn course_id_from_play_url(play_url: &str) -> Option<String> {
    let token = token_from_play_url(play_url)?;
    let payload = decode_jwt(&token)?;
    let course_id = value_string(&payload, &["hqyCourseId"]);
    if course_id.is_empty() {
        None
    } else {
        Some(course_id)
    }
}

fn body_snippet(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

pub fn parse_list(value: &Value, course: &Course) -> Vec<Replay> {
    let Some(items) = extract_list(value) else {
        return Vec::new();
    };
    let mut replays = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let sub_title = value_string(&item, &["sub_title", "subTitle", "title"]);
        let replay_id = value_string(&item, &["sub_id", "subId", "id", "hqySubId"]);
        let replay_id = if replay_id.is_empty() {
            sub_title.clone()
        } else {
            replay_id
        };
        if replay_id.is_empty() || !seen.insert(replay_id.clone()) {
            continue;
        }
        let lecturer = value_string(&item, &["lecturer_name", "lecturer", "teacher"]);
        let record_time = format_timestamp_value(&value_string(
            &item,
            &["record_time", "recordTime", "class_begin", "course_begin"],
        ));
        let sub_content = item
            .get("sub_content")
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        let media = media::extract_media_candidates(&sub_content);
        let resource_guid = value_string(&item, &["resource_guid", "resourceGuid"]);
        let guid = if resource_guid.is_empty() {
            media
                .first()
                .map(|candidate| candidate.guid.clone())
                .unwrap_or_default()
        } else {
            resource_guid
        };
        replays.push(Replay {
            course_id: course.id.clone(),
            course_name: course.name.clone(),
            replay_id,
            sub_title: if sub_title.is_empty() { "回放".to_string() } else { sub_title },
            lecturer,
            record_time,
            page_url: String::new(),
            media,
            guid,
            ..Default::default()
        });
    }
    replays
}

pub fn value_string(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = value.get(*key) {
            if let Some(text) = value.as_str() {
                if !text.is_empty() {
                    return text.to_string();
                }
            }
            if let Some(number) = value.as_u64() {
                return number.to_string();
            }
            if let Some(number) = value.as_i64() {
                return number.to_string();
            }
        }
    }
    String::new()
}

fn as_header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_with_payload(payload: &str) -> String {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        format!("header.{encoded}.signature")
    }

    #[test]
    fn decodes_jwt_payload() {
        let token = token_with_payload(r#"{"hqyCourseId":"200001","hqySubId":"900001"}"#);
        let payload = decode_jwt(&token).unwrap();
        assert_eq!(payload["hqyCourseId"], "200001");
        assert_eq!(payload["hqySubId"], "900001");
    }

    #[test]
    fn extracts_signed_casapi_entry() {
        let html = r#"<iframe src="https://yjapise.pku.edu.cn/casapi/index.php?r=auth/login-with-sign&amp;forward=https%3A%2F%2Fonlineroomse.pku.edu.cn%2Fplayer%3Fcourse_id%3D200001%26sub_id%3D900001&amp;sign=abc"></iframe>"#;
        let url = extract_casapi_url(html).unwrap();
        assert!(url.contains("casapi/index.php"));
        assert!(url.contains("sign=abc"));
        assert!(!url.contains("&amp;"));
    }

    #[test]
    fn extracts_token_from_play_url() {
        let url = "https://course.pku.edu.cn/webapps/bb-streammedia-hqy-BBLEARN/playVideo.action?course_id=1&token=abc.def.ghi";
        assert_eq!(token_from_play_url(url).unwrap(), "abc.def.ghi");
    }

    #[test]
    fn parses_onlineroomse_list() {
        let course = Course {
            id: "10001_1".to_string(),
            name: "测试课程".to_string(),
            teacher: "测试教师".to_string(),
            term: "2026春".to_string(),
            url: String::new(),
        };
        let value = serde_json::json!({
            "list": [{
                "sub_id": "900001",
                "sub_title": "第3-4节",
                "lecturer_name": "测试教师",
                "recordTime": "2026-11-06",
                "sub_content": "{\"save_playback\":{\"contents\":\"https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/abc123/0/playlist.m3u8\"}}"
            }]
        });
        let replays = parse_list(&value, &course);
        assert_eq!(replays.len(), 1);
        assert_eq!(replays[0].replay_id, "900001");
        assert_eq!(replays[0].guid, "abc123");
        assert_eq!(replays[0].media.len(), 1);
    }

    #[test]
    fn parses_legacy_mp4_replay_with_resource_guid() {
        let course = Course {
            id: "10002_1".to_string(),
            name: "测试课程".to_string(),
            teacher: "测试教师".to_string(),
            term: "24-25学年第1学期".to_string(),
            url: String::new(),
        };
        let value = serde_json::json!({
            "list": [{
                "sub_id": "900002",
                "sub_title": "2024-01-02第1-2节",
                "lecturer_name": "测试教师",
                "resource_guid": "00112233445566778899aabbccddeeff",
                "sub_content": "{\"save_playback\":{\"contents\":\"http://resourcese.pku.edu.cn/play/video/vod/newLowVideo/2024/01/02/FEDCBA9876543210FEDCBA9876543210/h264_500k_mp4_640_360.mp4\",\"is_m3u8\":\"no\"}}"
            }]
        });
        let replays = parse_list(&value, &course);
        assert_eq!(replays.len(), 1);
        assert_eq!(replays[0].guid, "00112233445566778899aabbccddeeff");
        assert_eq!(replays[0].media.len(), 1);
        assert_eq!(replays[0].media[0].kind, "file");
        assert_eq!(
            replays[0].media[0].guid,
            "FEDCBA9876543210FEDCBA9876543210"
        );
    }
}
