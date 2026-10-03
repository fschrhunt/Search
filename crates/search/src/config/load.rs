//! Reading, merging, and validating configuration.

use std::path::PathBuf;

use super::defaults;
use super::Config;

/// A configuration problem an operator must fix. It carries a message, not a
/// source error, because every cause is a file the operator owns.
#[derive(Debug)]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        ConfigError(message.into())
    }
    /// The operator-facing message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Load configuration from `path` (or the configured default), apply defaults
/// and environment overrides, and validate the result.
pub fn load(path: Option<PathBuf>) -> Result<Config, ConfigError> {
    let path = path
        .or_else(|| std::env::var_os("SEARCH_CONFIG").map(PathBuf::from))
        .or_else(default_path);

    let mut config = match &path {
        Some(path) if path.exists() => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| ConfigError::new(format!("read {}: {e}", path.display())))?;
            serde_json::from_str::<Config>(&text)
                .map_err(|e| ConfigError::new(format!("parse {}: {e}", path.display())))?
        }
        _ => Config::default(),
    };
    defaults::fill(&mut config);
    apply_env(&mut config);
    config.validate()?;
    Ok(config)
}

/// The standard location: `$XDG_CONFIG_HOME/search/search.json` or
/// `~/.config/search/search.json`.
fn default_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("search").join("search.json"));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("search")
            .join("search.json")
    })
}

/// Environment variables win over the file for the fields that name a place to
/// bind or store.
fn apply_env(config: &mut Config) {
    if let Ok(addr) = std::env::var("SEARCH_ADDR") {
        if !addr.is_empty() {
            config.addr = addr;
        }
    }
    if let Ok(dir) = std::env::var("SEARCH_DATA_DIR") {
        if !dir.is_empty() {
            config.data_dir = PathBuf::from(dir);
        }
    }
}

impl Config {
    /// Resolve the token: the environment first, then a literal in the file.
    /// `None` when neither is set.
    pub fn resolved_token(&self) -> Option<String> {
        if let Ok(value) = std::env::var(&self.token_env) {
            if !value.is_empty() {
                return Some(value);
            }
        }
        self.token.clone().filter(|t| !t.is_empty())
    }

    /// Resolve a provider key from its configured environment variable.
    pub fn resolved_key(&self, provider: &str) -> Option<String> {
        let var = self.engines.key_envs.get(provider)?;
        std::env::var(var).ok().filter(|v| !v.is_empty())
    }

    /// Whether the configured address is loopback. Accepts `host:port`, a bare
    /// `:port` (which binds every interface and is therefore NOT loopback), and
    /// `localhost:port`.
    pub fn is_loopback(&self) -> bool {
        match self.addr.rsplit_once(':') {
            Some((host, _)) => match host {
                "" => false,
                "localhost" => true,
                host => host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<std::net::IpAddr>()
                    .map(|ip| ip.is_loopback())
                    // A hostname that is not an IP literal: treat as non-loopback,
                    // which fails safe by requiring a token.
                    .unwrap_or(false),
            },
            None => false,
        }
    }

    /// Reject settings that would be unsafe or nonsensical: a non-loopback bind
    /// needs a token, and a token must be long enough to mean anything.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !valid_addr(&self.addr) {
            return Err(ConfigError::new(format!(
                "addr {:?} is not host:port",
                self.addr
            )));
        }
        let token = self.resolved_token();
        if !self.is_loopback() && token.is_none() {
            return Err(ConfigError::new(format!(
                "addr {:?} is not loopback; a token is required",
                self.addr
            )));
        }
        if let Some(token) = token {
            if token.len() < 16 {
                return Err(ConfigError::new("token must be at least 16 characters"));
            }
        }
        Ok(())
    }
}

/// Whether `addr` is a bindable `host:port`, accepting an empty host (`:8642`)
/// and a bracketed IPv6 literal.
fn valid_addr(addr: &str) -> bool {
    let Some((host, port)) = addr.rsplit_once(':') else {
        return false;
    };
    if port.is_empty() || port.parse::<u16>().is_err() {
        return false;
    }
    if host.is_empty() || host == "localhost" {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<std::net::IpAddr>().is_ok() || is_hostname(bare)
}

/// A conservative hostname check: letters, digits, dots, and hyphens only. A
/// name that passes still resolves at bind time; this only keeps validation from
/// rejecting a legitimate name like `search.internal.example`.
fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parsed loopback check must not mistake `:8642` — which binds every
    /// interface — for loopback.
    #[test]
    fn all_interfaces_is_not_loopback() {
        for addr in [":8642", "0.0.0.0:8642", "[::]:8642", "100.1.2.3:8642"] {
            let config = Config {
                addr: addr.into(),
                ..Config::default()
            };
            assert!(!config.is_loopback(), "{addr} should not be loopback");
        }
        for addr in ["127.0.0.1:8642", "[::1]:8642", "localhost:8642"] {
            let config = Config {
                addr: addr.into(),
                ..Config::default()
            };
            assert!(config.is_loopback(), "{addr} should be loopback");
        }
    }

    /// A non-loopback bind without a token must be refused.
    #[test]
    fn non_loopback_requires_a_token() {
        for addr in [":8642", "0.0.0.0:8642", "[::]:8642"] {
            let mut config = Config {
                addr: addr.into(),
                ..Config::default()
            };
            config.token = None;
            config.token_env = "SEARCH_TEST_UNSET_TOKEN".into();
            assert!(config.validate().is_err(), "{addr} should require a token");
            config.token = Some("a-sufficiently-long-token".into());
            assert!(config.validate().is_ok(), "{addr} with a token should pass");
        }
    }

    #[test]
    fn malformed_addr_is_rejected() {
        let config = Config {
            addr: "not-an-address".into(),
            token: Some("a-sufficiently-long-token".into()),
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn short_tokens_are_rejected() {
        let config = Config {
            addr: "127.0.0.1:8642".into(),
            token: Some("short".into()),
            token_env: "SEARCH_TEST_UNSET_TOKEN".into(),
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }
}
