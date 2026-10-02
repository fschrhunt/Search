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
use crate::fetch::{passages, Fetched, Passage, DEFAULT_BUDGET};

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
    /// One to five concise keyword queries. Each is searched independently and
    /// reported in order.
    pub queries: Vec<String>,
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
    /// What you are looking for in these pages. When given, only the passages
    /// that match are returned, which is far cheaper than the whole page.
    #[serde(default)]
    pub query: Option<String>,
    /// The most characters to return per page (default 6000, max 40000).
    #[serde(default)]
    pub max_characters: Option<usize>,
}

/// The JSON shape a search tool returns: one response per query, in order.
#[derive(serde::Serialize)]
struct SearchOutput {
    queries: Vec<Response>,
}

/// One fetched page, either whole or reduced to the passages a query matched.
#[derive(serde::Serialize)]
struct FocusedPage {
    #[serde(flatten)]
    page: Fetched,
    /// Present only when a query was given: the matching passages, replacing
    /// `text` as the thing to read.
    #[serde(skip_serializing_if = "Option::is_none")]
    passages: Option<Vec<Passage>>,
}

impl FocusedPage {
    /// Keep the whole text when no query narrowed it.
    fn whole(page: Fetched) -> Self {
        FocusedPage {
            page,
            passages: None,
        }
    }

    /// Reduce to the passages matching `query`; the whole text is dropped so the
    /// model is not tempted to read past the answer.
    fn build(mut page: Fetched, query: &str, budget: usize) -> Self {
        if query.trim().is_empty() || page.text.is_empty() {
            return Self::whole(page);
        }
        let found = passages(&page.text, query, budget);
        if found.is_empty() {
            return Self::whole(page);
        }
        // The full text stays in the index; the tool answer does not carry it.
        page.text = String::new();
        FocusedPage {
            page,
            passages: Some(found),
        }
    }
}

/// The combined answer the fetch tool returns.
#[derive(serde::Serialize)]
struct FetchOutput {
    pages: Vec<FocusedPage>,
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
        description = "Search the live web across several independent providers and return ranked results with title, URL, and snippet. Use it when you need current information, source discovery, or facts you are not confident about; do not use it for a page you already have a URL for — fetch that instead. The results are a starting point: read the few that matter with web_fetch before relying on them. Every answer names which providers responded, so an empty result is never mistaken for a broken one."
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
        json_result(SearchOutput { queries: responses })
    }

    #[tool(
        name = "web_fetch",
        description = "Read one or more URLs as clean, readable text, and add them to the local index. Pass a query to get only the passages that match it instead of the whole page — this is almost always what you want, and it is far cheaper. Reading is safe to repeat: a page read once is served instantly from the index. Public internet only: private and link-local addresses are refused."
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
        let budget = args
            .max_characters
            .unwrap_or(DEFAULT_BUDGET)
            .clamp(500, 40_000);
        let urls: Vec<String> = args.urls.into_iter().take(10).collect();
        let pages = self.service.fetch(&urls).await;

        // With a query, return only the matching passages; without one, the text.
        let query = args.query.unwrap_or_default();
        let focused: Vec<FocusedPage> = pages
            .into_iter()
            .map(|page| FocusedPage::build(page, &query, budget))
            .collect();
        json_result(FetchOutput { pages: focused })
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
