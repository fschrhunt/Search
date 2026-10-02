//! The configuration shape and its invariants.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// The default listener: loopback only. Exposing search beyond loopback is a
/// deliberate act, and the token must be set to do it.
pub const DEFAULT_ADDR: &str = "127.0.0.1:8642";

/// The environment variable a token is read from unless the config names another.
pub const DEFAULT_TOKEN_ENV: &str = "SEARCH_TOKEN";

/// Everything the service can be told. Durations are milliseconds in the file,
/// because a number is what an operator can diff and a comment can explain.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Listen address, `host:port`.
    pub addr: String,
    /// Directory the index lives in.
    pub data_dir: PathBuf,
    /// A literal token. Prefer `token_env`.
    pub token: Option<String>,
    /// The environment variable holding the token.
    pub token_env: String,
    /// Log verbosity name.
    pub log: LogLevel,
    /// The user agent the fetcher sends.
    pub user_agent: String,
    pub search: SearchSettings,
    pub fetch: FetchSettings,
    pub engines: EngineSettings,
}

/// Bounds on one discovery query. Milliseconds on disk; durations in memory.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchSettings {
    /// Results returned at most.
    pub max_results: usize,
    /// How long any single provider may take before it is abandoned.
    #[serde(rename = "maxProviderTimeMs")]
    pub max_provider_time_ms: u64,
    /// The ceiling for the whole fan-out.
    #[serde(rename = "overallTimeoutMs")]
    pub overall_timeout_ms: u64,
    /// How long a query answer is reused.
    #[serde(rename = "cacheTtlMs")]
    pub cache_ttl_ms: u64,
}

impl SearchSettings {
    pub fn max_results_or_default(&self) -> usize {
        if self.max_results == 0 {
            10
        } else {
            self.max_results
        }
    }
    pub fn max_provider_time(&self) -> Duration {
        Duration::from_millis(self.max_provider_time_ms)
    }
    pub fn overall_timeout(&self) -> Duration {
        Duration::from_millis(self.overall_timeout_ms)
    }
    pub fn cache_ttl(&self) -> Duration {
        Duration::from_millis(self.cache_ttl_ms)
    }
}

/// Bounds on the fetcher.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FetchSettings {
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: u64,
    /// The most bytes read from any one response.
    pub max_bytes: u64,
    /// The most redirects followed, each re-checked.
    pub max_redirects: usize,
    #[serde(rename = "cacheTtlMs")]
    pub cache_ttl_ms: u64,
    /// Refuse only for tests and air-gapped mirrors: disabling the guard makes
    /// the fetcher able to reach private addresses.
    pub allow_private: bool,
    /// Whether a fetched page joins the private index. Defaults on.
    pub index_fetched: Option<bool>,
    /// How many fetches run at once.
    pub max_concurrency: usize,
}

impl FetchSettings {
    /// Whether fetched pages join the index (`index_fetched` defaults on).
    pub fn should_index(&self) -> bool {
        self.index_fetched.unwrap_or(true)
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
    pub fn cache_ttl(&self) -> Duration {
        Duration::from_millis(self.cache_ttl_ms)
    }
}

/// Which providers run, and the keys any of them need.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineSettings {
    /// Restrict to these provider names; empty means every keyless provider.
    pub enabled: Vec<String>,
    /// Provider name to the environment variable holding its key.
    pub key_envs: BTreeMap<String, String>,
}

/// Log verbosity, parsed from a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}
