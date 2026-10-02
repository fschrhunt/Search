//! The search engine, terminal-free and frontend-free: discovery across
//! several independent providers, a hardened fetcher, a private on-disk index,
//! and the MCP surface that exposes search and fetch as tools.
//!
//! The binary is a separate crate that depends on this one. Nothing here names
//! a terminal or a transport the binary owns; this crate's Rust items are not a
//! stable third-party API.
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
pub mod search;
pub mod text;

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
