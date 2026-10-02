//! The streamable HTTP transport for MCP, mounted at `/mcp`.
//!
//! The tool handlers live in `search_core::search`; this file only adapts them
//! to the HTTP transport and its session manager. It is the one place that
//! names rmcp's HTTP tower service.

use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use search_core::search::{Server, Service};

/// Build the tower service that serves MCP over streamable HTTP. A new
/// `Server` is created per session, sharing the one `Service`.
pub fn mount(service: Arc<Service>) -> StreamableHttpService<Server, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default()
        // One request body is at most a small JSON-RPC envelope; bound it so a
        // hostile client cannot stream an unbounded frame.
        .with_max_request_body_bytes(1 << 20)
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(30)));
    StreamableHttpService::new(
        move || Ok(Server::new(Arc::clone(&service))),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}
