// Package engine turns a query into ranked, deduplicated results by fanning out
// to several discovery providers in parallel. It owns no index of its own: the
// index package is a separate, optional freshness layer. Every engine is
// keyless by default; engines that need a key run only when one is configured.
package engine

import (
	"context"
	"encoding/json"
	"time"
)

// millis is a duration that marshals as whole milliseconds, which is what every
// client of this service wants to see.
type millis time.Duration

func (m millis) MarshalJSON() ([]byte, error) {
	return json.Marshal(time.Duration(m).Milliseconds())
}

// Result is one discovered page, normalized across providers.
type Result struct {
	Title      string  `json:"title"`
	URL        string  `json:"url"`
	Snippet    string  `json:"snippet,omitempty"`
	Engine     string  `json:"engine"`
	EngineRank int     `json:"engineRank"`
	Score      float64 `json:"score"`
	// Published is zero when the provider exposes no date.
	Published time.Time `json:"published,omitempty"`
}

// Options shape a single query.
type Options struct {
	Query     string
	Limit     int
	PerEngine int
	Engines   []string // restrict to these; empty means all enabled
	Freshness time.Duration
	Type      string // "", "web", "news", "code", "academic"
}

// Engine discovers pages for a query. Implementations must return promptly on
// context cancellation and never block past the deadline they are given.
type Engine interface {
	Name() string
	// Search runs one query. The returned slice is in the engine's own ranking
	// order; the registry combines and re-ranks.
	Search(ctx context.Context, query string, limit int) ([]Result, error)
}

// Response is the normalized answer the API returns.
type Response struct {
	Query    string        `json:"query"`
	Results  []Result      `json:"results"`
	Engines  []EngineState `json:"engines"`
	Duration millis        `json:"durationMs"`
	Cached   bool          `json:"cached,omitempty"`
}

// EngineState reports how one engine fared, so an agent can tell a genuinely
// empty result set from a provider that failed or was skipped.
type EngineState struct {
	Name    string `json:"name"`
	Status  string `json:"status"` // ok, error, timeout, skipped
	Count   int    `json:"count"`
	Error   string `json:"error,omitempty"`
	Elapsed millis `json:"elapsedMs"`
}
