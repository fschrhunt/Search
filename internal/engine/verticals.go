package engine

import (
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"html"
	"net/url"
	"strings"
)

// wikipedia uses the MediaWiki search API: keyless, fast, and authoritative for
// entity and concept queries.
type wikipedia struct{}

func newWikipedia() Engine { return &wikipedia{} }

func (w *wikipedia) Name() string { return "wikipedia" }

func (w *wikipedia) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := fmt.Sprintf("https://en.wikipedia.org/w/api.php?action=query&list=search&format=json&srsearch=%s&srlimit=%d",
		url.QueryEscape(query), clamp(limit, 1, 20))
	body, _, err := get(ctx, u, map[string]string{"Accept": "application/json"})
	if err != nil {
		return nil, err
	}
	var payload struct {
		Query struct {
			Search []struct {
				Title   string `json:"title"`
				Snippet string `json:"snippet"`
			} `json:"search"`
		} `json:"query"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, fmt.Errorf("wikipedia json: %w", err)
	}
	var out []Result
	for _, r := range payload.Query.Search {
		out = append(out, Result{
			Title:   r.Title,
			URL:     "https://en.wikipedia.org/wiki/" + url.PathEscape(strings.ReplaceAll(r.Title, " ", "_")),
			Snippet: stripTags(r.Snippet),
			Engine:  "wikipedia",
		})
	}
	return out, nil
}

// hackernews searches Hacker News stories and comments through the Algolia API,
// which is ideal for developer and startup queries.
type hackernews struct{}

func newHN() Engine { return &hackernews{} }

func (h *hackernews) Name() string { return "hackernews" }

func (h *hackernews) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := fmt.Sprintf("https://hn.algolia.com/api/v1/search?query=%s&hitsPerPage=%d",
		url.QueryEscape(query), clamp(limit, 1, 30))
	body, _, err := get(ctx, u, map[string]string{"Accept": "application/json"})
	if err != nil {
		return nil, err
	}
	var payload struct {
		Hits []struct {
			Title     string `json:"title"`
			URL       string `json:"url"`
			StoryText string `json:"story_text"`
			ObjectID  string `json:"objectID"`
			Points    int    `json:"points"`
		} `json:"hits"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, fmt.Errorf("hn json: %w", err)
	}
	var out []Result
	for _, hit := range payload.Hits {
		link := hit.URL
		if link == "" {
			link = "https://news.ycombinator.com/item?id=" + hit.ObjectID
		}
		title := hit.Title
		if title == "" {
			title = "HN discussion"
		}
		out = append(out, Result{
			Title:   title,
			URL:     link,
			Snippet: stripTags(hit.StoryText),
			Engine:  "hackernews",
		})
	}
	return out, nil
}

// stackexchange searches Stack Overflow and friends through the Stack Exchange
// API: the highest-signal source for concrete programming questions.
type stackexchange struct{}

func newSE() Engine { return &stackexchange{} }

func (s *stackexchange) Name() string { return "stackexchange" }

func (s *stackexchange) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := fmt.Sprintf("https://api.stackexchange.com/2.3/search/advanced?order=desc&sort=relevance&q=%s&site=stackoverflow&pagesize=%d&filter=default",
		url.QueryEscape(query), clamp(limit, 1, 30))
	body, _, err := get(ctx, u, map[string]string{"Accept": "application/json"})
	if err != nil {
		return nil, err
	}
	var payload struct {
		Items []struct {
			Title string `json:"title"`
			Link  string `json:"link"`
			Body  string `json:"body_markdown"`
			Score int    `json:"score"`
		} `json:"items"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, fmt.Errorf("stackexchange json: %w", err)
	}
	var out []Result
	for _, item := range payload.Items {
		out = append(out, Result{
			Title:   html.UnescapeString(item.Title),
			URL:     item.Link,
			Snippet: truncate(stripTags(item.Body), 300),
			Engine:  "stackexchange",
		})
	}
	return out, nil
}

// arxiv searches preprints through the arXiv Atom API.
type arxiv struct{}

func newArxiv() Engine { return &arxiv{} }

func (a *arxiv) Name() string { return "arxiv" }

func (a *arxiv) Search(ctx context.Context, query string, limit int) ([]Result, error) {
	u := fmt.Sprintf("https://export.arxiv.org/api/query?search_query=all:%s&max_results=%d",
		url.QueryEscape(query), clamp(limit, 1, 20))
	body, _, err := get(ctx, u, map[string]string{"Accept": "application/atom+xml"})
	if err != nil {
		return nil, err
	}
	var feed struct {
		Entries []struct {
			Title     string `xml:"title"`
			ID        string `xml:"id"`
			Summary   string `xml:"summary"`
			Published string `xml:"published"`
		} `xml:"entry"`
	}
	if err := xml.Unmarshal(body, &feed); err != nil {
		return nil, fmt.Errorf("arxiv xml: %w", err)
	}
	var out []Result
	for _, e := range feed.Entries {
		out = append(out, Result{
			Title:     strings.Join(strings.Fields(e.Title), " "),
			URL:       strings.TrimSpace(e.ID),
			Snippet:   truncate(strings.Join(strings.Fields(e.Summary), " "), 300),
			Engine:    "arxiv",
			Published: published(e.Published),
		})
	}
	return out, nil
}

// clamp keeps a limit inside an engine's accepted range.
func clamp(v, lo, hi int) int {
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}
