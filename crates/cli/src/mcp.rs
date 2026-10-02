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
///
/// `allowed_host` is the authority the server is reached by (its bind address,
/// or the hostname a proxy forwards). rmcp validates the inbound `Host` against
/// this list to prevent DNS rebinding against a locally running server, so the
/// deployment must name itself here — a request whose `Host` is not listed is
/// refused. Loopback forms are always allowed.
pub fn mount(
    service: Arc<Service>,
    allowed_host: &str,
) -> StreamableHttpService<Server, LocalSessionManager> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if !allowed_host.is_empty() {
        hosts.push(allowed_host.to_string());
        // A `host:port` bind also answers by bare host for a proxy that strips
        // the port; keep both forms.
        if let Some((host, _)) = allowed_host.rsplit_once(':') {
            hosts.push(host.to_string());
        }
    }
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(hosts)
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
