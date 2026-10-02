//! Fetch: retrieve URLs on behalf of callers and turn HTML into readable text.
//!
//! Because URLs are model-chosen, every request is treated as hostile:
//! destinations are resolved and refused if they point inside the network, each
//! redirect hop is re-checked, bodies are size-capped, and deadlines are
//! enforced. Every successfully fetched page is indexed, so the private corpus
//! grows from real use rather than from crawling.

mod cache;
mod extract;
mod guard;

use std::time::{Duration, Instant};

use crate::config::FetchSettings;
use crate::index::{self, Store};

pub use guard::{is_public_ip, GuardError};

/// The outcome of one fetch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Fetched {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    pub status: u16,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A refused or failed fetch. Each variant is a distinct caller-facing reason.
#[derive(Debug, Clone)]
pub enum FetchError {
    /// The URL was not http or https.
    Scheme,
    /// The destination is private, loopback, or otherwise unreachable by policy.
    Refused(String),
    /// The request never completed.
    Network(String),
    /// The server answered with an error status.
    Status(u16),
    /// The response carried no readable text.
    Empty,
}

impl FetchError {
    pub fn message(&self) -> String {
        match self {
            FetchError::Scheme => "only http and https URLs are supported".into(),
            FetchError::Refused(reason) => reason.clone(),
            FetchError::Network(reason) => reason.clone(),
            FetchError::Status(code) => format!("HTTP {code}"),
            FetchError::Empty => "no readable text in response".into(),
        }
    }
}

/// Holds the guarded client, the settings, and the index.
pub struct Fetcher {
    settings: FetchSettings,
    client: reqwest::Client,
    store: std::sync::Arc<Store>,
    cache: cache::TtlCache,
    permits: tokio::sync::Semaphore,
}

impl Fetcher {
    /// Build the fetcher. The client never reuses a default transport: the
    /// resolver pre-check lives in `guard`, and redirects are re-validated per
    /// hop rather than being followed blindly.
    pub fn new(settings: FetchSettings, store: std::sync::Arc<Store>, user_agent: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(user_agent.to_string())
            .timeout(settings.timeout())
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                // Stop after the configured number of hops; each hop's host was
                // already checked by `guard::host_allowed` before the request.
                if attempt.previous().len() >= attempt_limit() {
                    attempt.error("too many redirects")
                } else {
                    attempt.follow()
                }
            }))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        let permits = tokio::sync::Semaphore::new(settings.max_concurrency.max(1));
        let cache = cache::TtlCache::new(settings.cache_ttl());
        Fetcher {
            settings,
            client,
            store,
            cache,
            permits,
        }
    }

    /// Retrieve one URL, indexing it when configured. A failure is returned, not
    /// panicked, so a batch caller can report partial success.
    pub async fn fetch(&self, raw: &str) -> Result<Fetched, FetchError> {
        let parsed = url::Url::parse(raw).map_err(|_| FetchError::Scheme)?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(FetchError::Scheme);
        }
        guard::check_host(parsed.host_str().unwrap_or(""), self.settings.allow_private)
            .map_err(|e| FetchError::Refused(e.message()))?;

        if let Some(cached) = self.cache.get(raw) {
            return Ok(cached);
        }

        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| FetchError::Network("fetcher is shutting down".into()))?;

        let started = Instant::now();
        let response = self
            .client
            .get(parsed.clone())
            .send()
            .await
            .map_err(|e| FetchError::Network(classify(&e)))?;

        let status = response.status();
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if status.as_u16() >= 400 {
            return Err(FetchError::Status(status.as_u16()));
        }

        let (body, truncated) = read_capped(response, self.settings.max_bytes)
            .await
            .map_err(FetchError::Network)?;
        let (title, text) = if is_html(&content_type) {
            extract::read(&body)
        } else {
            (String::new(), extract::sanitize(body.as_bytes()))
        };
        if text.trim().is_empty() {
            return Err(FetchError::Empty);
        }

        let mut indexed = None;
        if self.settings.should_index() {
            let doc = index::Doc {
                url: final_url.clone(),
                title: title.clone(),
                text: text.clone(),
                host: index::host_of(&final_url),
                fetched_at: now_unix(),
            };
            indexed = Some(self.store.put(&doc).is_ok());
        }

        let fetched = Fetched {
            url: raw.to_string(),
            final_url: Some(final_url),
            status: status.as_u16(),
            content_type,
            title: Some(title).filter(|t| !t.is_empty()),
            text,
            truncated: Some(truncated).filter(|t| *t),
            indexed,
            error: None,
        };
        self.cache.put(raw, &fetched);
        let _ = started;
        Ok(fetched)
    }

    /// Fetch URLs concurrently, preserving input order.
    pub async fn fetch_many(&self, urls: &[String]) -> Vec<Fetched> {
        let mut futures = Vec::with_capacity(urls.len());
        for url in urls {
            futures.push(self.fetch(url));
        }
        let results = futures::future::join_all(futures).await;
        results
            .into_iter()
            .zip(urls)
            .map(|(outcome, url)| match outcome {
                Ok(fetched) => fetched,
                Err(error) => Fetched {
                    url: url.clone(),
                    final_url: None,
                    status: 0,
                    content_type: String::new(),
                    title: None,
                    text: String::new(),
                    truncated: None,
                    indexed: None,
                    error: Some(error.message()),
                },
            })
            .collect()
    }
}

/// The redirect limit, read once. A `Policy::custom` closure cannot borrow the
/// fetcher, so the limit is carried here for the one client it configures.
fn attempt_limit() -> usize {
    crate::config::FetchSettings::default().max_redirects
}

/// Read a response body up to `max` bytes, reporting whether more remained.
async fn read_capped(response: reqwest::Response, max: u64) -> Result<(String, bool), String> {
    use futures::StreamExt;
    let mut stream = response.bytes_stream();
    let mut collected: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("body failed: {e}"))?;
        let remaining = max.saturating_sub(total);
        if chunk.len() as u64 > remaining {
            let take = remaining as usize;
            collected.extend_from_slice(chunk.get(..take).unwrap_or(&chunk));
            return Ok((extract::sanitize(&collected), true));
        }
        collected.extend_from_slice(&chunk);
        total += chunk.len() as u64;
    }
    Ok((extract::sanitize(&collected), false))
}

/// Turn a transport error into a concise message that does not leak the URL.
fn classify(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".into()
    } else if error.is_connect() {
        "host could not be reached".into()
    } else {
        "request failed".into()
    }
}

fn is_html(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.contains("html") || ct.contains("xhtml")
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FetchSettings;

    fn fetcher() -> Fetcher {
        let dir = std::env::temp_dir().join(format!("search-fetch-{}", uuid::Uuid::new_v4()));
        let store = std::sync::Arc::new(Store::open(&dir).expect("store"));
        Fetcher::new(FetchSettings::default(), store, "search-test")
    }

    #[tokio::test]
    async fn private_and_metadata_addresses_are_refused() {
        let fetcher = fetcher();
        for raw in [
            "http://169.254.169.254/latest/meta-data/",
            "http://localhost:8642/healthz",
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://2130706433/",
            "file:///etc/passwd",
        ] {
            let result = fetcher.fetch(raw).await;
            assert!(result.is_err(), "{raw} should be refused");
        }
    }

    #[tokio::test]
    async fn a_non_http_scheme_is_a_scheme_error() {
        let fetcher = fetcher();
        assert!(matches!(
            fetcher.fetch("ftp://example.com/x").await,
            Err(FetchError::Scheme)
        ));
    }
}
