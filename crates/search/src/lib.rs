//! The search engine library: discovery across independent providers, a
//! hardened fetcher, and a private on-disk index.
//!
//! Frontends and protocol adapters depend on this crate; the engine does not
//! depend on a terminal, listener, or protocol.
//!
//! Shipped code denies explicit panic sites outside test builds. Every allowed
//! site needs a proof comment explaining why runtime input cannot reach it.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing
    )
)]

pub mod config;
pub mod discovery;
pub mod fetch;
pub mod index;
mod service;
pub mod text;

pub use config::Config;
pub use discovery::{Finding, Query, Response};
pub use fetch::Fetched;
pub use index::{Hit, Stats};
pub use service::{Search, SearchError};

/// Release identity supplied by the release workflow; local builds keep the
/// manifest version.
pub const VERSION: &str = env!("SEARCH_BUILD_VERSION");
/// Update and state boundary. Local and PR builds never follow a release
/// channel.
pub const CHANNEL: &str = env!("SEARCH_BUILD_CHANNEL");
/// Exact source revision for published builds, or `local` for ordinary builds.
pub const COMMIT: &str = env!("SEARCH_BUILD_COMMIT");

/// The client name search identifies itself with to providers. Honest identity:
/// search names itself and does not impersonate a browser beyond the user
/// agent its own fetcher sends.
pub const CLIENT: &str = "search";
