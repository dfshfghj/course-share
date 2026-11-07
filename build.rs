use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/ffmpeg-minimal");

    let target = env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=COURSE_SHARE_TARGET={target}");

    let exe_name = if target.contains("windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("vendor")
        .join("ffmpeg-minimal")
        .join(&target)
        .join("bin")
        .join(exe_name);
    if !bundled.is_file() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
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
