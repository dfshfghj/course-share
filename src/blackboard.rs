use crate::http::{redact_url, Http};
use crate::media::{self, MediaCandidate};
use anyhow::{anyhow, Context, Result};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

pub const COURSE_BASE: &str = "https://course.pku.edu.cn";
pub const COURSE_PORTAL: &str = "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";

#[derive(Clone, Debug)]
pub struct Course {
    pub id: String,
    pub name: String,
    pub teacher: String,
    pub term: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replay {
    pub course_id: String,
    pub course_name: String,
    pub replay_id: String,
    pub sub_title: String,
    pub lecturer: String,
    pub record_time: String,
    pub page_url: String,
    pub media: Vec<MediaCandidate>,
    pub guid: String,
}

impl Replay {
    pub fn identity(&self) -> String {
        if !self.replay_id.is_empty() {
            format!("blackboard:{}:{}", self.course_id, self.replay_id)
        } else if !self.guid.is_empty() {
            format!("guid:{}", self.guid)
        } else {
            format!("meta:{}:{}", self.course_id, self.sub_title)
        }
    }

    pub fn display_title(&self) -> String {
        let mut parts = vec![self.course_name.clone()];
        if !self.sub_title.is_empty() {
            parts.push(self.sub_title.clone());
        }
        if !self.lecturer.is_empty() {
            parts.push(self.lecturer.clone());
        }
        parts.join(" · ")
    }

}

#[derive(Clone, Debug, Default)]
pub struct PlayEntry {
    pub play_url: String,
    pub row_text: String,
    pub replay_id: String,
    pub record_time: String,
}

#[derive(Clone, Debug, Default)]
pub struct RoomRef {
    pub launch_url: String,
    pub course_id: String,
    pub sub_id: String,
}

pub fn list_courses(http: &Http) -> Result<Vec<Course>> {
    let shell = http
        .get_follow(COURSE_PORTAL, 12)
        .context("打开教学网门户")?;
    if !shell.is_success() {
        return Err(anyhow!(
            "教学网门户返回 HTTP {}；请重新登录或检查 Cookie",
            shell.status
        ));
    }
    let mut groups = extract_tab_groups(&shell.body);
    if groups.is_empty() {
        groups = vec!["_2_1".to_string(), "_1_1".to_string()];
    }
    groups.sort_by_key(|group| match group.as_str() {
        "_2_1" => 0,
        "_1_1" => 1,
        _ => 2,
    });
    groups.dedup();

    let mut courses = Vec::new();
    let mut seen = HashSet::new();
    for group in groups.iter().take(12) {
        let url = format!("{COURSE_PORTAL}?tab_tab_group_id={group}");
        let Ok(page) = http.get_follow(&url, 12) else {
            continue;
        };
        if !page.is_success() {
            continue;
        }
        parse_courses(&page.body, &page.final_url, &mut courses, &mut seen);
    }
    if courses.is_empty() {
        return Err(anyhow!(
            "没有解析到课程。可能未选课、页面改版或会话已过期。"
        ));
    }
    Ok(courses)
}

fn extract_tab_groups(html: &str) -> Vec<String> {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"tab_tab_group_id=(_[0-9]+_[0-9]+)").unwrap());
    RE.captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn parse_courses(html: &str, base: &Url, out: &mut Vec<Course>, seen: &mut HashSet<String>) {
    let document = Html::parse_document(html);
    let Ok(anchor_selector) = Selector::parse("a[href]") else {
        return;
    };
    for element in document.select(&anchor_selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Some(id) = extract_course_id(href) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let raw_text = clean_text(&element.text().collect::<Vec<_>>().join(" "));
        let (name, term) = split_course_label(&raw_text);
        let url = base
            .join(href)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| format!("{COURSE_BASE}/{href}"));
        out.push(Course {
            id,
            name,
            teacher: String::new(),
            term,
            url,
        });
    }
}

