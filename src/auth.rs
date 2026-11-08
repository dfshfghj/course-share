use crate::http::{redact_url, Http};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use reqwest::Method;
use url::Url;

pub const COURSE_HOME: &str = "https://course.pku.edu.cn/";
pub const COURSE_PORTAL: &str =
    "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";
pub const SSO_LOGIN_URL: &str =
    "https://course.pku.edu.cn/webapps/bb-sso-BBLEARN/login.html";
pub const IAAA_BASE: &str = "https://iaaa.pku.edu.cn/iaaa";
pub const IAAA_CAPTCHA_URL: &str = "https://iaaa.pku.edu.cn/iaaa/servlet/DrawServlet";
pub const DEFAULT_CALLBACK: &str =
    "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
pub const PASSPORT_CALLBACK: &str = "https://passport.pku.edu.cn/";
pub const PKU_DOMAIN: &str = "pku.edu.cn";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationKind {
    Captcha,
    Sms,
    Otp,
}

impl VerificationKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Captcha => "图形验证码",
            Self::Sms => "短信验证码",
            Self::Otp => "手机令牌",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Self::Captcha => "输入图片中的验证码",
            Self::Sms => "输入短信验证码",
            Self::Otp => "输入手机令牌 / OTP",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoginChallenge {
    pub kind: VerificationKind,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
struct Codes {
    rand: String,
    sms: String,
    otp: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginTarget {
    Blackboard,
    Haoxue,
}

#[derive(Clone)]
pub struct PendingLogin {
    http: Http,
    target: LoginTarget,
    appid: String,
    redirect_url: String,
    username: String,
    encrypted_password: String,
    codes: Codes,
    pub challenge: LoginChallenge,
}

pub enum LoginOutcome {
    Success(Http),
    Verification(PendingLogin),
    Failed(String),
}

/// Start login through the "校园卡用户" SSO entry, which redirects to IAAA.
pub fn begin_login(
    username: &str,
    password: &str,
    cookie_path: impl Into<std::path::PathBuf>,
) -> Result<LoginOutcome> {
    let http = Http::new(cookie_path)?;
    if http.cookie_count() > 0 && validate_blackboard(&http) {
        return Ok(LoginOutcome::Success(http));
    }

    let mut page = http
        .get_follow(COURSE_HOME, 15)
        .context("打开教学网首页")?;
    if !is_iaaa_entry(&page.body, page.final_url.host_str()) {
        if let Ok(sso_page) = http.get_follow(SSO_LOGIN_URL, 15) {
            page = sso_page;
        }
    }
    if !is_iaaa_entry(&page.body, page.final_url.host_str()) {
        return Ok(LoginOutcome::Failed(
            "教学网返回了 Blackboard 用户名/图形验证码页面，未能进入 IAAA“校园卡用户”入口。\
             请稍后重试，或改用 Cookie 导入。"
                .to_string(),
        ));
    }

    let (appid, redirect_url, encrypted_password) =
        prepare_iaaa(&http, &page, password, DEFAULT_CALLBACK, false)?;
    let pending = PendingLogin {
        http,
        target: LoginTarget::Blackboard,
        appid,
        redirect_url,
        username: username.to_string(),
        encrypted_password,
        codes: Codes::default(),
        challenge: LoginChallenge {
            kind: VerificationKind::Captcha,
            message: String::new(),
        },
    };
    submit_pending(pending)
}

/// Start the passport/IAAA login used by the Haoxue course API. The same
/// username/password is used as the teaching-network login, but the callback
/// establishes `_token2`/`_token` instead of a Blackboard session.
pub fn begin_haoxue_login(
    username: &str,
    password: &str,
    cookie_path: impl Into<std::path::PathBuf>,
) -> Result<LoginOutcome> {
    let http = Http::new(cookie_path)?;
    if crate::haoxue::session_valid(&http) {
        crate::haoxue::normalize_cookies(&http);
        return Ok(LoginOutcome::Success(http));
    }

    // Do not follow redirects here. When the teaching-network/IAAA session is
    // already valid, passport answers this entry with a 302 that sets
    // _token/_token2; following it bounces between "/" and the login page.
    let response = http
        .get_with_headers(crate::haoxue::LOGIN_ENTRY, &[])
        .context("打开好学 passport 登录页")?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response.text().context("读取好学 passport 登录页")?;
    if crate::haoxue::session_valid(&http) {
        crate::haoxue::normalize_cookies(&http);
        let _ = http.save_cookies();
        return Ok(LoginOutcome::Success(http));
    }
    if !body.contains("iaaa/oauth.jsp") {
        return Ok(LoginOutcome::Failed(format!(
            "好学 passport 返回 HTTP {status}{}，未建立好学会话，也没有 IAAA 登录表单。\
             请确认教学网账号密码登录仍然有效。",
            if location.is_empty() {
                String::new()
            } else {
                format!("（Location: {location}）")
            }
        )));
    }
    let page = crate::http::Fetched {
        status,
        final_url: Url::parse(crate::haoxue::LOGIN_ENTRY)
            .context("parse passport login URL")?,
        body,
    };
    if username.trim().is_empty() || password.is_empty() {
        return Ok(LoginOutcome::Failed(
            "好学会话未建立，passport 需要 IAAA 账号密码登录。\
             请使用学号密码登录教学网后重试。"
                .to_string(),
        ));
    }
    let (appid, redirect_url, encrypted_password) =
        prepare_iaaa(&http, &page, password, PASSPORT_CALLBACK, true)?;
    let pending = PendingLogin {
        http,
        target: LoginTarget::Haoxue,
        appid,
        redirect_url,
        username: username.to_string(),
        encrypted_password,
        codes: Codes::default(),
        challenge: LoginChallenge {
            kind: VerificationKind::Captcha,
            message: String::new(),
        },
    };
    submit_pending(pending)
}

/// Submit one verification code using the same IAAA session and accumulated
/// fields, so SMS + captcha can be requested in sequence.
pub fn resume_login(mut pending: PendingLogin, code: String) -> Result<LoginOutcome> {
    let code = code.trim().to_string();
    match pending.challenge.kind {
        VerificationKind::Captcha => pending.codes.rand = code,
        VerificationKind::Sms => pending.codes.sms = code,
        VerificationKind::Otp => pending.codes.otp = code,
    }
    submit_pending(pending)
}

pub fn send_sms_code(pending: &PendingLogin) -> Result<String> {
    let mut url = Url::parse(&format!("{IAAA_BASE}/sendSMSCode.do"))
        .context("build sendSMSCode URL")?;
    url.query_pairs_mut()
        .append_pair("userName", &pending.username)
        .append_pair("appId", &pending.appid)
        .append_pair("_rand", &now_millis().to_string());
    let value = pending
        .http
        .get_json(
            url.as_str(),
            &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Referer", "https://iaaa.pku.edu.cn/iaaa/oauth.jsp"),
            ],
        )
        .context("调用 IAAA sendSMSCode.do")?;
    if truthy(value.get("success")) {
        Ok(value
            .get("mobileMask")
            .and_then(|v| v.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("手机")
            .to_string())
    } else {
        Err(anyhow!(
            "{}",
            value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("发送短信验证码失败")
        ))
    }
}

pub fn fetch_captcha(pending: &PendingLogin, path: &Path) -> Result<()> {
    let url = format!("{IAAA_CAPTCHA_URL}?Rand={}", now_millis());
    let bytes = pending
        .http
        .get_bytes_follow(&url)
        .context("获取 IAAA 图形验证码")?;
    if bytes.is_empty() {
        return Err(anyhow!("验证码图片为空"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

pub fn login_with_cookie(
    header: &str,
    cookie_path: impl Into<std::path::PathBuf>,
) -> Result<Http> {
    let http = Http::new(cookie_path)?;
    http.import_cookie_header(header, PKU_DOMAIN)?;
    if !validate_blackboard(&http) {
        return Err(anyhow!(
            "Cookie 无效或已过期：未能读取教学网课程页。请重新复制 course.pku.edu.cn 的 Cookie。"
        ));
    }
    Ok(http)
}

pub fn validate_blackboard(http: &Http) -> bool {
    for url in [
        format!("{COURSE_PORTAL}?tab_tab_group_id=_2_1"),
        COURSE_HOME.to_string(),
    ] {
        let Ok(response) = http.get_follow(&url, 10) else {
            continue;
        };
        if !response.is_success() {
            continue;
        }
        let host = response.final_url.host_str().unwrap_or("");
        if host.contains("iaaa.pku.edu.cn") {
            continue;
        }
        if looks_logged_in(&response.body) {
            return true;
        }
    }
    false
}

fn looks_logged_in(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let login_page = lower.contains("loginwithcaptcha")
        || lower.contains("getcaptcha")
        || lower.contains("pwd1")
        || lower.contains("user_id");
    if login_page {
        return false;
    }
    body.contains("PkId{key=")
        || body.contains("courseListing")
        || body.contains("courseMenuPalette")
        || body.contains("欢迎,")
        || body.contains("欢迎，")
        || lower.contains("welcome,")
        || lower.contains("logout-link")
        || lower.contains("global-nav-bar")
        || lower.contains("bb-session")
}

fn is_iaaa_entry(body: &str, host: Option<&str>) -> bool {
    host == Some("iaaa.pku.edu.cn")
        || body.contains("iaaa/oauth.jsp")
        || body.contains("oauthlogin.do")
        || body.contains("user_name")
        || body.contains("valid_code")
}

fn prepare_iaaa(
    http: &Http,
    page: &crate::http::Fetched,
    password: &str,
    default_redirect: &str,
    force_oauth: bool,
) -> Result<(String, String, String)> {
    let hidden = hidden_inputs(&page.body);
    let mut appid = get_ci(&hidden, "appID")
        .or_else(|| get_ci(&hidden, "appid"))
        .or_else(|| js_hidden_value(&page.body, "appID"))
        .or_else(|| js_hidden_value(&page.body, "appid"))
        .unwrap_or_default();
    let mut redirect_url = get_ci(&hidden, "redirectUrl")
        .or_else(|| get_ci(&hidden, "redirUrl"))
        .or_else(|| js_hidden_value(&page.body, "redirectUrl"))
        .unwrap_or_else(|| default_redirect.to_string());
    let app_name = get_ci(&hidden, "appName")
        .or_else(|| js_hidden_value(&page.body, "appName"))
        .unwrap_or_else(|| "1".to_string());

    if force_oauth || page.body.contains("iaaa/oauth.jsp") {
        let response = http
            .post_form(
                &format!("{IAAA_BASE}/oauth.jsp"),
                &[
                    ("appName", app_name.as_str()),
                    ("appID", appid.as_str()),
                    ("redirectUrl", redirect_url.as_str()),
                ],
            )
            .context("POST IAAA oauth.jsp")?;
        let status = response.status();
        let body = response.text().context("读取 IAAA 登录页")?;
        if !status.is_success() {
            return Err(anyhow!(
                "IAAA oauth.jsp 返回 HTTP {}",
                status.as_u16()
            ));
        }
        let hidden = hidden_inputs(&body);
        appid = get_ci(&hidden, "appid")
            .or_else(|| get_ci(&hidden, "appID"))
            .unwrap_or(appid);
        redirect_url = get_ci(&hidden, "redirUrl")
            .or_else(|| get_ci(&hidden, "redirectUrl"))
            .unwrap_or(redirect_url);
    }

    if appid.is_empty() {
        appid = "blackboard".to_string();
    }
    if redirect_url.is_empty() {
        redirect_url = default_redirect.to_string();
    }
    let encrypted_password = rsa_encrypt_password(http, password)?;
    Ok((appid, redirect_url, encrypted_password))
}

fn submit_pending(mut pending: PendingLogin) -> Result<LoginOutcome> {
    let url = format!("{IAAA_BASE}/oauthlogin.do");
    let form = vec![
        ("appid".to_string(), pending.appid.clone()),
        ("userName".to_string(), pending.username.clone()),
        ("password".to_string(), pending.encrypted_password.clone()),
        ("randCode".to_string(), pending.codes.rand.clone()),
        ("smsCode".to_string(), pending.codes.sms.clone()),
        ("otpCode".to_string(), pending.codes.otp.clone()),
        ("remTrustChk".to_string(), "false".to_string()),
        ("redirUrl".to_string(), pending.redirect_url.clone()),
    ];
    let response = pending
        .http
        .request(
            Method::POST,
            &url,
            Some(form),
            &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Referer", "https://iaaa.pku.edu.cn/iaaa/oauth.jsp"),
                ("Origin", "https://iaaa.pku.edu.cn"),
            ],
        )
        .context("POST IAAA oauthlogin.do")?;
    let status = response.status();
    let body = response.text().context("读取 IAAA 登录响应")?;
    if !status.is_success() {
        return Ok(LoginOutcome::Failed(format!(
            "IAAA 登录接口返回 HTTP {}",
            status.as_u16()
        )));
    }

    let success = serde_json::from_str::<Value>(&body)
        .ok()
        .map(|value| truthy(value.get("success")));
    if success == Some(true) {
        let Some(token) = extract_token(&body) else {
            return Ok(LoginOutcome::Failed(
                "IAAA 返回成功但没有 token".to_string(),
            ));
        };
        let redirect_target = json_string(&body, "redirectUrl")
            .or_else(|| json_string(&body, "redirUrl"))
            .unwrap_or_else(|| pending.redirect_url.clone());
        if pending.target == LoginTarget::Haoxue {
            // The reference implementation deliberately does not follow this
            // redirect: passport sets _token/_token2 on the 302 response, and
            // following it can bounce between "/" and the login page.
            let callback = match append_token(PASSPORT_CALLBACK, &token) {
                Ok(url) => match pending.http.get_with_headers(
                    url.as_str(),
                    &[("Referer", "https://iaaa.pku.edu.cn/iaaa/oauth.jsp")],
                ) {
                    Ok(response) => format!(
                        "HTTP {}",
                        response.status().as_u16()
                    ),
                    Err(error) => format!("{error:#}"),
                },
                Err(error) => format!("invalid callback target: {error:#}"),
            };
            if crate::haoxue::session_valid(&pending.http) {
                crate::haoxue::normalize_cookies(&pending.http);
                let _ = pending.http.save_cookies();
                return Ok(LoginOutcome::Success(pending.http));
            }
            return Ok(LoginOutcome::Failed(format!(
                "IAAA 登录成功，但 passport 回调未建立好学会话（{callback}）。\
                 请重试；若仍失败，可改用 Cookie 导入。"
            )));
        }

        let callback = match append_token(&redirect_target, &token) {
            Ok(url) => match pending.http.get_follow_with_headers(
                url.as_str(),
                &[("Referer", "https://iaaa.pku.edu.cn/iaaa/oauth.jsp")],
                15,
            ) {
                Ok(fetched) => format!(
                    "HTTP {}, final={}, body={}",
                    fetched.status,
                    redact_url(&fetched.final_url),
                    body_snippet(&fetched.body)
                ),
                Err(error) => format!("{error:#}"),
            },
            Err(error) => format!("invalid callback target: {error:#}"),
        };
        if validate_blackboard(&pending.http) {
            let _ = pending.http.save_cookies();
            return Ok(LoginOutcome::Success(pending.http));
        }
        return Ok(LoginOutcome::Failed(format!(
            "IAAA 登录成功，但 campusLogin 回调未建立教学网会话（{}）。\
             请重试；若仍失败，可改用 Cookie 导入。",
            callback
        )));
    }

    if let Some((kind, message)) = classify_failure(&body) {
        pending.challenge = LoginChallenge { kind, message };
        return Ok(LoginOutcome::Verification(pending));
    }

    Ok(LoginOutcome::Failed(login_error_message(&body)))
}

fn append_token(target: &str, token: &str) -> Result<Url> {
    let mut url = Url::parse(target).or_else(|_| {
        Url::parse(COURSE_HOME)
            .context("parse course home")?
            .join(target)
            .with_context(|| format!("bad callback URL {target}"))
    })?;
    if url.scheme() == "http"
        && url
            .host_str()
            .map(|host| host.to_ascii_lowercase().ends_with("pku.edu.cn"))
            .unwrap_or(false)
    {
        let _ = url.set_scheme("https");
        if url.port() == Some(80) {
            let _ = url.set_port(None);
        }
    }
    url.query_pairs_mut()
        .append_pair("_rand", &now_millis().to_string())
        .append_pair("token", token);
    Ok(url)
}

fn classify_failure(body: &str) -> Option<(VerificationKind, String)> {
    let value: Value = serde_json::from_str(body).ok()?;
    if truthy(value.get("success")) {
        return None;
    }
    let errors = value.get("errors");
    let code = errors
        .and_then(|e| e.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message = errors
        .and_then(|e| e.get("msg"))
        .and_then(|v| v.as_str())
        .or_else(|| value.get("errMsg").and_then(|v| v.as_str()))
        .or_else(|| value.get("message").and_then(|v| v.as_str()))
        .or_else(|| value.get("msg").and_then(|v| v.as_str()))
        .unwrap_or("");
    let show_code = truthy(value.get("showCode"));

    let kind = match code {
        "E03" => Some(VerificationKind::Captcha),
        "E04" => Some(VerificationKind::Sms),
        "E05" => Some(VerificationKind::Otp),
        "E01" if show_code => Some(VerificationKind::Captcha),
        _ => None,
    }
    .or_else(|| {
        let lower = message.to_ascii_lowercase();
        if message.contains("短信") || lower.contains("sms") {
            Some(VerificationKind::Sms)
        } else if message.contains("令牌") || lower.contains("otp") {
            Some(VerificationKind::Otp)
        } else if message.contains("验证码") || lower.contains("captcha") {
            Some(VerificationKind::Captcha)
        } else if show_code {
            Some(VerificationKind::Captcha)
        } else {
            None
        }
    })?;

    let message = if message.is_empty() {
        default_challenge_message(&kind).to_string()
    } else {
        message.to_string()
    };
    Some((kind, message))
}

fn body_snippet(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(160).collect()
}

fn js_hidden_value(body: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"(?i)name=['"]?{}['"]?\s+value=['"]([^'"]*)['"]"#,
        regex::escape(name)
    );
    let regex = regex::Regex::new(&pattern).ok()?;
    regex
        .captures(body)
        .and_then(|captures| captures.get(1).map(|m| m.as_str().to_string()))
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value == "true" || value == "1",
        Some(Value::Number(value)) => value.as_i64() == Some(1),
        _ => false,
    }
}

fn default_challenge_message(kind: &VerificationKind) -> &'static str {
    match kind {
        VerificationKind::Captcha => "请输入图形验证码",
        VerificationKind::Sms => "请输入短信验证码",
        VerificationKind::Otp => "请输入手机令牌",
    }
}

fn login_error_message(body: &str) -> String {
    if let Some(message) = json_error(body) {
        return message;
    }
    let compact = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.is_empty() {
        "IAAA 登录失败，未返回错误信息".to_string()
    } else {
        format!("IAAA 登录失败：{}", compact.chars().take(200).collect::<String>())
    }
}

fn rsa_encrypt_password(http: &Http, password: &str) -> Result<String> {
    let url = format!("{IAAA_BASE}/getPublicKey.do");
    let response = http
        .get_with_headers(&url, &[("X-Requested-With", "XMLHttpRequest")])
        .context("GET IAAA public key")?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "获取 IAAA 公钥失败：HTTP {}",
            response.status().as_u16()
        ));
    }
    let body = response.text().context("读取 IAAA 公钥")?;
    let key = json_string(&body, "key")
        .or_else(|| extract_quoted(&body, "key"))
        .ok_or_else(|| anyhow!("IAAA 公钥响应中没有 key 字段"))?;
    let pem = if key.contains("BEGIN PUBLIC KEY") {
        key
    } else {
        format!("-----BEGIN PUBLIC KEY-----\n{key}\n-----END PUBLIC KEY-----")
    };
    let public_key = RsaPublicKey::from_public_key_pem(&pem).context("解析 IAAA RSA 公钥")?;
    let encrypted = public_key
        .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, password.as_bytes())
        .context("RSA 加密密码")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

