use crate::blackboard::{Course, Replay};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::Url;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Share {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub m3u8_url: String,
    #[serde(default)]
    pub course_name: String,
    #[serde(default)]
    pub course_id: String,
    #[serde(default)]
    pub replay_title: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub term: String,
    #[serde(default)]
    pub record_date: String,
    #[serde(default)]
    pub record_time: String,
    #[serde(default)]
    pub uploader: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub downloads: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Share {
    pub fn from_replay(course: &Course, replay: &Replay, uploader: &str) -> Option<Self> {
        let candidate = replay
            .media
            .iter()
            .find(|candidate| candidate.url.contains("resourcese.pku.edu.cn"))?;
        let guid = if !replay.guid.is_empty() {
            replay.guid.clone()
        } else {
            candidate.guid.clone()
        };
        if guid.is_empty() {
            return None;
        }
        Some(Self {
            guid,
            m3u8_url: candidate.url.clone(),
            course_name: course.name.clone(),
            course_id: course.id.clone(),
            replay_title: if replay.sub_title.is_empty() {
                replay.replay_id.clone()
            } else {
                replay.sub_title.clone()
            },
            teacher: if replay.lecturer.is_empty() {
                course.teacher.clone()
            } else {
                replay.lecturer.clone()
            },
            term: course.term.clone(),
            record_date: record_date_from_url(&candidate.url),
            record_time: replay.record_time.clone(),
            uploader: uploader.to_string(),
            ..Default::default()
        })
    }


}

fn record_date_from_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return String::new();
    };
    let parts: Vec<&str> = parsed.path().split('/').collect();
    if let Some(index) = parts.iter().position(|part| *part == "harpocrates") {
        if parts.len() > index + 3 {
            return format!("{}-{}-{}", parts[index + 1], parts[index + 2], parts[index + 3]);
        }
    }
    String::new()
}

#[derive(Clone)]
pub struct ShareClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl ShareClient {
    pub fn new(base_url: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .context("build share client")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    pub fn health(&self) -> Result<serde_json::Value> {
        let url = format!("{}/api/health", self.base_url);
        let response = self.client.get(&url).send().with_context(|| format!("GET {url}"))?;
        parse_json(response)
    }

    pub fn list(&self, query: &str) -> Result<Vec<Share>> {
        let mut url = Url::parse(&format!("{}/api/shares", self.base_url))
            .with_context(|| format!("invalid backend URL {}", self.base_url))?;
        url.query_pairs_mut()
            .append_pair("limit", "200")
            .append_pair("q", query);
        let response = self
            .client
            .get(url.as_str())
            .send()
            .with_context(|| format!("GET {}", url.as_str()))?;
        let value = parse_json(response)?;
        let shares = value
            .get("shares")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        serde_json::from_value(shares).context("parse share list")
    }

    pub fn publish(&self, share: &Share) -> Result<Share> {
        let url = format!("{}/api/shares", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(share)
            .send()
            .with_context(|| format!("POST {url}"))?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        if !status.is_success() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
                    return Err(anyhow!("发布失败：{error}"));
                }
            }
            return Err(anyhow!("发布失败：HTTP {}", status.as_u16()));
        }
        serde_json::from_str(&text).context("parse published share")
    }

    pub fn mark_download(&self, id: &str) -> Result<String> {
        let url = format!("{}/api/shares/{}/download", self.base_url, id);
        let response = self
            .client
            .post(&url)
            .send()
            .with_context(|| format!("POST {url}"))?;
        let value = parse_json(response)?;
        value
            .get("m3u8_url")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .ok_or_else(|| anyhow!("后端没有返回 m3u8_url"))
    }
}

fn parse_json(response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
                return Err(anyhow!("后端错误：{error}"));
            }
        }
        return Err(anyhow!("后端返回 HTTP {}", status.as_u16()));
    }
    serde_json::from_str(&text).context("parse backend JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blackboard::Course;
    use crate::media::MediaCandidate;

    #[test]
    fn derives_record_date_from_guid_path() {
        let url = "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/37692e24e0b44566990731ac94b414c2/0/playlist.m3u8";
        assert_eq!(record_date_from_url(url), "2026-11-06");
    }

    #[test]
    fn builds_share_from_replay() {
        let course = Course {
            id: "86268_1".to_string(),
            name: "算法设计与分析".to_string(),
            teacher: "张老师".to_string(),
            term: "2026春".to_string(),
            url: String::new(),
        };
        let replay = Replay {
            course_id: course.id.clone(),
            course_name: course.name.clone(),
            replay_id: "4604286".to_string(),
            sub_title: "第3-4节".to_string(),
            lecturer: "张老师".to_string(),
            record_time: "2026-11-06".to_string(),
            page_url: String::new(),
            media: vec![MediaCandidate {
                url: "https://resourcese.pku.edu.cn/play/0/harpocrates/2026/11/06/37692e24e0b44566990731ac94b414c2/0/playlist.m3u8".to_string(),
                kind: "hls".to_string(),
                label: "saved playback".to_string(),
                quality: 26,
                guid: "37692e24e0b44566990731ac94b414c2".to_string(),
            }],
            guid: "37692e24e0b44566990731ac94b414c2".to_string(),
        };
        let share = Share::from_replay(&course, &replay, "tester").unwrap();
        assert_eq!(share.guid, replay.guid);
        assert_eq!(share.record_date, "2026-11-06");
        assert_eq!(share.uploader, "tester");
        assert_eq!(share.course_name, "算法设计与分析");
    }
}
