// Package mcp exposes the search core over the Model Context Protocol, so an
// agent gets web search and web reading as first-class tools with no bespoke
// integration. Two transports share one tool implementation: stdio for local
// agents that spawn the binary, and streamable HTTP for remote agents that reach
// it over a private network.
package mcp

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/fschrhunt/search/internal/engine"
	"github.com/fschrhunt/search/internal/fetch"
	"github.com/fschrhunt/search/internal/index"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Deps is what the tools need.
type Deps struct {
	Searcher *engine.Registry
	Fetcher  *fetch.Fetcher
	Index    *index.Store
	Version  string
}

// New builds the MCP server with both tools registered.
func New(deps Deps) *mcp.Server {
	srv := mcp.NewServer(&mcp.Implementation{
		Name:    "search",
		Version: deps.Version,
	}, &mcp.ServerOptions{
		Instructions: "Search the live web and read pages. Use web_search to find sources, then web_fetch to read the ones that matter. Results report which providers answered, so an empty answer is never mistaken for a broken one.",
	})

	mcp.AddTool(srv, &mcp.Tool{
		Name:        "web_search",
		Title:       "Web search",
		Description: "Search the live web across several independent providers. Returns ranked results with title, URL, and snippet. Prefer concise keyword queries; fetch the important URLs before relying on them.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, in searchInput) (*mcp.CallToolResult, any, error) {
		return deps.search(ctx, in)
	})

	mcp.AddTool(srv, &mcp.Tool{
		Name:        "web_fetch",
		Title:       "Read web pages",
		Description: "Read one or more URLs as clean text, and add them to the local index. Public internet only: private and link-local addresses are refused.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, in fetchInput) (*mcp.CallToolResult, any, error) {
		return deps.fetch(ctx, in)
	})

	return srv
}

// searchInput is the web_search argument shape.
type searchInput struct {
	Objective string   `json:"objective,omitempty" jsonschema:"the question or goal driving the search"`
	Queries   []string `json:"queries" jsonschema:"one to five concise keyword queries"`
	Limit     int      `json:"limit,omitempty" jsonschema:"maximum results per query (default 10, max 50)"`
	Engines   []string `json:"engines,omitempty" jsonschema:"restrict to specific providers, such as brave or wikipedia"`
}

// fetchInput is the web_fetch argument shape.
type fetchInput struct {
	URLs      []string `json:"urls" jsonschema:"one to ten http or https URLs to read"`
	Objective string   `json:"objective,omitempty" jsonschema:"the goal for why these URLs are being read"`
}

// searchResultJSON is the shape returned to the model.
type searchResultJSON struct {
	Objective string            `json:"objective,omitempty"`
	Queries   []engine.Response `json:"queries"`
}

func (d Deps) search(ctx context.Context, in searchInput) (*mcp.CallToolResult, any, error) {
	if len(in.Queries) == 0 {
		return nil, nil, fmt.Errorf("at least one query is required")
	}
	if len(in.Queries) > 5 {
		in.Queries = in.Queries[:5]
	}
	limit := in.Limit
	if limit <= 0 {
		limit = 10
	}
	out := searchResultJSON{Objective: in.Objective}
	for _, q := range in.Queries {
		q = strings.TrimSpace(q)
		if q == "" {
			continue
		}
		resp := d.Searcher.Search(ctx, engine.Options{Query: q, Limit: limit, Engines: in.Engines})
		out.Queries = append(out.Queries, resp)
	}
	return jsonResult(out)
}

type fetchResultJSON struct {
	Objective string         `json:"objective,omitempty"`
	Pages     []fetch.Result `json:"pages"`
}

func (d Deps) fetch(ctx context.Context, in fetchInput) (*mcp.CallToolResult, any, error) {
	if len(in.URLs) == 0 {
		return nil, nil, fmt.Errorf("at least one URL is required")
	}
	if len(in.URLs) > 10 {
		in.URLs = in.URLs[:10]
	}
	ctx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	pages := d.Fetcher.FetchMany(ctx, in.URLs)
	return jsonResult(fetchResultJSON{Objective: in.Objective, Pages: pages})
}

// jsonResult renders v as compact JSON text, which models parse reliably.
func jsonResult(v any) (*mcp.CallToolResult, any, error) {
	data, err := json.Marshal(v)
	if err != nil {
		return nil, nil, err
	}
	return &mcp.CallToolResult{
		Content: []mcp.Content{&mcp.TextContent{Text: string(data)}},
	}, v, nil
}
