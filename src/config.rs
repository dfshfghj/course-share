use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_backend")]
    pub backend_url: String,
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    #[serde(default = "default_ffmpeg")]
    pub ffmpeg: String,
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub starred_courses: Vec<String>,
}

fn default_backend() -> String {
    std::env::var("COURSE_SHARE_BACKEND")
        .unwrap_or_else(|_| "http://10.129.241.188:18080".to_string())
}

fn default_download_dir() -> String {
    dirs::download_dir()
        .or_else(|| dirs::home_dir())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CourseShare")
        .to_string_lossy()
        .to_string()
}

fn default_ffmpeg() -> String {
    String::new()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend_url: default_backend(),
            download_dir: default_download_dir(),
            ffmpeg: default_ffmpeg(),
            dark: false,
            starred_courses: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("course-share")
    }

    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("course-share")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn cookie_path() -> PathBuf {
        Self::data_dir().join("cookies.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut config = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default();
        if config.ffmpeg.trim() == "ffmpeg" || config.ffmpeg.trim() == "auto" {
            config.ffmpeg.clear();
        }
        config
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let data = serde_json::to_vec_pretty(self)?;
        fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn download_path(&self) -> PathBuf {
        PathBuf::from(shellexpand_home(&self.download_dir))
    }
}

fn shellexpand_home(value: &str) -> String {
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    value.to_string()
}
