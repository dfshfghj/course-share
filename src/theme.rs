use gpui_kit::{px, App, Hsla, Rgba, rgb};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeMode {
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub(crate) fn from_pref(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Auto,
        }
    }

    pub(crate) fn pref(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

static THEME_MODE: AtomicU8 = AtomicU8::new(0);
static LIGHT: AtomicBool = AtomicBool::new(true);

pub(crate) fn apply(pref: &str, cx: &mut App) {
    let mode = ThemeMode::from_pref(pref);
    let light = match mode {
        ThemeMode::Auto => system_prefers_light(),
        ThemeMode::Light => true,
        ThemeMode::Dark => false,
    };
    THEME_MODE.store(
        match mode {
            ThemeMode::Auto => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
        },
        Ordering::Relaxed,
    );
    LIGHT.store(light, Ordering::Relaxed);
    sync_component_theme(cx);
}

pub(crate) fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Map the app's light palette into the active light/dark palette.
pub(crate) fn tone(value: u32) -> Rgba {
    palette(value, is_light())
}

fn palette(value: u32, light: bool) -> Rgba {
    if light {
        return rgb(value);
    }
    let mapped = match value {
        0xf3f5f9 => 0x0b1220, // background
        0xffffff => 0x111a29, // panel
        0x111827 => 0xdbe6f5, // text
        0x6b7280 => 0x8fa4c0, // muted text
        0xe5e7eb => 0x1d2a3d, // border
        0x2563eb => 0x4dabf7, // primary
        0xe8efff => 0x16233a, // primary soft
        0xdc2626 => 0xff6b6b, // danger
        0x15803d => 0x51cf66, // success
        0xb45309 => 0xffd166, // warning
        0xf8fafc => 0x152238, // row hover
        0xf1f5f9 => 0x1b2b45, // hover strong
        0xeef2f8 => 0x152238, // section header
        0xe6ecf5 => 0x1d2a3d, // section header hover
        0x64748b => 0x8fa4c0, // arrow / icon
        0x1d4ed8 => 0x6bbcff, // primary hover
        0x1e40af => 0x8fd0ff, // primary active
        0xb91c1c => 0xe05252, // danger hover
        0x991b1b => 0xc53d3d, // danger active
        0x166534 => 0x40b45c, // success hover
        0x14532d => 0x349a4b, // success active
        0x92400e => 0xe0a93d, // warning hover
        0x78350f => 0xc78f2a, // warning active
        other => other,
    };
    rgb(mapped)
}

fn sync_component_theme(cx: &mut App) {
    let mode = if is_light() {
        gpui_component::ThemeMode::Light
    } else {
        gpui_component::ThemeMode::Dark
    };
    gpui_component::Theme::change(mode, None, cx);
    gpui_component::Theme::update(cx, |theme| {
        apply_component_palette(theme, is_light());
    });
    cx.refresh_windows();
}

fn apply_component_palette(theme: &mut gpui_component::Theme, light: bool) {
    let c = |value| Hsla::from(palette(value, light));
    let colors = &mut theme.colors;
    colors.background = c(0xf3f5f9);
    colors.foreground = c(0x111827);
    colors.border = c(0xe5e7eb);
    colors.input = c(0xe5e7eb);
    colors.ring = c(0x2563eb);
    colors.popover = c(0xffffff);
    colors.popover_foreground = c(0x111827);
    colors.muted = c(0xf1f5f9);
    colors.muted_foreground = c(0x6b7280);
    colors.accent = c(0xe8efff);
    colors.accent_foreground = c(0x2563eb);
    colors.primary = c(0x2563eb);
    colors.primary_foreground = Hsla::from(if light { rgb(0xffffff) } else { rgb(0x06101c) });
    colors.primary_hover = c(0x1d4ed8);
    colors.primary_active = c(0x1e40af);
    colors.secondary = c(0xf1f5f9);
    colors.secondary_foreground = c(0x111827);
    colors.button = c(0xffffff);
    colors.button_foreground = c(0x111827);
    colors.button_hover = c(0xf1f5f9);
    colors.button_active = c(0xe8efff);
    colors.list = c(0xffffff);
    colors.list_hover = c(0xf8fafc);
    colors.list_active = c(0xe8efff);
    colors.list_active_border = c(0x2563eb);
    colors.table = c(0xffffff);
    colors.table_even = c(0xf8fafc);
    colors.table_head = c(0xffffff);
    colors.table_head_foreground = c(0x111827);
    colors.table_foot = c(0xffffff);
    colors.table_foot_foreground = c(0x111827);
    colors.table_hover = c(0xf8fafc);
    colors.table_active = c(0xe8efff);
    colors.table_active_border = c(0x2563eb);
    colors.table_row_border = c(0xe5e7eb);
    colors.selection = Hsla::from(palette(0x2563eb, light)).alpha(0.25);
    colors.danger = c(0xdc2626);
    colors.danger_foreground = Hsla::from(if light { rgb(0xffffff) } else { rgb(0x1a0b0b) });
    colors.danger_hover = c(0xb91c1c);
    colors.danger_active = c(0x991b1b);
    colors.success = c(0x15803d);
    colors.success_foreground = Hsla::from(if light { rgb(0xffffff) } else { rgb(0x07160b) });
    colors.success_hover = c(0x166534);
    colors.success_active = c(0x14532d);
    colors.warning = c(0xb45309);
    colors.warning_foreground = Hsla::from(if light { rgb(0xffffff) } else { rgb(0x1a1205) });
    colors.warning_hover = c(0x92400e);
    colors.warning_active = c(0x78350f);
    theme.radius = px(6.);
}

#[cfg(target_os = "windows")]
fn system_prefers_light() -> bool {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const RRF_RT_REG_DWORD: u32 = 0x10;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            hkey: isize,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            value_type: *mut u32,
            data: *mut c_void,
            data_size: *mut u32,
        ) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let sub_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut data = 0u32;
    let mut data_size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut data_size,
        )
    };
    status != 0 || data == 1
}

#[cfg(target_os = "macos")]
fn system_prefers_light() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()
        .map(|output| {
            !String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("dark")
        })
        .unwrap_or(true)
}

#[cfg(target_os = "linux")]
fn system_prefers_light() -> bool {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let value = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            if value.contains("prefer-dark") {
                return false;
            }
            if value.contains("prefer-light") || value.contains("default") {
                return true;
            }
        }
    }
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            return !String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("dark");
        }
    }
    true
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn system_prefers_light() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::palette;

    #[test]
    fn palette_keeps_light_values_and_maps_dark() {
        assert_eq!(palette(0xf3f5f9, true), gpui_kit::rgb(0xf3f5f9));
        assert_eq!(palette(0xf3f5f9, false), gpui_kit::rgb(0x0b1220));
        assert_eq!(palette(0x123456, false), gpui_kit::rgb(0x123456));
    }
}