fn extract_course_id(href: &str) -> Option<String> {
    let marker = "id=PkId{key=";
    let start = href.find(marker)? + marker.len();
    let rest = &href[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    let id = rest[..end].trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn split_course_label(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if raw.is_empty() {
        return ("未命名课程".to_string(), String::new());
    }
    // Blackboard link text is usually "课程名: 课程 (24-25学年第1学期)".
    let colon = raw.rfind(':').max(raw.rfind('：'));
    let base = if let Some(index) = colon {
        let next = index + raw[index..].chars().next().map(char::len_utf8).unwrap_or(1);
        raw[next..].trim()
    } else {
        raw
    };
    if let Some(open) = base.rfind('(') {
        if base.ends_with(')') {
            let inside = &base[open + 1..base.len() - 1];
            if ["学期", "学年", "春", "秋"].iter().any(|key| inside.contains(key)) {
                let name = base[..open].trim();
                let mut term = inside.trim().to_string();
                for word in [
                    "本研合上",
                    "本研",
                    "本科生",
                    "研究生",
                    "本科",
                    "硕士",
                    "博士",
                ] {
                    term = term.replace(word, "");
                }
                let term = term.trim().to_string();
                return (
                    if name.is_empty() {
                        base.to_string()
                    } else {
                        name.to_string()
                    },
                    term,
                );
            }
        }
    }
    (base.to_string(), String::new())
}

pub fn list_replays(http: &Http, course: &Course) -> Result<Vec<Replay>> {
    let course_page = http
        .get_follow(&course.url, 12)
        .with_context(|| format!("打开课程页 {}", course.name))?;
    if !course_page.is_success() {
        return Err(anyhow!(
            "课程页返回 HTTP {}：{}",
            course_page.status,
            course.name
        ));
    }
    let mut queue = parse_menu_links(&course_page.body, &course_page.final_url);
    if queue.is_empty() {
        queue.push(course.url.clone());
    }

    let mut play_entries = Vec::new();
    let mut room_refs = Vec::new();
    let mut visited = HashSet::new();
    while let Some(page_url) = queue.pop() {
        if !visited.insert(page_url.clone()) || visited.len() > 20 {
            continue;
        }
        let Ok(page) = http.get_follow(&page_url, 12) else {
            continue;
        };
        if !page.is_success() {
            continue;
        }
        parse_replay_page(&page.body, &page.final_url, &mut play_entries, &mut room_refs, &mut queue);
    }

    let mut replays = Vec::new();
    // If only a player iframe is available, try the legacy room API first.
    if play_entries.is_empty() {
        if let Some(room) = room_refs.first() {
            if let Ok(list) = crate::onlineroomse::fetch_list_for_room(http, room) {
                replays = crate::onlineroomse::parse_list(&list, course);
            }
        }
    }

    merges_play_entries(&mut replays, &play_entries, course);
    if replays.is_empty() && !room_refs.is_empty() {
        for room in room_refs {
            if room.sub_id.is_empty() {
                continue;
            }
            replays.push(Replay {
                course_id: course.id.clone(),
                course_name: course.name.clone(),
                replay_id: room.sub_id.clone(),
                sub_title: format!("回放 {}", room.sub_id),
                lecturer: String::new(),
                record_time: String::new(),
                page_url: room.launch_url.clone(),
                media: Vec::new(),
                guid: String::new(),
            });
        }
    }
    replays.sort_by(|a, b| a.sub_title.cmp(&b.sub_title));
    replays.dedup_by(|a, b| a.identity() == b.identity());

    // Resolve media through the real player flow (playVideo -> casapi -> player
    // auth_data -> courseapi schedule detail). Cap the eager pass to keep huge
    // course lists responsive; downloads retry lazily for the rest.
    for replay in replays.iter_mut().take(30) {
        if replay.media.is_empty() && !replay.page_url.is_empty() {
            let _ = crate::onlineroomse::resolve_via_player(http, replay);
        }
    }
    Ok(replays)
}

fn parse_menu_links(html: &str, base: &Url) -> Vec<String> {
    let document = Html::parse_document(html);
    let mut links = Vec::new();
    let mut selectors = Vec::new();
    if let Ok(selector) = Selector::parse("#courseMenuPalette_contents a[href]") {
        selectors.push(selector);
    }
    if let Ok(selector) = Selector::parse("a[href]") {
        selectors.push(selector);
    }
    for selector in selectors {
        for element in document.select(&selector) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            let text = element.text().collect::<Vec<_>>().join(" ");
            let href_lower = href.to_ascii_lowercase();
            let is_replay = ["录播", "回放", "视频", "课堂实录", "在线课堂"]
                .iter()
                .any(|keyword| text.contains(keyword))
                || ["playvideo", "bb-streammedia", "streammedia", "videolist"]
                    .iter()
                    .any(|keyword| href_lower.contains(keyword));
            if !is_replay {
                continue;
            }
            if let Ok(url) = base.join(href) {
                let url = url.to_string();
                if !links.contains(&url) {
                    links.push(url);
                }
            }
        }
        if !links.is_empty() {
            break;
        }
    }
    links
}

fn parse_replay_page(
    html: &str,
    base: &Url,
    play_entries: &mut Vec<PlayEntry>,
    room_refs: &mut Vec<RoomRef>,
    queue: &mut Vec<String>,
) {
    let document = Html::parse_document(html);
    let anchor_selector = Selector::parse("a[href]").unwrap();
    let iframe_selector = Selector::parse("iframe[src]").unwrap();

    for element in document.select(&anchor_selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let url = base.join(href).map(|u| u.to_string()).unwrap_or_else(|_| href.to_string());
        if href.contains("playVideo.action") && href.contains("token=") {
            let row_text = row_text(element);
            if let Some(entry) = play_entry_from_url(url.clone(), row_text) {
                push_play_entry(play_entries, entry);
            }
        }
        if href.contains("videoList.action") && !queue.contains(&url) {
            queue.push(url.clone());
        }
        if let Some(room) = extract_room_ref(&url) {
            if !room_refs.iter().any(|r| r.launch_url == room.launch_url) {
                room_refs.push(room);
            }
        }
    }

    for element in document.select(&iframe_selector) {
        let Some(src) = element.value().attr("src") else {
            continue;
        };
        let url = base.join(src).map(|u| u.to_string()).unwrap_or_else(|_| src.to_string());
        if let Some(room) = extract_room_ref(&url) {
            if !room_refs.iter().any(|r| r.launch_url == room.launch_url) {
                room_refs.push(room);
            }
        }
    }

    static PLAY_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"playVideo\.action\?[^"'\s<>]*token=[^"'\s<>&]+"#).unwrap()
    });
    for found in PLAY_RE.find_iter(html) {
        let url = base
            .join(found.as_str())
            .map(|u| u.to_string())
            .unwrap_or_else(|_| found.as_str().to_string());
        if let Some(entry) = play_entry_from_url(url, String::new()) {
            push_play_entry(play_entries, entry);
        }
    }
}

