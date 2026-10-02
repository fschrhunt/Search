//! The MCP tool handlers and the stdio transport.
//!
//! Two tools, `web_search` and `web_fetch`, matching the shapes models already
//! know. The stdio transport is what a locally spawned agent uses; the
//! streamable HTTP transport lives in the binary, sharing these same handlers.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::stdio;
use rmcp::{
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};

use crate::discovery::{Query, Response};
use crate::fetch::Fetched;

use super::Service;

/// The MCP server over the in-process service.
#[derive(Clone)]
pub struct Server {
    service: Arc<Service>,
    /// Read by the `#[tool_handler]` macro's generated dispatch, which clippy's
    /// field-usage analysis cannot see. Scoped allow, proof: macro-generated use.
    #[allow(dead_code)]
    tool_router: ToolRouter<Server>,
}

/// Arguments for `web_search`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// One to five concise keyword queries.
    pub queries: Vec<String>,
    /// The question or goal driving the search.
    #[serde(default)]
    pub objective: Option<String>,
    /// Maximum results per query (default 10, max 50).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to specific providers, such as "brave" or "wikipedia".
    #[serde(default)]
    pub providers: Option<Vec<String>>,
}

/// Arguments for `web_fetch`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FetchArgs {
    /// One to ten http or https URLs to read.
    pub urls: Vec<String>,
    /// The goal for why these URLs are being read.
    #[serde(default)]
    pub objective: Option<String>,
}

/// The JSON shape a search tool returns.
#[derive(serde::Serialize)]
struct SearchOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    objective: Option<String>,
    queries: Vec<Response>,
}

/// The JSON shape a fetch tool returns.
#[derive(serde::Serialize)]
struct FetchOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    objective: Option<String>,
    pages: Vec<Fetched>,
}

#[tool_router]
impl Server {
    /// Build the MCP server for `service`.
    pub fn new(service: Arc<Service>) -> Self {
        Server {
            service,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "web_search",
        description = "Search the live web across several independent providers. Returns ranked results with title, URL, and snippet. Prefer concise keyword queries; fetch the important URLs before relying on them."
    )]
    async fn web_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut queries = args.queries;
        queries.retain(|q| !q.trim().is_empty());
        if queries.is_empty() {
            return Err(McpError::invalid_params(
                "at least one query is required",
                None,
            ));
        }
        queries.truncate(5);
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let providers = args.providers.unwrap_or_default();

        let mut responses = Vec::with_capacity(queries.len());
        for text in queries {
            let response = self
                .service
                .search(Query {
                    text,
                    limit,
                    per_provider: limit,
                    providers: providers.clone(),
                })
                .await;
            responses.push(response);
        }
        json_result(SearchOutput {
            objective: args.objective,
            queries: responses,
        })
    }

    #[tool(
        name = "web_fetch",
        description = "Read one or more URLs as clean text, and add them to the local index. Public internet only: private and link-local addresses are refused."
    )]
    async fn web_fetch(
        &self,
        Parameters(args): Parameters<FetchArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.urls.is_empty() {
            return Err(McpError::invalid_params(
                "at least one URL is required",
                None,
            ));
        }
        let urls: Vec<String> = args.urls.into_iter().take(10).collect();
        let pages = self.service.fetch(&urls).await;
        json_result(FetchOutput {
            objective: args.objective,
            pages,
        })
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("search", crate::VERSION))
            .with_instructions(
                "Search the live web and read pages. Use web_search to find sources, then \
                 web_fetch to read the ones that matter. Results report which providers \
                 answered, so an empty answer is never mistaken for a broken one.",
            )
    }
}

/// Render a value as compact JSON in a success result.
fn json_result<T: serde::Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string(&value)
        .map_err(|e| McpError::internal_error(format!("serialize result: {e}"), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// Serve MCP over stdin/stdout until the client disconnects.
pub async fn serve_stdio(
    service: Arc<Service>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let running = Server::new(service).serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
