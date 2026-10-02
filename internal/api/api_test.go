package api

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestRequireAuthBlocksUnauthenticated pins that a wrapped handler — such as the
// MCP endpoint — cannot be reached without the bearer token.
func TestRequireAuthBlocksUnauthenticated(t *testing.T) {
	s := New(Options{Token: "secret-token-value"})
	reached := false
	next := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { reached = true; w.WriteHeader(http.StatusOK) })
	guarded := s.RequireAuth(next)

	// No header: refused, and the inner handler never runs.
	rec := httptest.NewRecorder()
	guarded.ServeHTTP(rec, httptest.NewRequest(http.MethodPost, "/mcp", nil))
	if rec.Code != http.StatusUnauthorized || reached {
		t.Fatalf("unauthenticated request: code=%d reached=%v", rec.Code, reached)
	}

	// Wrong token: still refused.
	rec = httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/mcp", nil)
	req.Header.Set("Authorization", "Bearer wrong")
	guarded.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("wrong token: code=%d", rec.Code)
	}

	// Correct token: reaches the handler.
	rec = httptest.NewRecorder()
	req = httptest.NewRequest(http.MethodPost, "/mcp", nil)
	req.Header.Set("Authorization", "Bearer secret-token-value")
	guarded.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK || !reached {
		t.Fatalf("correct token: code=%d reached=%v", rec.Code, reached)
	}
}

// TestNoTokenDeniesEverything pins that a server with no configured token is
// closed, not open.
func TestNoTokenDeniesEverything(t *testing.T) {
	s := New(Options{Token: ""})
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/v1/status", nil)
	req.Header.Set("Authorization", "Bearer anything")
	s.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 with no configured token, got %d", rec.Code)
	}
}
