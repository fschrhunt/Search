package fetch

import (
	"io"
	"log/slog"
)

// discardLogger keeps tests quiet.
func discardLogger() *slog.Logger {
	return slog.New(slog.NewTextHandler(io.Discard, nil))
}
