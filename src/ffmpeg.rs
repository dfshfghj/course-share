use std::env;
use std::path::{Path, PathBuf};

pub struct LocatedFfmpeg {
    pub path: PathBuf,
    pub source: &'static str,
}

pub fn resolve(configured: &str) -> String {
    locate(configured).path.to_string_lossy().into_owned()
}

pub fn locate(configured: &str) -> LocatedFfmpeg {
    let configured = configured.trim();
    if !configured.is_empty() && configured != "auto" && configured != "ffmpeg" {
        return LocatedFfmpeg {
            path: PathBuf::from(configured),
            source: "手动指定",
        };
    }

    if let Some(path) = env::var_os("COURSE_SHARE_FFMPEG")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return LocatedFfmpeg {
            path,
            source: "环境变量",
        };
    }

    for path in bundled_candidates() {
        if path.is_file() {
            return LocatedFfmpeg {
                path,
                source: "内置",
            };
        }
    }

    if let Some(path) = find_in_path(exe_name()) {
        return LocatedFfmpeg {
            path,
            source: "系统 PATH",
        };
    }

    LocatedFfmpeg {
        path: PathBuf::from("ffmpeg"),
        source: "未找到",
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn bundled_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(exe_name()));
            candidates.push(dir.join("resources").join(exe_name()));
            if cfg!(target_os = "macos") {
                candidates.push(dir.join("../Resources").join(exe_name()));
            }
        }
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("ffmpeg-minimal")
            .join(env!("COURSE_SHARE_TARGET"))
            .join("bin")
            .join(exe_name()),
    );
    candidates
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
