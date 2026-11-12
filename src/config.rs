use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use chrono::NaiveTime;
use toml::Value;

use crate::crypto::decode_local_key;

/// 邮箱投递方式：`smtp` 真发（生产默认），`log` 只写应用日志（测试服专用）。
/// log 模式把收件人+验证码打到日志（带 `[email:log-only]` 标记），不连 SMTP，
/// 方便在测试服走通邮箱 OTP 登录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmailDelivery {
    #[default]
    Smtp,
    Log,
}

impl EmailDelivery {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_lowercase().as_str() {
            "smtp" => Ok(Self::Smtp),
            "log" => Ok(Self::Log),
            other => Err(format!(
                "email.delivery 非法: {other}（只能是 smtp 或 log）"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub super_username: String,
    pub super_password: String,
    pub webauthn_rp_id: String,
    pub webauthn_origin: String,
    pub webauthn_rp_name: String,
    pub email_enabled: bool,
    pub email_delivery: EmailDelivery,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_from: String,
    pub lock_time: NaiveTime,
    pub breakfast_after: NaiveTime,
    pub lunch_after: NaiveTime,
    pub dinner_after: NaiveTime,
    pub mean_threshold: f64,
    pub min_samples: i64,
    pub local_key: [u8; 32],
    pub dish_image_dir: String,
    pub dish_image_max_bytes: u64,
    pub database_url: String,
    pub bind: SocketAddr,
    /// 管理回环端口：只挂 /readyz。缺省只绑回环（外部到不了），边界靠绑定地址实现。
    pub admin_bind: SocketAddr,
    pub cookie_secure: bool,
    pub trust_proxy: bool,
    pub static_dir: Option<String>,
    /// `/app/*` 直出的 APK 目录。缺省为 `{static_dir}/app`（生产即此）；
    /// staging 经 `DLZ_APP_DIR` 指向镜像里另一份（如 `/app/static/app-staging`）。
    pub app_dir: Option<String>,
    /// 启动自动签名（A 方案）：进程就绪后若镜像里是未签包且库里已有 keystore，
    /// 自动签名发布。缺省 true；`DLZ_APK_AUTO_SIGN=false` 可关。
    pub apk_auto_sign: bool,
}

impl AppConfig {
    pub fn load_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let raw =
            fs::read_to_string(path.as_ref()).map_err(|err| format!("无法读取配置: {err}"))?;
        Self::parse_with_env(&raw, &|key| std::env::var(key).ok())
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        // 历史行为：纯 TOML，不读环境（单测/e2e 预置走这里）。
        Self::parse_with_env(raw, &|_| None)
    }

    /// 生产入口：TOML 先行，敏感与域名类字段可用 `DLZ_*` 环境变量覆盖，
    /// 密钥优先读 `*_FILE`（docker secret 挂载，如 /run/secrets/xxx）。
    pub fn parse_with_env(
        raw: &str,
        get: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut value: Value =
            toml::from_str(raw).map_err(|err| format!("配置不是合法 TOML: {err}"))?;
        apply_env_overrides(&mut value, get)?;
        Self::parse_value(&value)
    }

    fn parse_value(value: &Value) -> Result<Self, String> {
        let mut missing = Vec::new();

        let super_admin = table(value, "super_admin", &mut missing);
        let webauthn = table(value, "webauthn", &mut missing);
        let email = table(value, "email", &mut missing);
        let smtp = table(value, "smtp", &mut missing);
        let schedule = table(value, "schedule", &mut missing);
        let score_alert = table(value, "score_alert", &mut missing);
        let crypto = table(value, "crypto", &mut missing);
        let dish = table(value, "dish_image", &mut missing);
        let database = table(value, "database", &mut missing);
        let server = value.get("server").and_then(Value::as_table).cloned();

        let super_username = req_string(
            super_admin,
            "super_admin.username",
            "username",
            &mut missing,
        );
        let super_password = req_string(
            super_admin,
            "super_admin.password",
            "password",
            &mut missing,
        );
        let rp_id = req_string(webauthn, "webauthn.rp_id", "rp_id", &mut missing);
        let rp_origin = req_string(webauthn, "webauthn.rp_origin", "rp_origin", &mut missing);
        let rp_name = optional_string(webauthn, "rp_name").unwrap_or_else(|| "多邻灶".to_string());
        let email_enabled = optional_bool(email, "enabled").unwrap_or(false);
        let email_delivery = match optional_string(email, "delivery") {
            None => EmailDelivery::default(),
            Some(raw) => EmailDelivery::parse(&raw)?,
        };

        let lock_time = req_time(schedule, "schedule.lock_time", "lock_time", &mut missing);
        let breakfast_after = req_time(
            schedule,
            "schedule.breakfast_after",
            "breakfast_after",
            &mut missing,
        );
        let lunch_after = req_time(
            schedule,
            "schedule.lunch_after",
            "lunch_after",
            &mut missing,
        );
        let dinner_after = req_time(
            schedule,
            "schedule.dinner_after",
            "dinner_after",
            &mut missing,
        );
        let mean_threshold = req_f64(
            score_alert,
            "score_alert.mean_threshold",
            "mean_threshold",
            &mut missing,
        );
        let min_samples = req_i64(
            score_alert,
            "score_alert.min_samples",
            "min_samples",
            &mut missing,
        );
        let key_raw = req_string(crypto, "crypto.local_key", "local_key", &mut missing);
        let local_key = match key_raw.as_deref().and_then(decode_local_key) {
            Some(key) => Some(key),
            None => {
                if key_raw.is_some() {
                    missing.push("crypto.local_key".into());
                }
                None
            }
        };
        let dish_image_dir = req_string(dish, "dish_image.dir", "dir", &mut missing);
        let dish_image_max_bytes = req_u64(dish, "dish_image.max_bytes", "max_bytes", &mut missing);
        let database_url = req_string(database, "database.url", "url", &mut missing);

        let mut smtp_host = optional_string(smtp, "host").unwrap_or_default();
        let smtp_port = optional_u16(smtp, "port").unwrap_or(0);
        let smtp_username = optional_string(smtp, "username").unwrap_or_default();
        let smtp_password = optional_string(smtp, "password").unwrap_or_default();
        let mut smtp_from = optional_string(smtp, "from").unwrap_or_default();
        // log 模式不连 SMTP，不校验 smtp 字段；只有真发才要求配齐。
        if email_enabled && email_delivery == EmailDelivery::Smtp {
            if smtp_host.is_empty() || is_placeholder(&smtp_host) {
                missing.push("smtp.host".into());
                smtp_host.clear();
            }
            if smtp_from.is_empty() || is_placeholder(&smtp_from) {
                missing.push("smtp.from".into());
                smtp_from.clear();
            }
            if smtp_port == 0 {
                missing.push("smtp.port".into());
            }
        }

        if !missing.is_empty() {
            missing.sort();
            missing.dedup();
            return Err(format!(
                "缺少或无效的配置: {}。钟点、阈值、样本量、图片上限和密钥必须由配置给出，代码不设默认值",
                missing.join(", ")
            ));
        }

        let server = server.as_ref();
        let bind = optional_string(server, "bind")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|_| "server.bind 不是合法地址".to_string())?;
        let admin_bind = optional_string(server, "admin_bind")
            .unwrap_or_else(|| "127.0.0.1:8090".into())
            .parse()
            .map_err(|_| "server.admin_bind 不是合法地址".to_string())?;
        // 生产默认 true：即使后端容器只听 HTTP（TLS 由 Traefik 等网关终结），
        // 浏览器也只在 HTTPS 下回送 Secure Cookie。本地 HTTP 直连/e2e 才显式配 false。
        let cookie_secure = optional_bool(server, "cookie_secure").unwrap_or(true);
        // 反代可信开关：缺省 false，直接取对端地址；只有确定前面有可信反代
        // （如 Traefik）剥掉一层并设置 X-Forwarded-For 时才配 true。
        let trust_proxy = optional_bool(server, "trust_proxy").unwrap_or(false);
        let static_dir = optional_string(server, "static_dir").filter(|s| !s.is_empty());
        let app_dir = optional_string(server, "app_dir").filter(|s| !s.is_empty());
        // 启动自动签名默认开（A 方案）：关了就全靠超管手动点「签名发布」。
        let apk_auto_sign = optional_bool(server, "apk_auto_sign").unwrap_or(true);

        Ok(Self {
            super_username: super_username.unwrap(),
            super_password: super_password.unwrap(),
            webauthn_rp_id: rp_id.unwrap(),
            webauthn_origin: rp_origin.unwrap(),
            webauthn_rp_name: rp_name,
            email_enabled,
            email_delivery,
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
            smtp_from,
            lock_time: lock_time.unwrap(),
            breakfast_after: breakfast_after.unwrap(),
            lunch_after: lunch_after.unwrap(),
            dinner_after: dinner_after.unwrap(),
            mean_threshold: mean_threshold.unwrap(),
            min_samples: min_samples.unwrap(),
            local_key: local_key.unwrap(),
            dish_image_dir: dish_image_dir.unwrap(),
            dish_image_max_bytes: dish_image_max_bytes.unwrap(),
            database_url: database_url.unwrap(),
            bind,
            admin_bind,
            cookie_secure,
            trust_proxy,
            static_dir,
            app_dir,
            apk_auto_sign,
        })
    }

