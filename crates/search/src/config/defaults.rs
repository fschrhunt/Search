//! The built-in defaults, applied after any deserialization so a config file
//! names only what it changes.

use std::time::Duration;

use super::{
    Config, EngineSettings, FetchSettings, IndexSettings, SearchSettings, DEFAULT_ADDR,
    DEFAULT_TOKEN_ENV,
};

/// Seconds a provider may take before it is abandoned.
pub const DEFAULT_PROVIDER_SECS: u64 = 2;
/// Seconds the whole fan-out may take.
pub const DEFAULT_OVERALL_SECS: u64 = 8;
/// Seconds a query answer is reused.
pub const DEFAULT_SEARCH_CACHE_SECS: u64 = 300;
/// Seconds a fetched page is reused from memory.
pub const DEFAULT_FETCH_CACHE_SECS: u64 = 600;
/// The most bytes read from one response.
pub const DEFAULT_MAX_BYTES: u64 = 4 << 20;
/// Redirects followed.
pub const DEFAULT_MAX_REDIRECTS: usize = 5;
/// Fetches in flight at once.
pub const DEFAULT_MAX_CONCURRENCY: usize = 8;
/// The most characters of a page stored in the index.
pub const DEFAULT_INDEX_TEXT_CHARS: usize = 40_000;
/// The corpus's size ceiling, in megabytes.
pub const DEFAULT_INDEX_MAX_SIZE_MB: u64 = 512;
/// How long a corpus document lives before it is pruned.
pub const DEFAULT_INDEX_MAX_AGE_DAYS: u64 = 180;
/// How long a seeded document stays fresh.
pub const DEFAULT_REFRESH_AFTER_DAYS: u64 = 7;
/// How much a local hit counts against a borrowed one.
pub const DEFAULT_INDEX_WEIGHT: f64 = 1.5;

impl Default for Config {
    fn default() -> Self {
        Config {
            addr: DEFAULT_ADDR.into(),
            data_dir: default_data_dir(),
            token: None,
            token_env: DEFAULT_TOKEN_ENV.into(),
            log: Default::default(),
            user_agent: format!(
                "search/{} (+https://github.com/fschrhunt/search)",
                crate::VERSION
            ),
            search: SearchSettings::default(),
            fetch: FetchSettings::default(),
            engines: EngineSettings::default(),
            index: IndexSettings::default(),
        }
    }
}

impl Default for SearchSettings {
    fn default() -> Self {
        SearchSettings {
            max_results: 10,
            max_provider_time_ms: secs(DEFAULT_PROVIDER_SECS),
            overall_timeout_ms: secs(DEFAULT_OVERALL_SECS),
            cache_ttl_ms: secs(DEFAULT_SEARCH_CACHE_SECS),
            use_index: None,
            index_weight: DEFAULT_INDEX_WEIGHT,
        }
    }
}

impl Default for FetchSettings {
    fn default() -> Self {
        FetchSettings {
            timeout_ms: secs(15),
            max_bytes: DEFAULT_MAX_BYTES,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            cache_ttl_ms: secs(DEFAULT_FETCH_CACHE_SECS),
            allow_private: false,
            index_fetched: None,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            index_text_chars: DEFAULT_INDEX_TEXT_CHARS,
        }
    }
}

impl Default for IndexSettings {
    fn default() -> Self {
        IndexSettings {
            max_size_mb: DEFAULT_INDEX_MAX_SIZE_MB,
            max_age_days: DEFAULT_INDEX_MAX_AGE_DAYS,
            refresh_hosts: Vec::new(),
            refresh_after_days: DEFAULT_REFRESH_AFTER_DAYS,
        }
    }
}

fn secs(n: u64) -> u64 {
    Duration::from_secs(n).as_millis() as u64
}

/// `~/.local/share/search`, or the working directory when there is no home.
fn default_data_dir() -> std::path::PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => std::path::PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("search"),
        None => std::path::PathBuf::from("data"),
    }
}

/// Fill a zero-valued settings block with its defaults. A hand-written config
/// that names only `addr` must still get sane timeouts.
pub(super) fn fill(config: &mut Config) {
    let search_defaults = SearchSettings::default();
    if config.search.max_results == 0 {
        config.search.max_results = search_defaults.max_results;
    }
    if config.search.max_provider_time_ms == 0 {
        config.search.max_provider_time_ms = search_defaults.max_provider_time_ms;
    }
    if config.search.overall_timeout_ms == 0 {
        config.search.overall_timeout_ms = search_defaults.overall_timeout_ms;
    }
    if config.search.cache_ttl_ms == 0 {
        config.search.cache_ttl_ms = search_defaults.cache_ttl_ms;
    }

    let fetch_defaults = FetchSettings::default();
    if config.fetch.timeout_ms == 0 {
        config.fetch.timeout_ms = fetch_defaults.timeout_ms;
    }
    if config.fetch.max_bytes == 0 {
        config.fetch.max_bytes = fetch_defaults.max_bytes;
    }
    if config.fetch.max_redirects == 0 {
        config.fetch.max_redirects = fetch_defaults.max_redirects;
    }
    if config.fetch.cache_ttl_ms == 0 {
        config.fetch.cache_ttl_ms = fetch_defaults.cache_ttl_ms;
    }
    if config.fetch.max_concurrency == 0 {
        config.fetch.max_concurrency = fetch_defaults.max_concurrency;
    }
    if config.fetch.index_text_chars == 0 {
        config.fetch.index_text_chars = fetch_defaults.index_text_chars;
    }

    // IndexSettings: a zero size ceiling or age is a deliberate "no limit", so
    // `fill` restores only the field a zero cannot express.
    let index_defaults = IndexSettings::default();
    if config.index.refresh_after_days == 0 {
        config.index.refresh_after_days = index_defaults.refresh_after_days;
    }
}
