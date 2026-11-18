//! Typed configuration ([system/10](../../../spec/system/10-config-ops.md#environment-variables)).
//! Parsed once at startup; invalid values fail fast naming the variable. Secrets never appear in `Debug`.

use std::fmt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use corvus_domain::AgentName;

use crate::envfile::{EnvFile, EnvFileError};

pub const DEFAULT_HTTP_PORT: u16 = 8765;
pub const DEFAULT_LEASE_SECONDS: u64 = 900;
pub const DEFAULT_ACTIVE_SECONDS: u64 = 120;
pub const DEFAULT_IDLE_SECONDS: u64 = 900;
pub const DEFAULT_AUTO_END_SECONDS: u64 = 3600;
pub const DEFAULT_HOOK_THROTTLE_SECONDS: u64 = 5;
pub const DEFAULT_POLL_SECONDS: u64 = 60;
pub const MIN_POLL_SECONDS: u64 = 15;
pub const MIN_TOKEN_LEN: usize = 32;

/// A secret string. `Debug` never shows it, and it has no `PartialEq`: comparing secrets must go through a
/// constant-time check (system/08).
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    /// The secret value. Call only where it is actually sent.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A configuration error, always naming the variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub var: String,
    pub message: String,
}

impl ConfigError {
    fn new(var: &str, message: impl Into<String>) -> Self {
        Self {
            var: var.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.var, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Errors from [`Config::load`].
#[derive(Debug)]
pub enum LoadError {
    EnvFile(EnvFileError),
    Config(ConfigError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvFile(e) => e.fmt(f),
            Self::Config(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug, Clone)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: Secret,
    pub poll_seconds: u64,
    pub pool: String,
    pub ignore_accounts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LinearConfig {
    pub api_key: Secret,
    pub poll_seconds: u64,
    pub pool: String,
    pub ignore_users: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub state_dir: PathBuf,
    pub database_path: PathBuf,
    pub api_token: Option<Secret>,
    pub http_port: u16,
    pub lease_seconds: u64,
    pub active_seconds: u64,
    pub idle_seconds: u64,
    /// 0 disables auto-end.
    pub auto_end_seconds: u64,
    pub operator_name: AgentName,
    pub export_repo: Option<PathBuf>,
    pub agent_name: Option<AgentName>,
    pub hook_throttle_seconds: u64,
    /// Whether the desktop app may ask GitHub for a newer version (`CORVUS_UPDATE_CHECK`, default on).
    pub update_check: bool,
    pub env_file: PathBuf,
    pub log: String,
    pub jira: Option<JiraConfig>,
    pub linear: Option<LinearConfig>,
}

impl Config {
    /// Loads from the process environment plus the env file (`CORVUS_ENV_FILE`, default
    /// `~/.config/corvus/env`). The process environment wins.
    pub fn load() -> Result<Self, LoadError> {
        let process = |k: &str| std::env::var(k).ok();
        let env_file = env_file_path(&process).map_err(LoadError::Config)?;
        let file = EnvFile::load(&env_file).map_err(LoadError::EnvFile)?;
        Self::from_lookup(|k| process(k).or_else(|| file.get(k).map(str::to_owned)))
            .map_err(LoadError::Config)
    }

    /// Parses configuration from any variable lookup. Empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |k: &str| lookup(k).filter(|v| !v.trim().is_empty());
        let env_file = env_file_path(&get)?;

        let state_dir = match get("CORVUS_STATE_DIR") {
            Some(p) => absolute("CORVUS_STATE_DIR", &p)?,
            None => home(&get, "CORVUS_STATE_DIR")?.join(".corvus"),
        };
        let database_path = match get("CORVUS_DATABASE_URL") {
            Some(url) => parse_database_url(&url)?,
            None => state_dir.join("corvus.db"),
        };

        let api_token = get("CORVUS_API_TOKEN")
            .map(|t| {
                if t.chars().count() < MIN_TOKEN_LEN {
                    Err(ConfigError::new(
                        "CORVUS_API_TOKEN",
                        format!("must be at least {MIN_TOKEN_LEN} characters (try: openssl rand -hex 32)"),
                    ))
                } else {
                    Ok(Secret::new(t))
                }
            })
            .transpose()?;

        let http_port = match get("CORVUS_HTTP_PORT") {
            None => DEFAULT_HTTP_PORT,
            Some(v) => match v.trim().parse::<u16>() {
                Ok(p) if p > 0 => p,
                _ => {
                    return Err(ConfigError::new(
                        "CORVUS_HTTP_PORT",
                        "must be a port number 1-65535",
                    ));
                }
            },
        };

        let lease_seconds = seconds(&get, "CORVUS_LEASE_SECONDS", DEFAULT_LEASE_SECONDS, 1)?;
        let active_seconds = seconds(&get, "CORVUS_ACTIVE_SECONDS", DEFAULT_ACTIVE_SECONDS, 1)?;
        let idle_seconds = seconds(&get, "CORVUS_IDLE_SECONDS", DEFAULT_IDLE_SECONDS, 1)?;
        if idle_seconds <= active_seconds {
            return Err(ConfigError::new(
                "CORVUS_IDLE_SECONDS",
                format!("must be greater than CORVUS_ACTIVE_SECONDS ({active_seconds})"),
            ));
        }
        let auto_end_seconds = seconds(&get, "CORVUS_AUTO_END_SECONDS", DEFAULT_AUTO_END_SECONDS, 0)?;
        let hook_throttle_seconds = seconds(
            &get,
            "CORVUS_HOOK_THROTTLE_SECONDS",
            DEFAULT_HOOK_THROTTLE_SECONDS,
            0,
        )?;

        let operator_name = agent(&get, "CORVUS_OPERATOR_NAME")?
            .unwrap_or_else(|| AgentName::parse("operator").expect("valid literal"));
        let agent_name = agent(&get, "CORVUS_AGENT_NAME")?;
        let export_repo = get("CORVUS_EXPORT_REPO")
            .map(|p| absolute("CORVUS_EXPORT_REPO", &p))
            .transpose()?;

        let update_check = match get("CORVUS_UPDATE_CHECK").as_deref().map(str::trim) {
            None | Some("on") => true,
            Some("off") => false,
            Some(_) => return Err(ConfigError::new("CORVUS_UPDATE_CHECK", "must be on or off")),
        };

        let log = get("CORVUS_LOG").unwrap_or_else(|| "info".to_owned());
        if !log
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_=,:.-".contains(c))
        {
            return Err(ConfigError::new(
                "CORVUS_LOG",
                "must be a level like info, debug or warn",
            ));
        }

        Ok(Self {
            state_dir,
            database_path,
            api_token,
            http_port,
            lease_seconds,
            active_seconds,
            idle_seconds,
            auto_end_seconds,
            operator_name,
            export_repo,
            agent_name,
            hook_throttle_seconds,
            update_check,
            env_file,
            log,
            jira: jira(&get)?,
            linear: linear(&get)?,
        })
    }

    /// Creates the state directory (`0700`) if it does not exist.
    pub fn ensure_state_dir(&self) -> std::io::Result<()> {
        ensure_private_dir(&self.state_dir)
    }
}

/// Creates `dir` and missing parents; directories this call creates get mode `0700`.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // `recursive` applies the mode to every created level, subject to umask; make the leaf exact.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

fn env_file_path(get: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    match get("CORVUS_ENV_FILE").filter(|v| !v.trim().is_empty()) {
        Some(p) => absolute("CORVUS_ENV_FILE", &p),
        None => Ok(home(get, "CORVUS_ENV_FILE")?.join(".config/corvus/env")),
    }
}

fn home(get: &impl Fn(&str) -> Option<String>, var: &str) -> Result<PathBuf, ConfigError> {
    match get("HOME").filter(|h| h.starts_with('/')) {
        Some(h) => Ok(PathBuf::from(h)),
        None => Err(ConfigError::new(
            var,
            "not set, and HOME is not set to an absolute path to default from",
        )),
    }
}

fn absolute(var: &str, p: &str) -> Result<PathBuf, ConfigError> {
    let p = p.trim();
    if p.starts_with('/') {
        Ok(PathBuf::from(p))
    } else {
        Err(ConfigError::new(var, "must be an absolute path"))
    }
}

/// Accepts a plain absolute path, `sqlite:/abs/path` or `sqlite:///abs/path`.
pub fn parse_database_url(url: &str) -> Result<PathBuf, ConfigError> {
    const VAR: &str = "CORVUS_DATABASE_URL";
    let url = url.trim();
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    if path.contains('?') || path.contains(":memory:") {
        return Err(ConfigError::new(
            VAR,
            "must be a file path; query strings and :memory: are not supported",
        ));
    }
    if !path.starts_with('/') {
        return Err(ConfigError::new(
            VAR,
            "must be an absolute file path or sqlite:///absolute/path",
        ));
    }
    if path.ends_with('/') {
        return Err(ConfigError::new(VAR, "must name a file, not a directory"));
    }
    Ok(PathBuf::from(path))
}

fn seconds(
    get: &impl Fn(&str) -> Option<String>,
    var: &str,
    default: u64,
    min: u64,
) -> Result<u64, ConfigError> {
    match get(var) {
        None => Ok(default),
        Some(v) => match v.trim().parse::<u64>() {
            Ok(n) if n >= min && n <= 31_536_000 => Ok(n),
            _ => Err(ConfigError::new(
                var,
                format!("must be a whole number of seconds from {min} to 31536000"),
            )),
        },
    }
}

fn agent(get: &impl Fn(&str) -> Option<String>, var: &str) -> Result<Option<AgentName>, ConfigError> {
    get(var)
        .map(|v| {
            AgentName::parse(v.trim()).map_err(|_| {
                ConfigError::new(
                    var,
                    "must be 1-64 characters of A-Z a-z 0-9 _ . - starting with a letter or digit",
                )
            })
        })
        .transpose()
}

fn pool(get: &impl Fn(&str) -> Option<String>, var: &str) -> Result<String, ConfigError> {
    let p = get(var).unwrap_or_else(|| "default".to_owned()).trim().to_owned();
    if p.chars().count() > 64 || p.chars().any(char::is_control) {
        return Err(ConfigError::new(var, "must be at most 64 printable characters"));
    }
    Ok(p)
}

fn list(get: &impl Fn(&str) -> Option<String>, var: &str) -> Vec<String> {
    get(var)
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn poll(get: &impl Fn(&str) -> Option<String>, var: &str) -> Result<u64, ConfigError> {
    seconds(get, var, DEFAULT_POLL_SECONDS, MIN_POLL_SECONDS)
}

fn jira(get: &impl Fn(&str) -> Option<String>) -> Result<Option<JiraConfig>, ConfigError> {
    const REQUIRED: [&str; 3] = [
        "CORVUS_JIRA_BASE_URL",
        "CORVUS_JIRA_EMAIL",
        "CORVUS_JIRA_API_TOKEN",
    ];
    let present: Vec<_> = REQUIRED.iter().map(|k| get(k)).collect();
    if present.iter().all(Option::is_none) {
        return Ok(None);
    }
    let missing: Vec<&str> = REQUIRED
        .iter()
        .zip(&present)
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| *k)
        .collect();
    if !missing.is_empty() {
        return Err(ConfigError::new(
            missing[0],
            format!(
                "the Jira source needs all of {}; missing {}",
                REQUIRED.join(", "),
                missing.join(", ")
            ),
        ));
    }
    let [base_url, email, token] = [0, 1, 2].map(|i| present[i].clone().unwrap_or_default());
    let base_url = base_url.trim().trim_end_matches('/').to_owned();
    let host = base_url.strip_prefix("https://").unwrap_or("");
    if host.is_empty() || host.contains(['@', '?', '#', ' ']) {
        return Err(ConfigError::new(
            "CORVUS_JIRA_BASE_URL",
            "must be an https:// URL without credentials, query or fragment",
        ));
    }
    Ok(Some(JiraConfig {
        base_url,
        email: email.trim().to_owned(),
        api_token: Secret::new(token),
        poll_seconds: poll(get, "CORVUS_JIRA_POLL_SECONDS")?,
        pool: pool(get, "CORVUS_JIRA_POOL")?,
        ignore_accounts: list(get, "CORVUS_JIRA_IGNORE_ACCOUNTS"),
    }))
}

fn linear(get: &impl Fn(&str) -> Option<String>) -> Result<Option<LinearConfig>, ConfigError> {
    let Some(key) = get("CORVUS_LINEAR_API_KEY") else {
        return Ok(None);
    };
    Ok(Some(LinearConfig {
        api_key: Secret::new(key.trim()),
        poll_seconds: poll(get, "CORVUS_LINEAR_POLL_SECONDS")?,
        pool: pool(get, "CORVUS_LINEAR_POOL")?,
        ignore_users: list(get, "CORVUS_LINEAR_IGNORE_USERS"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        m.entry("HOME".into()).or_insert_with(|| "/home/u".into());
        Config::from_lookup(|k| m.get(k).cloned())
    }

    fn err_var(pairs: &[(&str, &str)]) -> String {
        cfg(pairs).unwrap_err().var
    }

    #[test]
    fn defaults() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.state_dir, PathBuf::from("/home/u/.corvus"));
        assert_eq!(c.database_path, PathBuf::from("/home/u/.corvus/corvus.db"));
        assert_eq!(c.env_file, PathBuf::from("/home/u/.config/corvus/env"));
        assert!(c.api_token.is_none());
        assert_eq!(c.http_port, 8765);
        assert_eq!(c.lease_seconds, 900);
        assert_eq!(c.active_seconds, 120);
        assert_eq!(c.idle_seconds, 900);
        assert_eq!(c.auto_end_seconds, 3600);
        assert_eq!(c.operator_name.as_str(), "operator");
        assert_eq!(c.export_repo, None);
        assert_eq!(c.agent_name, None);
        assert_eq!(c.hook_throttle_seconds, 5);
        assert!(c.update_check);
        assert_eq!(c.log, "info");
        assert!(c.jira.is_none());
        assert!(c.linear.is_none());
    }

    #[test]
    fn overrides() {
        let c = cfg(&[
            ("CORVUS_STATE_DIR", "/s"),
            ("CORVUS_DATABASE_URL", "sqlite:///d/x.db"),
            ("CORVUS_API_TOKEN", TOKEN),
            ("CORVUS_HTTP_PORT", "9000"),
            ("CORVUS_LEASE_SECONDS", "60"),
            ("CORVUS_ACTIVE_SECONDS", "10"),
            ("CORVUS_IDLE_SECONDS", "20"),
            ("CORVUS_AUTO_END_SECONDS", "0"),
            ("CORVUS_OPERATOR_NAME", "boss"),
            ("CORVUS_EXPORT_REPO", "/repo"),
            ("CORVUS_AGENT_NAME", "agent-1"),
            ("CORVUS_HOOK_THROTTLE_SECONDS", "0"),
            ("CORVUS_UPDATE_CHECK", "off"),
            ("CORVUS_ENV_FILE", "/e/env"),
            ("CORVUS_LOG", "debug"),
        ])
        .unwrap();
        assert_eq!(c.state_dir, PathBuf::from("/s"));
        assert_eq!(c.database_path, PathBuf::from("/d/x.db"));
        assert_eq!(c.api_token.as_ref().map(Secret::expose), Some(TOKEN));
        assert_eq!(c.http_port, 9000);
        assert_eq!(
            (
                c.lease_seconds,
                c.active_seconds,
                c.idle_seconds,
                c.auto_end_seconds
            ),
            (60, 10, 20, 0)
        );
        assert_eq!(c.operator_name.as_str(), "boss");
        assert_eq!(c.export_repo, Some(PathBuf::from("/repo")));
        assert_eq!(
            c.agent_name.map(|a| a.as_str().to_owned()),
            Some("agent-1".into())
        );
        assert_eq!(c.hook_throttle_seconds, 0);
        assert!(!c.update_check);
        assert_eq!(c.env_file, PathBuf::from("/e/env"));
        assert_eq!(c.log, "debug");
    }

    #[test]
    fn database_url_forms() {
        for (u, p) in [
            ("/a/b.db", "/a/b.db"),
            ("sqlite:/a/b.db", "/a/b.db"),
            ("sqlite:///a/b.db", "/a/b.db"),
        ] {
            assert_eq!(parse_database_url(u).unwrap(), PathBuf::from(p), "{u}");
        }
        for u in [
            "rel.db",
            "sqlite:rel.db",
            "sqlite::memory:",
            "/a/b.db?mode=ro",
            "/a/dir/",
            "postgres://x",
        ] {
            assert!(parse_database_url(u).is_err(), "{u}");
        }
    }

    #[test]
    fn each_invalid_value_names_its_variable() {
        let cases: &[(&str, &str)] = &[
            ("CORVUS_STATE_DIR", "relative"),
            ("CORVUS_DATABASE_URL", "x.db"),
            ("CORVUS_API_TOKEN", "short"),
            ("CORVUS_HTTP_PORT", "0"),
            ("CORVUS_HTTP_PORT", "70000"),
            ("CORVUS_HTTP_PORT", "abc"),
            ("CORVUS_LEASE_SECONDS", "0"),
            ("CORVUS_LEASE_SECONDS", "-5"),
            ("CORVUS_LEASE_SECONDS", "1.5"),
            ("CORVUS_ACTIVE_SECONDS", "x"),
            ("CORVUS_AUTO_END_SECONDS", "-1"),
            ("CORVUS_HOOK_THROTTLE_SECONDS", "soon"),
            ("CORVUS_UPDATE_CHECK", "maybe"),
            ("CORVUS_OPERATOR_NAME", "-bad"),
            ("CORVUS_OPERATOR_NAME", "has space"),
            ("CORVUS_AGENT_NAME", "a:b"),
            ("CORVUS_EXPORT_REPO", "repo"),
            ("CORVUS_ENV_FILE", "env"),
            ("CORVUS_LOG", "debug; rm"),
            ("CORVUS_JIRA_POLL_SECONDS", "5"),
            ("CORVUS_LINEAR_POLL_SECONDS", "14"),
        ];
        for (var, val) in cases {
            let mut pairs = vec![(*var, *val)];
            if var.starts_with("CORVUS_JIRA") {
                pairs.extend([
                    ("CORVUS_JIRA_BASE_URL", "https://x.atlassian.net"),
                    ("CORVUS_JIRA_EMAIL", "e@x"),
                    ("CORVUS_JIRA_API_TOKEN", "t"),
                ]);
            }
            if var.starts_with("CORVUS_LINEAR") {
                pairs.push(("CORVUS_LINEAR_API_KEY", "k"));
            }
            assert_eq!(err_var(&pairs), *var, "{var}={val}");
        }
        assert_eq!(
            err_var(&[("CORVUS_ACTIVE_SECONDS", "900")]),
            "CORVUS_IDLE_SECONDS"
        );
    }

    #[test]
    fn no_home_and_no_state_dir_is_an_error() {
        let e = Config::from_lookup(|_| None).unwrap_err();
        assert_eq!(e.var, "CORVUS_ENV_FILE");
        let e = Config::from_lookup(|k| (k == "CORVUS_ENV_FILE").then(|| "/e".to_owned())).unwrap_err();
        assert_eq!(e.var, "CORVUS_STATE_DIR");
    }

    #[test]
    fn jira_all_or_nothing() {
        let e = cfg(&[("CORVUS_JIRA_EMAIL", "e@x")]).unwrap_err();
        assert!(e.message.contains("CORVUS_JIRA_BASE_URL") && e.message.contains("CORVUS_JIRA_API_TOKEN"));
        let full = [
            ("CORVUS_JIRA_BASE_URL", "https://x.atlassian.net/"),
            ("CORVUS_JIRA_EMAIL", "e@x"),
            ("CORVUS_JIRA_API_TOKEN", "tok"),
            ("CORVUS_JIRA_IGNORE_ACCOUNTS", "a, b,,c"),
        ];
        let j = cfg(&full).unwrap().jira.unwrap();
        assert_eq!(j.base_url, "https://x.atlassian.net");
        assert_eq!(j.poll_seconds, 60);
        assert_eq!(j.pool, "default");
        assert_eq!(j.ignore_accounts, vec!["a", "b", "c"]);
        for bad in ["http://x", "https://", "https://u:p@x", "https://x?q=1"] {
            let mut p = full.to_vec();
            p[0] = ("CORVUS_JIRA_BASE_URL", bad);
            assert_eq!(err_var(&p), "CORVUS_JIRA_BASE_URL", "{bad}");
        }
    }

    #[test]
    fn linear_enabled_by_key() {
        let l = cfg(&[
            ("CORVUS_LINEAR_API_KEY", "lin_api_x"),
            ("CORVUS_LINEAR_POOL", "ops"),
        ])
        .unwrap()
        .linear
        .unwrap();
        assert_eq!(l.api_key.expose(), "lin_api_x");
        assert_eq!((l.poll_seconds, l.pool.as_str()), (60, "ops"));
    }

    #[test]
    fn secrets_absent_from_debug() {
        let c = cfg(&[
            ("CORVUS_API_TOKEN", TOKEN),
            ("CORVUS_JIRA_BASE_URL", "https://x.atlassian.net"),
            ("CORVUS_JIRA_EMAIL", "e@x"),
            ("CORVUS_JIRA_API_TOKEN", "jira-secret-value"),
            ("CORVUS_LINEAR_API_KEY", "linear-secret-value"),
        ])
        .unwrap();
        let d = format!("{c:?}");
        for s in [TOKEN, "jira-secret-value", "linear-secret-value"] {
            assert!(!d.contains(s), "{s} leaked into Debug");
        }
        assert!(d.contains("<redacted>"));
    }

    #[test]
    fn empty_values_count_as_unset() {
        let c = cfg(&[("CORVUS_API_TOKEN", ""), ("CORVUS_HTTP_PORT", " ")]).unwrap();
        assert!(c.api_token.is_none());
        assert_eq!(c.http_port, 8765);
    }

    #[test]
    fn state_dir_created_private() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("a/b/.corvus");
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        ensure_private_dir(&dir).unwrap();
    }
}
