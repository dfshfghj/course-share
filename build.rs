use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const HLS_VERSION: &str = "1.7.4";
const HLS_SHA256: &str = "482cccb3a224554f64d8c2cf0d335dba4a3819bdb726afbfddfe15de4c277d82";
const FALLBACK_HLS_JS: &str = r#"window.Hls = undefined;
console.warn("hls.js unavailable: native HLS playback only");"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/ffmpeg-minimal");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-env-changed=HLS_JS_URL");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=COURSE_SHARE_TARGET={target}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    prepare_hls_js(&manifest, &out_dir);

    let exe_name = if target.contains("windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = manifest
        .join("vendor")
        .join("ffmpeg-minimal")
        .join(&target)
        .join("bin")
        .join(exe_name);
    if !bundled.is_file() {
        return;
    }

    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    let destination = profile_dir.join(exe_name);
    let temporary = profile_dir.join(format!("{exe_name}.tmp"));
    if let Err(error) = fs::copy(&bundled, &temporary) {
        println!("cargo:warning=failed to copy bundled ffmpeg: {error}");
        return;
    }
    #[cfg(windows)]
    let _ = fs::remove_file(&destination);
    if let Err(error) = fs::rename(&temporary, &destination) {
        println!("cargo:warning=failed to install bundled ffmpeg: {error}");
    }
}

/// hls.js is not committed (it is a large third-party asset). Prefer a local
/// `assets/hls.min.js` if present; otherwise download a pinned light build.
/// If the network is unavailable, emit a tiny stub so native-HLS browsers can
/// still build and run. The file is written to OUT_DIR, never the source tree.
fn prepare_hls_js(manifest: &Path, out_dir: &Path) {
    let destination = out_dir.join("hls.min.js");
    if destination.is_file() {
        return;
    }

    let local = manifest.join("assets").join("hls.min.js");
    if local.is_file() {
        if let Err(error) = fs::copy(&local, &destination) {
            println!("cargo:warning=failed to copy assets/hls.min.js: {error}");
        } else {
            return;
        }
    }

    let url = env::var("HLS_JS_URL").unwrap_or_else(|_| {
        format!(
            "https://cdn.jsdelivr.net/npm/hls.js@{HLS_VERSION}/dist/hls.light.min.js"
        )
    });
    println!("cargo:warning=downloading hls.js {HLS_VERSION} from {url}");
    let temporary = out_dir.join("hls.min.js.part");
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(&temporary)
        .arg(&url)
        .status();
    match status {
        Ok(status) if status.success() => {
            match fs::read(&temporary) {
                Ok(bytes) if sha256(&bytes) == HLS_SHA256 => {
                    let _ = fs::rename(&temporary, &destination);
                    return;
                }
                Ok(_) => {
                    println!("cargo:warning=hls.js checksum mismatch, using native-HLS fallback");
                }
                Err(error) => {
                    println!("cargo:warning=could not read downloaded hls.js: {error}");
                }
            }
            let _ = fs::remove_file(&temporary);
        }
        Ok(status) => {
            println!("cargo:warning=curl exited with {status}; using native-HLS fallback");
        }
        Err(error) => {
            println!("cargo:warning=could not run curl: {error}; using native-HLS fallback");
        }
    }
    let _ = fs::write(&destination, FALLBACK_HLS_JS);
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
