package engine

import (
	"context"
	"errors"
	"log/slog"
	"net/url"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/fschrhunt/search/internal/config"
)

// Registry fans a query out to every enabled engine in parallel and merges the
// answers. It is the only place engine failure is interpreted: a failing engine
// never fails the query, it is reported in EngineState beside the results.
type Registry struct {
	engines []Engine
	cfg     config.SearchConfig
	log     *slog.Logger
}

// New builds the registry from configuration. Engines whose required key is
// missing are dropped with a log line rather than failing startup, so a key
// rotation can leave the service running on its keyless engines.
func New(engCfg config.EngineConfig, searchCfg config.SearchConfig, log *slog.Logger) (*Registry, error) {
	enabled := map[string]bool{}
	for _, name := range engCfg.Enabled {
		enabled[strings.ToLower(strings.TrimSpace(name))] = true
	}
	allow := func(name string) bool { return len(enabled) == 0 || enabled[name] }

	all := []Engine{
		newBrave(engCfg.Keys["brave"]),
		newMarginalia(),
		newMwmbl(),
		newWikipedia(),
		newHN(),
		newSE(),
		newArxiv(),
	}

	r := &Registry{cfg: searchCfg, log: log}
	for _, e := range all {
		if !allow(e.Name()) {
			continue
		}
		if k, needs := e.(keyed); needs && k.MissingKey() {
			log.Warn("engine disabled: no key configured", "engine", e.Name())
			continue
		}
		r.engines = append(r.engines, e)
	}
	if len(r.engines) == 0 {
		return nil, errors.New("no engines enabled")
	}
	return r, nil
}

// keyed is implemented by engines that require an API key.
type keyed interface{ MissingKey() bool }

// Names returns the enabled engine names, for status output.
func (r *Registry) Names() []string {
	out := make([]string, 0, len(r.engines))
	for _, e := range r.engines {
		out = append(out, e.Name())
	}
	return out
}

// OverallTimeout is the server-side ceiling for one query.
func (r *Registry) OverallTimeout() time.Duration { return r.cfg.OverallTimeout }

// Search runs every selected engine concurrently, each under its own timeout,
// and returns a merged ranking plus per-engine state. The overall deadline is
// the caller's context, so a client cancel stops all engines at once.
func (r *Registry) Search(ctx context.Context, opt Options) Response {
	limit := opt.Limit
	if limit <= 0 || limit > r.cfg.MaxResults {
		limit = r.cfg.MaxResults
	}
	perEngine := opt.PerEngine
	if perEngine <= 0 {
		perEngine = limit
	}

	selected := r.selectEngines(opt.Engines)
	type outcome struct {
		state   EngineState
		results []Result
	}
	outcomes := make([]outcome, len(selected))

	start := time.Now()
	var wg sync.WaitGroup
	for i, e := range selected {
		wg.Add(1)
		go func(i int, e Engine) {
			defer wg.Done()
			start := time.Now()
			ectx, cancel := context.WithTimeout(ctx, r.cfg.MaxEngineTime)
			defer cancel()
			results, err := e.Search(ectx, opt.Query, perEngine)
			elapsed := time.Since(start)
			st := EngineState{Name: e.Name(), Elapsed: millis(elapsed), Count: len(results)}
			switch {
			case err == nil:
				st.Status = "ok"
			case errors.Is(err, context.DeadlineExceeded) && ctx.Err() == nil:
				st.Status = "timeout"
				st.Error = "engine exceeded its deadline"
			case errors.Is(err, context.Canceled) && ctx.Err() != nil:
				st.Status = "canceled"
			default:
				st.Status = "error"
				st.Error = err.Error()
				r.log.Debug("engine failed", "engine", e.Name(), "err", err)
			}
			outcomes[i] = outcome{state: st, results: results}
		}(i, e)
	}
	wg.Wait()

	var (
		states []EngineState
		merged []Result
	)
	for _, o := range outcomes {
		states = append(states, o.state)
		if len(o.results) == 0 {
			continue
		}
		// The best rank an engine gave a URL is its vote.
		for i := range o.results {
			o.results[i].EngineRank = i + 1
		}
		merged = append(merged, o.results...)
	}

	sort.SliceStable(states, func(i, j int) bool { return states[i].Name < states[j].Name })
	ranked := rank(merged, limit)
	return Response{Query: opt.Query, Results: ranked, Engines: states, Duration: millis(time.Since(start))}
}

// selectEngines restricts to the requested names, preserving registry order.
// Unknown names are ignored so a stale client never errors.
func (r *Registry) selectEngines(names []string) []Engine {
	if len(names) == 0 {
		return r.engines
	}
	want := map[string]bool{}
	for _, n := range names {
		want[strings.ToLower(strings.TrimSpace(n))] = true
	}
	var out []Engine
	for _, e := range r.engines {
		if want[e.Name()] {
			out = append(out, e)
		}
	}
	if len(out) == 0 {
		return r.engines
	}
	return out
}

// rank deduplicates by normalized URL and applies reciprocal-rank fusion: each
// engine contributes 1/(k + rank), so a URL that several independent engines
// rank well rises above one that only a single engine liked. Ties keep first
// appearance order, which preserves provider intent for the head of the list.
func rank(results []Result, limit int) []Result {
	const k = 10.0
	type agg struct {
		result     Result
		score      float64
		order      int
		firstTitle string
	}
	seen := map[string]*agg{}
	order := 0
	for _, res := range results {
		key := canonicalURL(res.URL)
		if key == "" {
			continue
		}
		a := seen[key]
		if a == nil {
			a = &agg{result: res, order: order, firstTitle: res.Title}
			seen[key] = a
			order++
		}
		a.score += 1.0 / (k + float64(res.EngineRank))
		// Prefer a non-empty snippet and title from whichever engine supplied one.
		if a.result.Snippet == "" && res.Snippet != "" {
			a.result.Snippet = res.Snippet
		}
		if a.result.Title == "" && res.Title != "" {
			a.result.Title = res.Title
		}
		if a.result.Published.IsZero() && !res.Published.IsZero() {
			a.result.Published = res.Published
		}
	}

	out := make([]Result, 0, len(seen))
	for _, a := range seen {
		a.result.Score = a.score
		a.result.Engine = strings.Join(engineList(results, a.result.URL), ",")
		out = append(out, a.result)
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Score != out[j].Score {
			return out[i].Score > out[j].Score
		}
		return false
	})
	if len(out) > limit {
		out = out[:limit]
	}
	return out
}

// engineList names the engines that returned a given URL, for transparency.
func engineList(results []Result, url string) []string {
	key := canonicalURL(url)
	seen := map[string]bool{}
	var names []string
	for _, r := range results {
		if canonicalURL(r.URL) == key && !seen[r.Engine] {
			seen[r.Engine] = true
			names = append(names, r.Engine)
		}
	}
	sort.Strings(names)
	return names
}

// canonicalURL normalizes a URL for deduplication: lowercased host, no fragment,
// no tracking query parameters, no trailing slash. It never drops the query.
func canonicalURL(raw string) string {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") {
		return ""
	}
	u.Scheme = strings.ToLower(u.Scheme)
	u.Host = strings.ToLower(u.Host)
	u.Fragment = ""
	q := u.Query()
	for _, p := range []string{"utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content", "ref", "fbclid", "gclid", "mc_cid", "mc_eid"} {
		q.Del(p)
	}
	u.RawQuery = q.Encode()
	u.Path = strings.TrimSuffix(u.Path, "/")
	return u.String()
}
