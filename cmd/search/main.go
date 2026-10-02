// Command search is a self-hosted, agent-first web search service. It fans a
// query out to several independent providers, merges the results, reads pages
// through a hardened guard, and indexes everything it reads into a private
// corpus. It speaks MCP over stdio and HTTP, and a small JSON API alongside.
//
// It is designed to run on a personal server and be reached over a private
// network such as a tailnet; it authenticates every network request itself.
package main

import (
	"context"
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
	"github.com/fschrhunt/search/internal/mcp"
	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
)

// version is set at build time with -ldflags "-X main.version=v1.0.0".
var version = "dev"

func main() { os.Exit(run(os.Args[1:])) }

// run dispatches a subcommand: no arguments serves MCP over stdio (what an agent
// spawns), and "serve" starts the HTTP surfaces.
func run(args []string) int {
	if len(args) > 0 {
		switch args[0] {
		case "serve":
			return serveHTTP(args[1:])
		case "stdio":
			return serveStdio(args[1:])
		case "version", "--version", "-version":
			fmt.Println(version)
			return 0
		case "help", "--help", "-h":
			usage()
			return 0
		}
		// No subcommand but flags present: default to stdio so harness configs
		// that pass flags still work.
		if args[0] == "" || args[0][0] != '-' {
			fmt.Fprintf(os.Stderr, "search: unknown command %q\n", args[0])
			usage()
			return 2
		}
	}
	return serveStdio(args)
}

// usage prints the command surface.
func usage() {
	fmt.Fprint(os.Stderr, `search — self-hosted, agent-first web search

Usage:
  search                 serve MCP over stdio (what an agent spawns)
  search stdio [flags]   the same, explicit
  search serve [flags]   serve the JSON API and MCP over HTTP
  search version         print the version

Serve flags:
  -config PATH   JSON config (default $SEARCH_CONFIG or ~/.config/search/search.json)
  -addr ADDR     listen address (default 127.0.0.1:8642)
  -data DIR      data directory for the index
`)
}

// core holds the wired dependencies shared by every transport.
type core struct {
	cfg    *config.Config
	store  *index.Store
	fetch  *fetch.Fetcher
	search *engine.Registry
	log    *slog.Logger
}

// buildCore loads configuration and opens the index and service layers. The
// same step backs stdio and HTTP, so both behave identically.
func buildCore(configPath string) (*core, error) {
	cfg, err := config.Load(configPath)
	if err != nil {
		return nil, err
	}
	logger := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: cfg.LogLevel()}))
	store, err := index.Open(cfg.DataDir)
	if err != nil {
		return nil, fmt.Errorf("open index: %w", err)
	}
	fetcher, err := fetch.New(cfg.Fetch, store, logger)
	if err != nil {
		store.Close()
		return nil, fmt.Errorf("fetcher: %w", err)
	}
	searcher, err := engine.New(cfg.Engines, cfg.Search, logger)
	if err != nil {
		store.Close()
		return nil, fmt.Errorf("engines: %w", err)
	}
	return &core{cfg: cfg, store: store, fetch: fetcher, search: searcher, log: logger}, nil
}

func (c *core) close() { c.store.Close() }

// serveStdio runs MCP over stdin/stdout for a locally spawned agent.
func serveStdio(args []string) int {
	fs := flag.NewFlagSet("stdio", flag.ContinueOnError)
	configPath := fs.String("config", "", "path to a JSON config file")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	// stdio must not write logs to stdout; they already go to stderr.
	c, err := buildCore(*configPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: %v\n", err)
		return 1
	}
	defer c.close()

	srv := mcp.New(mcp.Deps{Searcher: c.search, Fetcher: c.fetch, Index: c.store, Version: version})
	if err := srv.Run(context.Background(), &sdkmcp.StdioTransport{}); err != nil {
		fmt.Fprintf(os.Stderr, "search: stdio: %v\n", err)
		return 1
	}
	return 0
}

// serveHTTP runs the JSON API and the streamable MCP endpoint on one listener.
func serveHTTP(args []string) int {
	fs := flag.NewFlagSet("serve", flag.ContinueOnError)
	configPath := fs.String("config", "", "path to a JSON config file")
	addr := fs.String("addr", "", "listen address")
	dataDir := fs.String("data", "", "data directory")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	c, err := buildCore(*configPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "search: %v\n", err)
		return 1
	}
	defer c.close()
	if *addr != "" {
		c.cfg.Addr = *addr
	}
	if *dataDir != "" {
		c.cfg.DataDir = *dataDir
	}

	mcpServer := mcp.New(mcp.Deps{Searcher: c.search, Fetcher: c.fetch, Index: c.store, Version: version})
	mcpHandler := sdkmcp.NewStreamableHTTPHandler(func(*http.Request) *sdkmcp.Server { return mcpServer }, nil)

	jsonHandler := api.New(api.Options{
		Searcher: c.search,
		Fetcher:  c.fetch,
		Index:    c.store,
		Token:    c.cfg.Token,
		Version:  version,
		Logger:   c.log,
	})
	if c.cfg.Token == "" {
		fmt.Fprintln(os.Stderr, "search: no token configured; set the variable named by tokenEnv (default SEARCH_TOKEN)")
		return 1
	}

	root := http.NewServeMux()
	root.Handle("/mcp", jsonHandler.RequireAuth(mcpHandler))
	root.Handle("/", jsonHandler)

	server := &http.Server{
		Addr:              c.cfg.Addr,
		Handler:           root,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      120 * time.Second,
		IdleTimeout:       120 * time.Second,
		MaxHeaderBytes:    1 << 16,
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	errc := make(chan error, 1)
	go func() {
		c.log.Info("listening", "addr", c.cfg.Addr, "data", c.cfg.DataDir, "version", version)
		if err := server.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			errc <- err
		}
	}()

	select {
	case err := <-errc:
		c.log.Error("server stopped", "err", err)
		return 1
	case <-ctx.Done():
		c.log.Info("shutting down")
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdownCtx)
		return 0
	}
}
