// Package api is the service's HTTP surface: a small JSON API that the MCP
// surface will front. Every request is authenticated, every request body is
// bounded, and responses carry per-engine state so callers can tell an empty
// result from a failed provider. It is designed to sit behind a private network
// (a tailnet) and to defend itself if it is ever exposed.
package api

import (
	"crypto/subtle"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/fschrhunt/search/internal/engine"
	"github.com/fschrhunt/search/internal/fetch"
	"github.com/fschrhunt/search/internal/index"
)

// Options carries the wired dependencies.
type Options struct {
	Searcher *engine.Registry
	Fetcher  *fetch.Fetcher
	Index    *index.Store
	Token    string
	Version  string
	Logger   *slog.Logger
}

// Server implements http.Handler.
type Server struct {
	opt Options
}

// New returns the handler with routes mounted.
func New(opt Options) *Server {
	return &Server{opt: opt}
}

// ServeHTTP authenticates and routes. Authentication precedes routing so no
// endpoint, including errors, leaks whether a path exists to an unauthenticated
// caller.
func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	start := time.Now()
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Referrer-Policy", "no-referrer")

	if !s.authorized(r) {
		w.Header().Set("WWW-Authenticate", `Bearer realm="search"`)
		s.fail(w, http.StatusUnauthorized, "unauthorized")
		return
	}

	switch {
	case r.Method == http.MethodGet && r.URL.Path == "/healthz":
		s.ok(w, map[string]any{"status": "ok", "version": s.opt.Version, "uptimeMs": time.Since(start).Milliseconds()})
	case r.Method == http.MethodGet && r.URL.Path == "/v1/status":
		s.status(w)
	case r.Method == http.MethodGet && r.URL.Path == "/v1/search":
		s.search(w, r)
	case r.Method == http.MethodGet && r.URL.Path == "/v1/index":
		s.indexSearch(w, r)
	case r.Method == http.MethodPost && r.URL.Path == "/v1/fetch":
		s.fetch(w, r)
	default:
		s.fail(w, http.StatusNotFound, "not found")
	}
}

// authorized checks the bearer token in constant time. A missing token on the
// server side is a configuration error, not an open door: it denies everything.
func (s *Server) authorized(r *http.Request) bool {
	if s.opt.Token == "" {
		return false
	}
	header := r.Header.Get("Authorization")
	const prefix = "Bearer "
	if !strings.HasPrefix(header, prefix) {
		return false
	}
	got := strings.TrimSpace(strings.TrimPrefix(header, prefix))
	return subtle.ConstantTimeCompare([]byte(got), []byte(s.opt.Token)) == 1
}

// RequireAuth wraps any handler with the same bearer check the JSON API uses, so
// the MCP endpoint cannot be reached unauthenticated.
func (s *Server) RequireAuth(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !s.authorized(r) {
			w.Header().Set("WWW-Authenticate", `Bearer realm="search"`)
			s.fail(w, http.StatusUnauthorized, "unauthorized")
			return
		}
		next.ServeHTTP(w, r)
	})
}

// search handles GET /v1/search.
func (s *Server) search(w http.ResponseWriter, r *http.Request) {
	q := strings.TrimSpace(r.URL.Query().Get("q"))
	if q == "" {
		s.fail(w, http.StatusBadRequest, "missing query parameter q")
		return
	}
	if len(q) > 512 {
		s.fail(w, http.StatusBadRequest, "query too long")
		return
	}
	limit := intParam(r, "limit", 10, 1, 50)
	perEngine := intParam(r, "perEngine", limit, 1, 30)

	ctx, cancel := contextWithTimeout(r, s.opt.Searcher.OverallTimeout())
	defer cancel()

	resp := s.opt.Searcher.Search(ctx, engine.Options{
		Query:     q,
		Limit:     limit,
		PerEngine: perEngine,
		Engines:   splitCSV(r.URL.Query().Get("engines")),
	})
	s.ok(w, resp)
}

// indexSearch handles GET /v1/index: search only what we have already fetched.
func (s *Server) indexSearch(w http.ResponseWriter, r *http.Request) {
	q := strings.TrimSpace(r.URL.Query().Get("q"))
	if q == "" {
		s.fail(w, http.StatusBadRequest, "missing query parameter q")
		return
	}
	limit := intParam(r, "limit", 10, 1, 50)
	hits, err := s.opt.Index.Search(q, limit)
	if err != nil {
		s.opt.Logger.Error("index search", "err", err)
		s.fail(w, http.StatusInternalServerError, "index search failed")
		return
	}
	s.ok(w, map[string]any{"query": q, "results": hits, "count": len(hits)})
}

// fetchRequest is the POST /v1/fetch body.
type fetchRequest struct {
	URLs []string `json:"urls"`
}

// fetch handles POST /v1/fetch.
func (s *Server) fetch(w http.ResponseWriter, r *http.Request) {
	var req fetchRequest
	if err := decodeJSON(w, r, &req); err != nil {
		s.fail(w, http.StatusBadRequest, err.Error())
		return
	}
	if len(req.URLs) == 0 {
		s.fail(w, http.StatusBadRequest, "no urls given")
		return
	}
	if len(req.URLs) > 10 {
		s.fail(w, http.StatusBadRequest, "at most 10 urls per request")
		return
	}
	ctx, cancel := contextWithTimeout(r, 60*time.Second)
	defer cancel()
	results := s.opt.Fetcher.FetchMany(ctx, req.URLs)
	s.ok(w, map[string]any{"results": results, "count": len(results)})
}

// status reports engine and corpus health.
func (s *Server) status(w http.ResponseWriter) {
	stats, err := s.opt.Index.Stats()
	if err != nil {
		s.opt.Logger.Error("index stats", "err", err)
	}
	s.ok(w, map[string]any{
		"version": s.opt.Version,
		"engines": s.opt.Searcher.Names(),
		"index":   stats,
	})
}

// ok writes a JSON success envelope.
func (s *Server) ok(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
}

// fail writes a JSON error envelope.
func (s *Server) fail(w http.ResponseWriter, code int, msg string) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(code)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(map[string]any{"error": msg})
}

// decodeJSON reads a bounded JSON body and rejects unknown fields and trailing
// data, so a malformed request fails clearly instead of half-applying.
func decodeJSON(w http.ResponseWriter, r *http.Request, v any) error {
	r.Body = http.MaxBytesReader(w, r.Body, 1<<20)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		if err == io.EOF {
			return errNoBody
		}
		return errBadJSON
	}
	if dec.More() {
		return errTrailing
	}
	return nil
}

// intParam reads a bounded integer query parameter.
func intParam(r *http.Request, name string, def, lo, hi int) int {
	v := r.URL.Query().Get(name)
	if v == "" {
		return def
	}
	n, err := strconv.Atoi(v)
	if err != nil || n < lo {
		return def
	}
	if n > hi {
		return hi
	}
	return n
}

// splitCSV splits a comma-separated parameter, dropping blanks.
func splitCSV(v string) []string {
	if v == "" {
		return nil
	}
	var out []string
	for _, p := range strings.Split(v, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}
