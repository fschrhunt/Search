package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"
	"time"
)

// brave queries Brave's HTML search page. Brave owns an independent index of
// 40B+ pages and answered well from this network, making it the primary general
// engine. It is keyless here; the API (with a key) is a future alternative.
type brave struct{}

func newBrave(key string) Engine { return &brave{} }

func (b *brave) Name() string { return "brave" }

func (b *brave) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	// Brave's JSON endpoint used by its own page is not public; the server
	// rendered page is scrape-friendly and stable enough to parse. Ask for more
	// than needed so ranking has headroom.
	u := "https://search.brave.com/search?q=" + url.QueryEscape(query) + "&source=web"
	body, _, err := get(ctx, u, nil)
	if err != nil {
		return nil, err
	}
	return parseBrave(body, limit), nil
}

// parseBrave reads result blocks out of Brave's server-rendered HTML. It is
// intentionally tolerant: it keys on result anchors and the snippet element,
// and skips anything it cannot classify rather than failing the whole page.
func parseBrave(body []byte, limit int) []Result {
	html := string(body)
	var out []Result
	seen := map[string]bool{}

	// Each web result carries data-type="web"; walk the block after each marker.
	blocks := splitBlocks(html, `data-type="web"`, `data-type="`)
	for _, block := range blocks {
		href := firstAttr(block, `href="`)
		title := text(firstByClass(block, "search-snippet-title"))
		if title == "" {
			title = text(matchTag(block, "title"))
		}
		snippet := text(firstByClass(block, "line-clamp-dynamic"))
		if snippet == "" {
			snippet = text(firstByClass(block, "generic-snippet"))
		}
		u := cleanURL(href)
		if u == "" || title == "" || seen[u] {
			continue
		}
		seen[u] = true
		out = append(out, Result{Title: title, URL: u, Snippet: snippet, Engine: "brave"})
		if len(out) >= limit {
			break
		}
	}
	if len(out) == 0 {
		// Fallback: any external anchor with nearby text.
		out = fallbackLinks(html, limit, "brave")
	}
	return out
}

// marginalia queries the independent Marginalia Search, which favors
// non-commercial and obscure pages. It complements Brave's mainstream results.
// The old-search host is used because the new site renders results client-side.
type marginalia struct{}

func newMarginalia() Engine { return &marginalia{} }

func (m *marginalia) Name() string { return "marginalia" }

func (m *marginalia) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := "https://old-search.marginalia.nu/search?query=" + url.QueryEscape(query)
	body, _, err := get(ctx, u, nil)
	if err != nil {
		return nil, err
	}
	return parseMarginalia(body, limit), nil
}

// parseMarginalia reads Marginalia's server-rendered result sections:
// <section class="card search-result"> with a title anchor and a URL div.
func parseMarginalia(body []byte, limit int) []Result {
	html := string(body)
	var out []Result
	seen := map[string]bool{}
	// Split on the result marker itself; each section runs to the next <section.
	parts := strings.Split(html, `class="card search-result"`)
	for _, block := range parts[1:] {
		if end := strings.Index(block, "<section"); end >= 0 {
			block = block[:end]
		}
		u := cleanURL(firstAttr(block, `href="`))
		if u == "" || !strings.HasPrefix(u, "http") || seen[u] {
			continue
		}
		title := text(matchTag(block, "h2"))
		if title == "" {
			title = text(matchTag(block, "a"))
		}
		seen[u] = true
		out = append(out, Result{Title: title, URL: u, Engine: "marginalia"})
		if len(out) >= limit {
			break
		}
	}
	return out
}

// mwmbl is a community-crawled independent index with a JSON API.
type mwmbl struct{}

func newMwmbl() Engine { return &mwmbl{} }

func (m *mwmbl) Name() string { return "mwmbl" }

func (m *mwmbl) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := "https://api.mwmbl.org/search/?s=" + url.QueryEscape(query)
	body, _, err := get(ctx, u, map[string]string{"Accept": "application/json"})
	if err != nil {
		return nil, err
	}
	var rows []struct {
		URL   string `json:"url"`
		Title []struct {
			Value string `json:"value"`
		} `json:"title"`
		Extract []struct {
			Value string `json:"value"`
		} `json:"extract"`
	}
	if err := json.Unmarshal(body, &rows); err != nil {
		return nil, fmt.Errorf("mwmbl json: %w", err)
	}
	var out []Result
	for _, r := range rows {
		u := cleanURL(r.URL)
		if u == "" {
			continue
		}
		res := Result{URL: u, Engine: "mwmbl"}
		if len(r.Title) > 0 {
			res.Title = r.Title[0].Value
		}
		if len(r.Extract) > 0 {
			res.Snippet = r.Extract[0].Value
		}
		out = append(out, res)
		if len(out) >= limit {
			break
		}
	}
	return out, nil
}

// published parses a few common date layouts defensively.
func published(s string) time.Time {
	s = strings.TrimSpace(s)
	for _, layout := range []string{time.RFC3339, "2006-01-02", "2006-01-02T15:04:05", "Jan 2, 2006", "2 Jan 2006"} {
		if t, err := time.Parse(layout, s); err == nil {
			return t
		}
	}
	return time.Time{}
}