fn hidden_inputs(html: &str) -> HashMap<String, String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("input[name]").unwrap();
    let mut values = HashMap::new();
    for element in document.select(&selector) {
        let value = element.value();
        if let Some(name) = value.attr("name") {
            values.insert(
                name.to_string(),
                value.attr("value").unwrap_or("").to_string(),
            );
        }
    }
    values
}

fn get_ci(map: &HashMap<String, String>, key: &str) -> Option<String> {
    map.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.clone())
}

fn extract_token(body: &str) -> Option<String> {
    json_string(body, "token").or_else(|| extract_quoted(body, "token"))
}

fn json_string(body: &str, key: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

fn json_error(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    if let Some(errors) = value.get("errors") {
        for key in ["msg", "message", "errMsg", "err", "error"] {
            if let Some(text) = errors.get(key).and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    for key in ["errMsg", "message", "msg", "err", "error"] {
        if let Some(text) = value.get(key).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn extract_quoted(body: &str, key: &str) -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(String, regex::Regex)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().ok()?;
    if cache.as_ref().map(|(k, _)| k.as_str()) != Some(key) {
        let pattern = format!(
            r#"(?i)["']?{}["']?\s*[:=]\s*["']([^"']+)["']"#,
            regex::escape(key)
        );
        *cache = Some((key.to_string(), regex::Regex::new(&pattern).ok()?));
    }
    let regex = &cache.as_ref()?.1;
    regex
        .captures(body)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hidden_inputs_case_insensitively() {
        let html = r#"<html><body>
            <input name="appID" value="test-app">
            <input name="redirectUrl" value="https://course.pku.edu.cn/callback">
            <input name="other" value="x">
        </body></html>"#;
        let hidden = hidden_inputs(html);
        assert_eq!(get_ci(&hidden, "appid").unwrap(), "test-app");
        assert_eq!(get_ci(&hidden, "APPID").unwrap(), "test-app");
        assert_eq!(
            get_ci(&hidden, "redirecturl").unwrap(),
            "https://course.pku.edu.cn/callback"
        );
    }

    #[test]
    fn extracts_token_from_json_and_script() {
        assert_eq!(
            extract_token(r#"{"success":true,"token":"abc.def.ghi"}"#).unwrap(),
            "abc.def.ghi"
        );
        assert_eq!(
            extract_token(r#"<script>var token = "xyz";</script>"#).unwrap(),
            "xyz"
        );
    }

    #[test]
    fn reports_login_error_message() {
        let message = json_error(r#"{"success":false,"errors":{"code":"E01","msg":"用户名或密码错误"}}"#);
        assert_eq!(message.unwrap(), "用户名或密码错误");
    }

    #[test]
    #[ignore]
    fn live_sso_entry_prepares_iaaa_login() {
        let path = std::env::temp_dir().join(format!(
            "course-share-sso-test-{}.json",
            std::process::id()
        ));
        let http = Http::new(&path).unwrap();
        let sso = http.get_follow(SSO_LOGIN_URL, 15).unwrap();
        assert!(sso.is_success());
        assert!(sso.body.contains("iaaa/oauth.jsp"));
        let (appid, redirect, encrypted) =
            prepare_iaaa(&http, &sso, "dummy-password", DEFAULT_CALLBACK, false).unwrap();
        assert_eq!(appid, "blackboard");
        assert!(redirect.contains("campusLogin"));
        assert!(!encrypted.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore]
    fn live_passport_entry_prepares_iaaa_login() {
        let path = std::env::temp_dir().join(format!(
            "course-share-passport-test-{}.json",
            std::process::id()
        ));
        let http = Http::new(&path).unwrap();
        let page = http
            .get_follow(crate::haoxue::LOGIN_ENTRY, 15)
            .unwrap();
        assert!(page.is_success());
        assert!(page.body.contains("iaaa/oauth.jsp"));
        let (appid, redirect, encrypted) =
            prepare_iaaa(&http, &page, "dummy-password", PASSPORT_CALLBACK, true).unwrap();
        assert_eq!(appid, "resource");
        assert!(redirect.contains("passport.pku.edu.cn"));
        assert!(!encrypted.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn recognizes_logged_in_portal_home() {
        assert!(looks_logged_in(
            r#"<title>欢迎, 张三 &ndash; Blackboard Learn</title><a class="logout-link">退出</a>"#
        ));
        assert!(looks_logged_in(r#"<ul id="courseMenuPalette_contents"></ul>"#));
    }

    #[test]
    fn rejects_blackboard_captcha_login_page() {
        assert!(!looks_logged_in(
            r#"<form action="/webapps/bb-sso-BBLEARN/execute/authValidate/loginwithcaptcha"><input id="user_id" name="user_id"><input type="password" name="pwd1"><img src="/getCaptcha"></form>"#
        ));
        assert!(!looks_logged_in("<title>Blackboard Learn</title><p>欢迎使用</p>"));
    }

    #[test]
    fn callback_upgrades_pku_http_to_https() {
        let url = append_token(
            "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin",
            "test.token",
        )
        .unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(url.query().unwrap_or("").contains("token=test.token"));
    }

    #[test]
    fn parses_js_document_write_hidden_values() {
        let html = r#"<script>
            document.write("<input type=hidden name='appID' value='blackboard'>");
            document.write("<input type=hidden name='redirectUrl' value='http://course.pku.edu.cn/campusLogin'>");
        </script>"#;
        assert_eq!(js_hidden_value(html, "appID").unwrap(), "blackboard");
        assert_eq!(
            js_hidden_value(html, "redirectUrl").unwrap(),
            "http://course.pku.edu.cn/campusLogin"
        );
    }

    #[test]
    fn classifies_verification_challenges() {
        assert_eq!(
            classify_failure(r#"{"success":false,"errors":{"code":"E03","msg":"验证码错误"},"showCode":true}"#)
                .unwrap()
                .0,
            VerificationKind::Captcha
        );
        assert_eq!(
            classify_failure(r#"{"success":false,"errors":{"code":"E04","msg":"短信验证码错误或已过期"}}"#)
                .unwrap()
                .0,
            VerificationKind::Sms
        );
        assert_eq!(
            classify_failure(r#"{"success":false,"errors":{"code":"E05","msg":"手机令牌错误或已过期"}}"#)
                .unwrap()
                .0,
            VerificationKind::Otp
        );
        assert_eq!(
            classify_failure(r#"{"success":false,"errors":{"code":"E01","msg":"用户名或密码错误"},"showCode":true}"#)
                .unwrap()
                .0,
            VerificationKind::Captcha
        );
        assert!(classify_failure(r#"{"success":false,"errors":{"code":"E01","msg":"用户名或密码错误"},"showCode":false}"#).is_none());
    }
}
