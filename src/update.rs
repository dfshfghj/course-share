#![cfg_attr(debug_assertions, allow(dead_code))]

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const RELEASES_API: &str =
    "https://api.github.com/repos/dfshfghj/course-share/releases/latest";

#[derive(Clone, Debug)]
pub(crate) struct UpdateInfo {
    pub(crate) version: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
}

pub(crate) fn perform_update() -> Result<String> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("dfshfghj")
        .repo_name("course-share")
        .bin_name("course-share")
        .bin_path_in_archive("course-share-{{ target }}/{{ bin }}")
        .target(target_name())
        .current_version(env!("CARGO_PKG_VERSION"))
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()
        .context("configure self update")?
        .update()
        .context("download or install update")?;
    if !status.is_updated() {
        return Err(anyhow::anyhow!("没有可安装的新版本"));
    }
    Ok(status.version().to_string())
}

fn target_name() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        "unknown"
    }
}

pub(crate) fn check(current: &str) -> Result<Option<UpdateInfo>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .context("build update client")?;
    let response = client
        .get(RELEASES_API)
        .header(reqwest::header::USER_AGENT, "course-share-updater")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .context("GET GitHub latest release")?
        .error_for_status()
        .context("GitHub release request failed")?;
    let release: Release = response.json().context("parse GitHub release")?;
    if !version_is_newer(&release.tag_name, current) {
        return Ok(None);
    }
    Ok(Some(UpdateInfo {
        version: release.tag_name,
    }))
}

fn version_is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn parse_version(value: &str) -> (u64, u64, u64) {
    let value = value.trim();
    let value = value
        .strip_prefix('v')
        .or_else(|| value.strip_prefix('V'))
        .unwrap_or(value);
    let core = value
        .split(['-', '+'])
        .next()
        .unwrap_or(value);
    let mut parts = core
        .split('.')
        .map(|part| part.trim().parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_tags() {
        assert!(version_is_newer("v0.2.0", "0.1.0"));
        assert!(version_is_newer("v0.1.1", "v0.1.0"));
        assert!(version_is_newer("v1.0.0", "0.9.9"));
        assert!(!version_is_newer("v0.1.0", "0.1.0"));
        assert!(!version_is_newer("0.1.0", "v0.2.0"));
        assert!(version_is_newer("v0.2.0-beta.1", "0.1.0"));
    }
}