fn push_play_entry(entries: &mut Vec<PlayEntry>, entry: PlayEntry) {
    let duplicate = entries.iter().any(|existing| {
        existing.play_url == entry.play_url
            || (!entry.replay_id.is_empty() && existing.replay_id == entry.replay_id)
    });
    if !duplicate {
        entries.push(entry);
    }
}

fn play_entry_from_url(url: String, row_text: String) -> Option<PlayEntry> {
    let token = crate::onlineroomse::token_from_play_url(&url)?;
    let payload = crate::onlineroomse::decode_jwt(&token)?;
    let replay_id = payload
        .get("hqySubId")
        .and_then(|v| v.as_str().map(ToOwned::to_owned).or_else(|| v.as_u64().map(|n| n.to_string())))
        .unwrap_or_default();
    let record_time = payload
        .get("recordTime")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(PlayEntry {
        play_url: url,
        row_text,
        replay_id,
        record_time,
    })
}

fn merges_play_entries(replays: &mut Vec<Replay>, play_entries: &[PlayEntry], course: &Course) {
    for entry in play_entries {
        let (sub_title, lecturer) = parse_row_text(&entry.row_text, &entry.record_time);
        if let Some(existing) = replays
            .iter_mut()
            .find(|replay| !entry.replay_id.is_empty() && replay.replay_id == entry.replay_id)
        {
            existing.page_url = entry.play_url.clone();
            if existing.sub_title.is_empty() {
                existing.sub_title = sub_title;
            }
            if existing.lecturer.is_empty() {
                existing.lecturer = lecturer;
            }
            continue;
        }
        replays.push(Replay {
            course_id: course.id.clone(),
            course_name: course.name.clone(),
            replay_id: entry.replay_id.clone(),
            sub_title,
            lecturer,
            record_time: entry.record_time.clone(),
            page_url: entry.play_url.clone(),
            media: Vec::new(),
            guid: String::new(),
        });
    }
}

