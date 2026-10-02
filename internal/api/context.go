package api

import (
	"context"
	"errors"
	"net/http"
	"time"
)

var (
	errNoBody   = errors.New("request body is required")
	errBadJSON  = errors.New("request body is not valid JSON")
	errTrailing = errors.New("unexpected data after JSON body")
)

// contextWithTimeout bounds a handler by both the client's cancellation and a
// server-side ceiling, so a slow upstream cannot hold a connection open.
func contextWithTimeout(r *http.Request, d time.Duration) (context.Context, context.CancelFunc) {
	if d <= 0 {
		d = 30 * time.Second
	}
	return context.WithTimeout(r.Context(), d)
}
