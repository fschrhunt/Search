//! The HTTP surface: a small JSON API and the streamable MCP endpoint on one
//! listener.
//!
//! Every request passes the same bearer check before it is routed, so no path —
//! including errors — reveals itself to an unauthenticated caller. The MCP
//! endpoint is behind that same check.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use search_core::discovery::Query;
use search_core::search::Service;

use crate::mcp;
use crate::run::build_service;

/// Serve the JSON API and MCP over HTTP until interrupted.
pub async fn serve(
    config_path: Option<String>,
    addr: Option<String>,
    data_dir: Option<String>,
) -> i32 {
    let service = match build_service(config_path, addr, data_dir) {
        Ok(service) => service,
        Err(message) => {
            eprintln!("search: {message}");
            return 1;
        }
    };
    if service.config().resolved_token().is_none() {
        eprintln!(
            "search: no token configured; set the variable named by tokenEnv (default {})",
            search_core::config::DEFAULT_TOKEN_ENV
        );
        return 1;
    }
    let listener = match tokio::net::TcpListener::bind(&service.config().addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("search: bind {}: {error}", service.config().addr);
            return 1;
        }
    };
    eprintln!(
        "search: listening on {} (data {}, version {})",
        service.config().addr,
        service.config().data_dir.display(),
        search_core::VERSION
    );
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    if let Err(error) = axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("search: server stopped: {error}");
        return 1;
    }
    0
}

/// Build the router. The auth layer wraps every route, MCP included. The token
/// is resolved once here, so a per-request check never reads the environment.
fn router(service: Arc<Service>) -> Router {
    let allowed_host = service.config().addr.clone();
    let mcp = mcp::mount(Arc::clone(&service), &allowed_host);
    let token = service.config().resolved_token().unwrap_or_default();
    let guard = AuthGuard {
        expected: Arc::new(token),
    };
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/status", get(status))
        .route("/v1/search", get(search))
        .route("/v1/index", get(index_search))
        .route("/v1/fetch", post(fetch))
        // The MCP endpoint is nested so its own paths stay under /mcp, and it
        // sits inside the same auth layer as everything else.
        .nest_service("/mcp", mcp)
        .layer(axum::middleware::from_fn_with_state(guard, auth))
        .with_state(service)
}

/// The resolved token the auth layer compares against, held once per server.
#[derive(Clone)]
struct AuthGuard {
    expected: Arc<String>,
}

/// Reject any request without the configured bearer token, in constant time.
async fn auth(
    State(guard): State<AuthGuard>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // A server with no token denies everything.
    if guard.expected.is_empty() {
        return unauthorized();
    }
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    if !constant_time_eq(presented.as_bytes(), guard.expected.as_bytes()) {
        return unauthorized();
    }
    next.run(request).await
}

/// A 401 with the standard challenge.
fn unauthorized() -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response.headers_mut().insert(
        axum::http::header::WWW_AUTHENTICATE,
        axum::http::HeaderValue::from_static("Bearer realm=\"search\""),
    );
    response
}

/// Compare two byte slices in constant time. Lengths are allowed to differ in
/// timing; the token value never is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Query parameters for the search endpoint.
#[derive(serde::Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    providers: Option<String>,
}

/// `GET /v1/search`.
async fn search(
    State(service): State<Arc<Service>>,
    axum::extract::Query(params): axum::extract::Query<SearchParams>,
) -> Response {
    let query = params.q.trim();
    if query.is_empty() {
        return bad_request("missing query parameter q");
    }
    if query.len() > 512 {
        return bad_request("query too long");
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 50);
    let providers = params
        .providers
        .map(|p| {
            p.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let response = service
        .search(Query {
            text: query.to_string(),
            limit,
            per_provider: limit,
            providers,
        })
        .await;
    Json(response).into_response()
}

/// Query parameters for the index endpoint.
#[derive(serde::Deserialize)]
struct IndexParams {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /v1/index`.
async fn index_search(
    State(service): State<Arc<Service>>,
    axum::extract::Query(params): axum::extract::Query<IndexParams>,
) -> Response {
    let query = params.q.trim();
    if query.is_empty() {
        return bad_request("missing query parameter q");
    }
    if query.len() > 512 {
        return bad_request("query too long");
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 50);
    match service.index_search(query, limit) {
        Ok(hits) => Json(serde_json::json!({
            "query": query,
            "results": hits,
            "count": hits.len(),
        }))
        .into_response(),
        Err(error) => {
            eprintln!("search: index search: {error}");
            status_error(StatusCode::INTERNAL_SERVER_ERROR, "index search failed")
        }
    }
}

/// Body for the fetch endpoint.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchBody {
    urls: Vec<String>,
}

/// `POST /v1/fetch`.
async fn fetch(State(service): State<Arc<Service>>, Json(body): Json<FetchBody>) -> Response {
    if body.urls.is_empty() {
        return bad_request("no urls given");
    }
    if body.urls.len() > 10 {
        return bad_request("at most 10 urls per request");
    }
    let pages = service.fetch(&body.urls).await;
    Json(serde_json::json!({"results": pages, "count": pages.len()})).into_response()
}

/// `GET /v1/status`.
async fn status(State(service): State<Arc<Service>>) -> Response {
    let stats = service.index_stats().ok();
    Json(serde_json::json!({
        "version": search_core::VERSION,
        "providers": service.provider_names(),
        "index": stats,
    }))
    .into_response()
}

/// `GET /healthz`.
async fn health() -> Response {
    Json(serde_json::json!({"status": "ok", "version": search_core::VERSION})).into_response()
}

/// A JSON 400.
fn bad_request(message: &str) -> Response {
    status_error(StatusCode::BAD_REQUEST, message)
}

/// A JSON error with a status.
fn status_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bearer check must accept the exact token and refuse everything else,
    /// including a different token of the same length.
    #[test]
    fn constant_time_compare_matches_exactly() {
        assert!(constant_time_eq(
            b"secret-token-value",
            b"secret-token-value"
        ));
        assert!(!constant_time_eq(
            b"secret-token-value",
            b"secret-token-valuX"
        ));
        assert!(!constant_time_eq(b"secret-token-value", b"short"));
        assert!(!constant_time_eq(b"", b"secret"));
    }
}