fn parse_row_text(row_text: &str, record_time: &str) -> (String, String) {
    let text = clean_text(row_text);
    static TEACHER: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"教\s*师\s*[:：]\s*([^\s]+)").unwrap());
    static NAME_TIME: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^\s*(.*?)\s+时\s*间\s*[:：]").unwrap());
    let lecturer = TEACHER
        .captures(&text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .unwrap_or_default();
    let sub_title = NAME_TIME
        .captures(&text)
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if record_time.is_empty() {
                if text.is_empty() {
                    "回放".to_string()
                } else {
                    text
                }
            } else {
                record_time.to_string()
            }
        });
    (sub_title, lecturer)
}

pub fn resolve_media(http: &Http, replay: &mut Replay) -> Result<()> {
    if !replay.media.is_empty() {
        return Ok(());
    }
    let player_error = match crate::onlineroomse::resolve_via_player(http, replay) {
        Ok(()) => return Ok(()),
        Err(error) => format!("{error:#}"),
    };
    if replay.page_url.is_empty() {
        return Err(anyhow!(
            "这条回放没有播放页 URL，无法解析媒体地址（播放器解析失败：{player_error}）"
        ));
    }
    let page = http
        .get_follow(&replay.page_url, 12)
        .with_context(|| format!("打开回放页 {}", replay.sub_title))?;
    if !page.is_success() {
        return Err(anyhow!(
            "回放页返回 HTTP {}（播放器解析失败：{player_error}）",
            page.status
        ));
    }
    let candidates = media::extract_media_from_html(&page.body);
    if candidates.is_empty() {
        return Err(anyhow!(
            "回放页没有找到 resourcese 媒体地址（page {}{}；播放器解析失败：{player_error}）。\
             downloadVideo.action 已失效，不能作为下载入口。",
            redact_url(&page.final_url),
            if page.final_url.query().is_some() { " (query redacted)" } else { "" }
        ));
    }
    replay.guid = candidates
        .first()
        .map(|candidate| candidate.guid.clone())
        .unwrap_or_default();
    replay.media = candidates;
    Ok(())
}

