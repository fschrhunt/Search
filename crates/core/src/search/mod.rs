//! The MCP surface: expose discovery and fetching to an agent as tools.
//!
//! `Service` is the in-process facade over the three concerns — discovery,
//! fetch, and the index — and is what both the MCP tools and the binary's JSON
//! API call. Keeping one facade means the two surfaces cannot drift.

mod tools;

pub use tools::{serve_stdio, Server};

use std::sync::Arc;

use crate::config::Config;
use crate::discovery::{self, Query, Response};
use crate::fetch::{Fetched, Fetcher};
use crate::index::{Store, StoreError};

/// The wired service: everything a request handler needs, ready to use.
pub struct Service {
    registry: discovery::Registry,
    fetcher: Fetcher,
    store: Arc<Store>,
    config: Config,
}

impl Service {
    /// Build the service, opening the index under the configured data directory.
    pub fn open(config: Config) -> Result<Self, ServiceError> {
        let store = Arc::new(Store::open(&config.data_dir).map_err(ServiceError::Store)?);
        let fetcher = Fetcher::new(config.fetch.clone(), Arc::clone(&store), &config.user_agent);
        let registry = discovery::Registry::new(&config.engines, config.search.clone());
        Ok(Service {
            registry,
            fetcher,
            store,
            config,
        })
    }

    /// Discover results across providers.
    pub async fn search(&self, query: Query) -> Response {
        self.registry.search(query).await
    }

    /// Fetch and index one or more URLs, preserving order.
    pub async fn fetch(&self, urls: &[String]) -> Vec<Fetched> {
        self.fetcher.fetch_many(urls).await
    }

    /// Search only what has already been fetched.
    pub fn index_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::index::Hit>, StoreError> {
        self.store.search(query, limit)
    }

    /// The enabled provider names, for status output.
    pub fn provider_names(&self) -> Vec<&'static str> {
        self.registry.names()
    }

    /// Corpus counts.
    pub fn index_stats(&self) -> Result<crate::index::Stats, StoreError> {
        self.store.stats()
    }

    /// The running configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The configured query ceiling, for the HTTP surface.
    pub fn overall_timeout(&self) -> std::time::Duration {
        self.registry.overall_timeout()
    }
}

/// Why the service could not start.
#[derive(Debug)]
pub enum ServiceError {
    Store(StoreError),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Store(error) => write!(f, "open index: {error}"),
        }
    }
}

impl std::error::Error for ServiceError {}