    /// `/app/*` 的实际目录：显式配了 `[server] app_dir`（或 `DLZ_APP_DIR`）就用它，
    /// 否则回退 `{static_dir}/app`；都没配则无目录（路由返回 404）。
    pub fn resolved_app_dir(&self) -> Option<String> {
        if let Some(dir) = self.app_dir.clone().filter(|s| !s.is_empty()) {
            return Some(dir);
        }
        self.static_dir.clone().map(|dir| format!("{dir}/app"))
    }
}

/// `DLZ_*` 环境覆盖表：([TOML 路径], 环境变量名)。敏感项优先读 `<VAR>_FILE`
/// 指向的文件（docker secret），回退直读变量；域名类只读变量。
/// 缺 `_FILE` 文件即报错（fail closed），不静默回退，避免配错密钥还起库。
fn apply_env_overrides(
    value: &mut Value,
    get: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
    const RULES: [(&[&str], &str); 10] = [
        (&["server", "app_dir"], "DLZ_APP_DIR"),
        (&["server", "admin_bind"], "DLZ_ADMIN_BIND"),
        (&["server", "apk_auto_sign"], "DLZ_APK_AUTO_SIGN"),
        (&["email", "delivery"], "DLZ_EMAIL_DELIVERY"),
        (&["database", "url"], "DLZ_DATABASE_URL"),
        (&["super_admin", "password"], "DLZ_SUPER_PASSWORD"),
        (&["crypto", "local_key"], "DLZ_LOCAL_KEY"),
        (&["smtp", "password"], "DLZ_SMTP_PASSWORD"),
        (&["webauthn", "rp_id"], "DLZ_WEBAUTHN_RP_ID"),
        (&["webauthn", "rp_origin"], "DLZ_WEBAUTHN_ORIGIN"),
    ];
    for (path, var) in RULES {
        let Some(secret) = resolve_secret(get, var)? else {
            continue;
        };
        let mut node: &mut Value = &mut *value;
        for key in &path[..path.len() - 1] {
            node = node
                .get_mut(*key)
                .filter(|item| item.is_table())
                .ok_or_else(|| format!("环境覆盖 {var} 需要 [{table}] 表", table = path[0]))?;
        }
        // insert 而非 IndexMut：TOML 里缺该键正是常态（密钥不进文件）。
        node.as_table_mut()
            .ok_or_else(|| format!("环境覆盖 {var} 需要 [{table}] 表", table = path[0]))?
            .insert(path[path.len() - 1].to_string(), Value::String(secret));
    }
    Ok(())
}

fn resolve_secret(
    get: &dyn Fn(&str) -> Option<String>,
    var: &str,
) -> Result<Option<String>, String> {
    let file_key = format!("{var}_FILE");
    if let Some(path) = get(&file_key).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        let content = fs::read_to_string(&path)
            .map_err(|err| format!("读密钥文件失败 {path}: {err}"))?;
        let content = content.trim().to_string();
        if content.is_empty() {
            return Err(format!("密钥文件为空 {path}"));
        }
        return Ok(Some(content));
    }
    Ok(get(var)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

fn is_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed == "REPLACE_ME"
        || trimmed == "TODO"
        || trimmed == "CHANGEME"
        || trimmed == "请填写"
}

fn table<'a>(
    root: &'a Value,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<&'a toml::map::Map<String, Value>> {
    match root.get(key).and_then(Value::as_table) {
        Some(table) => Some(table),
        None => {
            missing.push(format!("[{key}]"));
            None
        }
    }
}

fn optional_string(table: Option<&toml::map::Map<String, Value>>, key: &str) -> Option<String> {
    table
        .and_then(|item| item.get(key))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn optional_bool(table: Option<&toml::map::Map<String, Value>>, key: &str) -> Option<bool> {
    let value = table.and_then(|item| item.get(key))?;
    // 环境覆盖统一按字符串塞进来（如 DLZ_APK_AUTO_SIGN=false），这里与
    // optional_u16 一样容忍字符串；非法值回 None（调用方取缺省）。
    match value {
        Value::Boolean(hit) => Some(*hit),
        Value::String(text) => match text.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn optional_u16(table: Option<&toml::map::Map<String, Value>>, key: &str) -> Option<u16> {
    let value = table.and_then(|item| item.get(key))?;
    match value {
        Value::Integer(number) if *number > 0 && *number <= u16::MAX as i64 => Some(*number as u16),
        Value::String(text) if !is_placeholder(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn req_string(
    table: Option<&toml::map::Map<String, Value>>,
    label: &str,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<String> {
    match table.and_then(|item| item.get(key)).and_then(Value::as_str) {
        Some(value) if !is_placeholder(value) => Some(value.trim().to_string()),
        _ => {
            missing.push(label.into());
            None
        }
    }
}

fn req_time(
    table: Option<&toml::map::Map<String, Value>>,
    label: &str,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<NaiveTime> {
    let Some(raw) = req_string(table, label, key, missing) else {
        return None;
    };
    match NaiveTime::parse_from_str(&raw, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(&raw, "%H:%M:%S"))
    {
        Ok(time) => Some(time),
        Err(_) => {
            missing.push(label.into());
            None
        }
    }
}

fn req_f64(
    table: Option<&toml::map::Map<String, Value>>,
    label: &str,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<f64> {
    let Some(table) = table else {
        return None;
    };
    let parsed = match table.get(key) {
        Some(Value::Float(value)) if value.is_finite() => Some(*value),
        Some(Value::Integer(value)) => Some(*value as f64),
        Some(Value::String(value)) if !is_placeholder(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    };
    match parsed.filter(|value| value.is_finite()) {
        Some(value) => Some(value),
        None => {
            missing.push(label.into());
            None
        }
    }
}

fn req_i64(
    table: Option<&toml::map::Map<String, Value>>,
    label: &str,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<i64> {
    let Some(table) = table else {
        return None;
    };
    let parsed = match table.get(key) {
        Some(Value::Integer(value)) if *value >= 0 => Some(*value),
        Some(Value::String(value)) if !is_placeholder(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    };
    match parsed.filter(|value| *value >= 0) {
        Some(value) => Some(value),
        None => {
            missing.push(label.into());
            None
        }
    }
}

fn req_u64(
    table: Option<&toml::map::Map<String, Value>>,
    label: &str,
    key: &str,
    missing: &mut Vec<String>,
) -> Option<u64> {
    let Some(table) = table else {
        return None;
    };
    let parsed = match table.get(key) {
        Some(Value::Integer(value)) if *value > 0 => Some(*value as u64),
        Some(Value::String(value)) if !is_placeholder(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    };
    match parsed.filter(|value| *value > 0) {
        Some(value) => Some(value),
        None => {
            missing.push(label.into());
            None
        }
    }
}

pub fn example_config() -> String {
    format!(
        r#"
[super_admin]
username = "root"
password = "root-password-1"

[webauthn]
rp_id = "localhost"
rp_origin = "http://localhost:8080"
rp_name = "多邻灶"

[email]
enabled = {email}
delivery = "smtp"

[smtp]
host = "127.0.0.1"
port = 2525
username = ""
password = ""
from = "noreply@example.com"

[schedule]
lock_time = "{lock}"
breakfast_after = "{breakfast}"
lunch_after = "{lunch}"
dinner_after = "{dinner}"

[score_alert]
mean_threshold = "{threshold}"
min_samples = "{samples}"

[crypto]
local_key = "{key}"

[dish_image]
dir = "{dir}"
max_bytes = "{max_bytes}"

[server]
bind = "127.0.0.1:0"
cookie_secure = false
trust_proxy = false

[database]
url = "postgres://ubuntu@localhost/duolinzao_test"
"#,
        email = "false",
        lock = "15:04",
        breakfast = "08:30",
        lunch = "12:30",
        dinner = "18:30",
        threshold = "0",
        samples = "2",
        key = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
        dir = "/tmp/duolinzao-images",
        max_bytes = "1048576",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_file_refuses_to_start() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml");
        let err = AppConfig::load_path(path).expect_err("placeholder config must fail");
        assert!(err.contains("schedule.lock_time"), "{err}");
        assert!(err.contains("score_alert.mean_threshold"), "{err}");
        assert!(err.contains("score_alert.min_samples"), "{err}");
        assert!(err.contains("dish_image.max_bytes"), "{err}");
        assert!(err.contains("crypto.local_key"), "{err}");
    }

    #[test]
    fn email_off_does_not_require_smtp() {
        let mut raw = example_config();
        raw = raw.replace("host = \"127.0.0.1\"", "host = \"\"");
        raw = raw.replace("from = \"noreply@example.com\"", "from = \"\"");
        let config = AppConfig::parse(&raw).expect("email off");
        assert!(!config.email_enabled);
        assert_eq!(config.lock_time.to_string(), "15:04:00");
    }

    #[test]
    fn cookie_secure_defaults_to_true() {
        // 缺省即生产姿态；本地/e2e 在 example_config 里显式 false。
        let raw = example_config().replace("cookie_secure = false", "");
        let config = AppConfig::parse(&raw).expect("缺省应为 true");
        assert!(config.cookie_secure);
        let raw = example_config();
        let config = AppConfig::parse(&raw).expect("显式 false");
        assert!(!config.cookie_secure);
    }

    #[test]
    fn admin_bind_defaults_to_loopback() {
        let config = AppConfig::parse(&example_config()).expect("缺省回环");
        assert_eq!(config.admin_bind.to_string(), "127.0.0.1:8090");
        let raw = example_config().replace("[server]", "[server]\nadmin_bind = \"127.0.0.1:8190\"");
        let config = AppConfig::parse(&raw).expect("显式 admin_bind");
        assert_eq!(config.admin_bind.to_string(), "127.0.0.1:8190");
        let env = env_map(&[("DLZ_ADMIN_BIND", " 127.0.0.1:8191 ")]);
        let config =
            AppConfig::parse_with_env(&example_config(), &|k| env.get(k).cloned())
                .expect("env 覆盖");
        assert_eq!(config.admin_bind.to_string(), "127.0.0.1:8191");
    }

    #[test]
    fn app_dir_defaults_to_static_subdir_and_env_overrides() {
        // 缺省：未配 app_dir 时回退 {static_dir}/app（生产行为不变）。
        let raw = example_config().replace(
            "[server]",
            "[server]\nstatic_dir = \"/app/static\"",
        );
        let config = AppConfig::parse(&raw).expect("缺省 app_dir");
        assert_eq!(config.app_dir, None);
        assert_eq!(
            config.resolved_app_dir().as_deref(),
            Some("/app/static/app")
        );
        // TOML 显式 app_dir 优先于缺省。
        let raw = example_config().replace(
            "[server]",
            "[server]\nstatic_dir = \"/app/static\"\napp_dir = \"/app/static/app-staging\"",
        );
        let config = AppConfig::parse(&raw).expect("显式 app_dir");
        assert_eq!(
            config.resolved_app_dir().as_deref(),
            Some("/app/static/app-staging")
        );
        // DLZ_APP_DIR 环境覆盖（staging 经 stack 注入）。
        let env = env_map(&[("DLZ_APP_DIR", " /srv/app-dir ")]);
        let config =
            AppConfig::parse_with_env(&example_config(), &|k| env.get(k).cloned())
                .expect("env 覆盖");
        assert_eq!(config.app_dir.as_deref(), Some("/srv/app-dir"));
        assert_eq!(config.resolved_app_dir().as_deref(), Some("/srv/app-dir"));
        // 都没配：无目录（路由返回 404）。
        let config = AppConfig::parse(&example_config()).expect("无静态目录");
        assert_eq!(config.resolved_app_dir(), None);
    }

    #[test]
    fn apk_auto_sign_defaults_on_and_env_overrides() {
        // 缺省开：不配就是 A 方案（启动自检自动签名发布）。
        let config = AppConfig::parse(&example_config()).expect("缺省应开");
        assert!(config.apk_auto_sign);
        // TOML 显式关。
        let raw = example_config().replace("[server]", "[server]\napk_auto_sign = false");
        let config = AppConfig::parse(&raw).expect("显式 false");
        assert!(!config.apk_auto_sign);
        // DLZ_APK_AUTO_SIGN 环境覆盖（字符串也要能解析，这是 env 覆盖的统一形态）。
        let env = env_map(&[("DLZ_APK_AUTO_SIGN", " false ")]);
        let config =
            AppConfig::parse_with_env(&example_config(), &|k| env.get(k).cloned())
                .expect("env 关");
        assert!(!config.apk_auto_sign);
        let env = env_map(&[("DLZ_APK_AUTO_SIGN", "true")]);
        let config =
            AppConfig::parse_with_env(&example_config(), &|k| env.get(k).cloned())
                .expect("env 开");
        assert!(config.apk_auto_sign);
    }

    #[test]
    fn email_on_requires_smtp() {
        let raw = example_config().replace("enabled = false", "enabled = true");
        let mut broken = raw.replace("host = \"127.0.0.1\"", "host = \"\"");
        broken = broken.replace("from = \"noreply@example.com\"", "from = \"\"");
        let err = AppConfig::parse(&broken).expect_err("smtp required");
        assert!(err.contains("smtp.host"), "{err}");
    }

    #[test]
    fn email_log_delivery_skips_smtp_and_parses() {
        // 测试服模式：enabled + delivery=log 时 smtp 可空，OTP 照样可用。
        let raw = example_config()
            .replace("enabled = false", "enabled = true")
            .replace("delivery = \"smtp\"", "delivery = \"log\"")
            .replace("host = \"127.0.0.1\"", "host = \"\"")
            .replace("from = \"noreply@example.com\"", "from = \"\"")
            .replace("port = 2525", "port = 0");
        let config = AppConfig::parse(&raw).expect("log 模式不该要求 smtp");
        assert!(config.email_enabled);
        assert_eq!(config.email_delivery, EmailDelivery::Log);
        // 缺省仍是真发：生产行为不变。
        let fallback = AppConfig::parse(&example_config()).expect("example");
        assert_eq!(fallback.email_delivery, EmailDelivery::Smtp);
    }

    #[test]
    fn email_delivery_rejects_unknown_and_env_override() {
        let raw = example_config().replace("delivery = \"smtp\"", "delivery = \"sendmail\"");
        let err = AppConfig::parse(&raw).expect_err("未知投递方式必须报错");
        assert!(err.contains("email.delivery"), "{err}");
        // DLZ_EMAIL_DELIVERY 环境覆盖（staging 由 stack 注入 log）。
        let env = env_map(&[("DLZ_EMAIL_DELIVERY", " log ")]);
        let config =
            AppConfig::parse_with_env(&example_config(), &|k| env.get(k).cloned())
                .expect("env 覆盖");
        assert_eq!(config.email_delivery, EmailDelivery::Log);
    }

    fn env_map(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn env_overrides_sensitive_fields() {
        // TOML 留占位，环境变量注入真实值（含前后空白应被裁掉）。
        let raw = example_config()
            .replace("url = \"postgres://ubuntu@localhost/duolinzao_test\"", "url = \"REPLACE_ME\"")
            .replace("password = \"root-password-1\"", "password = \"REPLACE_ME\"");
        let env = env_map(&[
            ("DLZ_DATABASE_URL", "  postgres://u:p@db:5432/x  "),
            ("DLZ_SUPER_PASSWORD", "s3cret-pass"),
            ("DLZ_WEBAUTHN_RP_ID", "duolinzao.example.com"),
            ("DLZ_WEBAUTHN_ORIGIN", "https://duolinzao.example.com"),
        ]);
        let config =
            AppConfig::parse_with_env(&raw, &|k| env.get(k).cloned()).expect("env 覆盖");
        assert_eq!(config.database_url, "postgres://u:p@db:5432/x");
        assert_eq!(config.super_password, "s3cret-pass");
        assert_eq!(config.webauthn_rp_id, "duolinzao.example.com");
        assert_eq!(config.webauthn_origin, "https://duolinzao.example.com");
    }

    #[test]
    fn deploy_config_needs_secrets_from_env() {
        // deploy/config.toml 只放非敏感值；密钥缺了必须拒绝启动，齐了必须全绿。
        // 部署改了任一必填还测得过去，这里先炸。
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/config.toml");
        let raw = std::fs::read_to_string(path).expect("deploy config 存在");
        let err = AppConfig::parse_with_env(&raw, &|_| None).expect_err("缺密钥必须拒起");
        for key in ["database.url", "super_admin.password", "crypto.local_key"] {
            assert!(err.contains(key), "{key} 缺失时应报错，实际：{err}");
        }
        let env = env_map(&[
            ("DLZ_DATABASE_URL", "postgres://u:p@db:5432/x"),
            ("DLZ_SUPER_PASSWORD", "prod-pass-1"),
            ("DLZ_LOCAL_KEY", "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY="),
            ("DLZ_WEBAUTHN_RP_ID", "duolinzao.example.com"),
            ("DLZ_WEBAUTHN_ORIGIN", "https://duolinzao.example.com"),
        ]);
        let config =
            AppConfig::parse_with_env(&raw, &|k| env.get(k).cloned()).expect("密钥齐应启动");
        assert!(config.cookie_secure);
        assert!(config.trust_proxy);
        assert_eq!(config.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(config.database_url, "postgres://u:p@db:5432/x");
    }

    #[test]
    fn staging_config_uses_log_delivery() {
        // deploy/config.staging.toml：密钥齐了能起，邮箱走日志投递且默认开启。
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/config.staging.toml");
        let raw = std::fs::read_to_string(path).expect("staging config 存在");
        let env = env_map(&[
            ("DLZ_DATABASE_URL", "postgres://u:p@db:5432/x"),
            ("DLZ_SUPER_PASSWORD", "stg-pass-1"),
            ("DLZ_LOCAL_KEY", "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY="),
            ("DLZ_WEBAUTHN_RP_ID", "staging.example.com"),
            ("DLZ_WEBAUTHN_ORIGIN", "https://staging.example.com"),
        ]);
        let config =
            AppConfig::parse_with_env(&raw, &|k| env.get(k).cloned()).expect("staging 应启动");
        assert!(config.email_enabled);
        assert_eq!(config.email_delivery, EmailDelivery::Log);
        assert_eq!(config.dish_image_dir, "/srv/duolinzao-staging/images");
    }

    #[test]
    fn secret_file_wins_and_fail_closed() {
        let dir = std::env::temp_dir().join(format!("dlz-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("db_url");
        std::fs::write(&secret, "postgres://u:filepw@db:5432/x\n").unwrap();
        let raw = example_config().replace(
            "url = \"postgres://ubuntu@localhost/duolinzao_test\"",
            "url = \"REPLACE_ME\"",
        );
        // _FILE 优先于同名直读变量，且裁掉末尾换行。
        let env = env_map(&[
            ("DLZ_DATABASE_URL_FILE", secret.to_str().unwrap()),
            ("DLZ_DATABASE_URL", "postgres://u:envpw@db:5432/x"),
        ]);
        let config =
            AppConfig::parse_with_env(&raw, &|k| env.get(k).cloned()).expect("file 优先");
        assert_eq!(config.database_url, "postgres://u:filepw@db:5432/x");
        // 指向不存在的文件：直接报错，不回退、不起库。
        let env = env_map(&[("DLZ_DATABASE_URL_FILE", "/no/such/secret")]);
        let err = AppConfig::parse_with_env(&raw, &|k| env.get(k).cloned())
            .expect_err("缺文件必须报错");
        assert!(err.contains("读密钥文件失败"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