fn row_text(element: ElementRef<'_>) -> String {
    let mut current = Some(element);
    while let Some(node) = current {
        if node.value().name() == "tr" {
            return clean_text(&node.text().collect::<Vec<_>>().join(" "));
        }
        current = node.parent().and_then(ElementRef::wrap);
    }
    clean_text(&element.text().collect::<Vec<_>>().join(" "))
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn extract_room_ref(url: &str) -> Option<RoomRef> {
    let parsed = Url::parse(url).ok()?;
    extract_room_ref_from_url(&parsed, url, 0)
}

fn extract_room_ref_from_url(parsed: &Url, launch_url: &str, depth: usize) -> Option<RoomRef> {
    if depth > 3 {
        return None;
    }
    let host = parsed.host_str().unwrap_or("");
    if host.contains("onlineroomse.pku.edu.cn") && parsed.path().contains("player") {
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        let course_id = query.get("course_id").map(|v| v.to_string()).unwrap_or_default();
        let sub_id = query.get("sub_id").map(|v| v.to_string()).unwrap_or_default();
        if course_id.is_empty() {
            return None;
        }
        return Some(RoomRef {
            launch_url: launch_url.to_string(),
            course_id,
            sub_id,
        });
    }
    if host.contains("yjapise.pku.edu.cn") {
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        if let Some(forward) = query.get("forward") {
            let nested = Url::parse(forward).ok()?;
            return extract_room_ref_from_url(&nested, launch_url, depth + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://course.pku.edu.cn/").unwrap()
    }

    fn token_with_payload(payload: &str) -> String {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        format!("header.{encoded}.signature")
    }

    #[test]
    fn parses_course_links() {
        let html = r#"<ul class="courseListing">
            <li><a href="/webapps/blackboard/execute/launcher?type=Course&id=PkId{key=10001_1,type=Course}">课程名: 测试课程 (2026春)</a></li>
        </ul>"#;
        let mut courses = Vec::new();
        let mut seen = HashSet::new();
        parse_courses(html, &base(), &mut courses, &mut seen);
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].id, "10001_1");
        assert_eq!(courses[0].name, "测试课程");
        assert_eq!(courses[0].term, "2026春");

        let html = r#"<ul class="courseListing">
            <li><a href="/x?id=PkId{key=_2_1,type=Course}">测试课程(B)习题(24-25学年第1学期本研合上)</a></li>
        </ul>"#;
        let mut courses = Vec::new();
        let mut seen = HashSet::new();
        parse_courses(html, &base(), &mut courses, &mut seen);
        assert_eq!(courses[0].name, "测试课程(B)习题");
        assert_eq!(courses[0].term, "24-25学年第1学期");
    }

    #[test]
    fn parses_replay_page_with_play_video_and_room_iframe() {
        let token = token_with_payload(
            r#"{"hqyCourseId":"200001","hqySubId":"900001","recordTime":"2026-11-06"}"#,
        );
        let html = format!(
            r#"<html><body>
            <a href="/webapps/bb-streammedia-hqy-BBLEARN/playVideo.action?x=1&token={token}">第3次课 时间：2026-11-06 教师：测试教师</a>
            <iframe src="https://yjapise.pku.edu.cn/casapi/index.php?r=auth/login-with-sign&forward=https%3A%2F%2Fonlineroomse.pku.edu.cn%2Fplayer%3Fcourse_id%3D200001%26sub_id%3D900001"></iframe>
            </body></html>"#
        );
        let mut plays = Vec::new();
        let mut rooms = Vec::new();
        let mut queue = Vec::new();
        parse_replay_page(&html, &base(), &mut plays, &mut rooms, &mut queue);
        assert_eq!(plays.len(), 1);
        assert_eq!(plays[0].replay_id, "900001");
        assert!(plays[0].row_text.contains("测试教师"));
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].course_id, "200001");
        assert_eq!(rooms[0].sub_id, "900001");
    }

    #[test]
    fn extracts_room_ref_from_signed_forward() {
        let url = "https://yjapise.pku.edu.cn/casapi/index.php?r=auth/login-with-sign&forward=https%3A%2F%2Fonlineroomse.pku.edu.cn%2Fplayer%3Fcourse_id%3D200001%26sub_id%3D900001";
        let room = extract_room_ref(url).unwrap();
        assert_eq!(room.course_id, "200001");
        assert_eq!(room.sub_id, "900001");
    }

    #[test]
    fn parses_row_text() {
        let (sub_title, lecturer) = parse_row_text("第3次课 时间：2026-11-06 教师：测试教师", "");
        assert_eq!(sub_title, "第3次课");
        assert_eq!(lecturer, "测试教师");
    }
}
