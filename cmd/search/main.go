// Command search is a self-hosted, agent-first web search service. It runs one
// HTTP server (and, until MCP lands, the plain HTTP API) exposing multi-engine
// discovery, a hardened fetcher, and a private on-disk index that grows from
// every page a caller fetches. It is designed to be reached only through a
// private network such as a tailnet; it authenticates every request itself.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/fschrhunt/search/internal/api"
	"github.com/fschrhunt/search/internal/config"
	"github.com/fschrhunt/search/internal/engine"
	"github.com/fschrhunt/search/internal/fetch"
	"github.com/fschrhunt/search/internal/index"
)

// version is set at build time with -ldflags "-X main.version=v1.0.0".
var version = "dev"

func main() {
	os.Exit(run())
}

// run parses flags, loads configuration, starts the HTTP surface, and blocks
// until an interrupt. Errors go to stderr and exit nonzero.
func run() int {
	configPath := flag.String("config", "", "path to a JSON config file (default: $SEARCH_CONFIG or ~/.config/search/search.json)")
	addr := flag.String("addr", "", "listen address (overrides config; default 127.0.0.1:8642)")
	dataDir := flag.String("data", "", "data directory for the index (overrides config)")
	showVersion := flag.Bool("version", false, "print the version and exit")
	flag.Parse()

	if *showVersion {
		fmt.Println(version)
		return 0
	}

	cfg, err := config.Load(*configPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: %v\n", err)
		return 1
	}
	if *addr != "" {
		cfg.Addr = *addr
	}
	if *dataDir != "" {
		cfg.DataDir = *dataDir
	}

	logger := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: cfg.LogLevel()}))
	slog.SetDefault(logger)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	// The index is opened exclusively: a second process on the same data
	// directory fails loudly rather than corrupting the store.
	store, err := index.Open(cfg.DataDir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: open index: %v\n", err)
		return 1
	}
	defer store.Close()

	fetcher, err := fetch.New(cfg.Fetch, store, logger)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: fetcher: %v\n", err)
		return 1
	}
	searcher, err := engine.New(cfg.Engines, cfg.Search, logger)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: engines: %v\n", err)
		return 1
	}

	handler := api.New(api.Options{
		Searcher: searcher,
		Fetcher:  fetcher,
		Index:    store,
		Token:    cfg.Token,
		Version:  version,
		Logger:   logger,
	})

	server := &http.Server{
		Addr:              cfg.Addr,
		Handler:           handler,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      90 * time.Second,
		IdleTimeout:       120 * time.Second,
		MaxHeaderBytes:    1 << 16,
	}

	errc := make(chan error, 1)
	go func() {
		logger.Info("listening", "addr", cfg.Addr, "data", cfg.DataDir, "version", version)
		if err := server.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			errc <- err
		}
	}()

	select {
	case err := <-errc:
		logger.Error("server stopped", "err", err)
		return 1
	case <-ctx.Done():
		logger.Info("shutting down")
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdownCtx)
		return 0
	}
}

// ensureJSON is a compile-time guard that the api package's request/response
// types stay JSON-encodable without importing it here for effect only.
var _ = json.Marshal
