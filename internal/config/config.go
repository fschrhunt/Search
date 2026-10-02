// Package config loads and validates the search service's settings. Every setting
// has a safe default; a config file only overrides what it names. The token is the
// one secret, and it may come from the file or, preferably, the named environment
// variable, so a config file can be shared without leaking the credential.
package config

import (
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// DefaultAddr binds loopback by default. Exposing search beyond loopback is a
// deliberate act: the operator sets Addr and the token together.
const DefaultAddr = "127.0.0.1:8642"

// Config is the whole of the service's settings.
type Config struct {
	Addr      string       `json:"addr"`
	DataDir   string       `json:"dataDir"`
	Token     string       `json:"token"`    // literal token; prefer TokenEnv
	TokenEnv  string       `json:"tokenEnv"` // environment variable holding the token
	Log       string       `json:"log"`      // debug, info, warn, error
	Search    SearchConfig `json:"search"`
	Fetch     FetchConfig  `json:"fetch"`
	Engines   EngineConfig `json:"engines"`
	UserAgent string       `json:"userAgent"`
}

// SearchConfig bounds query work. MaxEngineTime caps a single engine's latency so
// a slow provider cannot hold up the whole fan-out.
type SearchConfig struct {
	MaxResults     int           `json:"maxResults"`
	MaxEngineTime  time.Duration `json:"maxEngineTime"`
	OverallTimeout time.Duration `json:"overallTimeout"`
	CacheTTL       time.Duration `json:"cacheTTL"`
}

// FetchConfig bounds follower behavior. Private networks are refused by default;
// AllowPrivate exists only for tests and air-gapped mirrors.
type FetchConfig struct {
	Timeout      time.Duration `json:"timeout"`
	MaxBytes     int64         `json:"maxBytes"`
	MaxRedirects int           `json:"maxRedirects"`
	CacheTTL     time.Duration `json:"cacheTTL"`
	AllowPrivate bool          `json:"allowPrivate"`
	// IndexFetched defaults true; set false to stop adding fetched pages to the
	// private corpus.
	IndexFetched   *bool `json:"indexFetched"`
	MaxConcurrency int   `json:"maxConcurrency"`
}

// ShouldIndex reports the effective indexing setting.
func (f FetchConfig) ShouldIndex() bool {
	return f.IndexFetched == nil || *f.IndexFetched
}

// EngineConfig selects which discovery providers run and how to reach ones that
// need a key. A nil/omitted slice means "the built-in defaults".
type EngineConfig struct {
	Enabled []string          `json:"enabled"`
	Keys    map[string]string `json:"keys"`    // engine name -> literal key
	KeyEnvs map[string]string `json:"keyEnvs"` // engine name -> env var holding the key
}

// Load reads configuration from path (or the default location) and applies
// defaults, environment overrides, and validation.
func Load(path string) (*Config, error) {
	if path == "" {
		path = os.Getenv("SEARCH_CONFIG")
	}
	if path == "" {
		home, err := os.UserHomeDir()
		if err == nil {
			path = filepath.Join(home, ".config", "search", "search.json")
		}
	}

	cfg := &Config{}
	cfg.applyDefaults()
	if path != "" {
		if data, err := os.ReadFile(path); err == nil {
			if err := json.Unmarshal(data, cfg); err != nil {
				return nil, fmt.Errorf("parse %s: %w", path, err)
			}
			cfg.applyDefaults()
		} else if !errors.Is(err, os.ErrNotExist) {
			return nil, fmt.Errorf("read %s: %w", path, err)
		}
	}

	if env := os.Getenv("SEARCH_ADDR"); env != "" {
		cfg.Addr = env
	}
	if env := os.Getenv("SEARCH_DATA_DIR"); env != "" {
		cfg.DataDir = env
	}
	cfg.resolveToken()
	cfg.resolveEngineKeys()

	if err := cfg.validate(); err != nil {
		return nil, err
	}
	return cfg, nil
}

// applyDefaults fills unset fields, and is safe to call after any unmarshal.
func (c *Config) applyDefaults() {
	if c.Addr == "" {
		c.Addr = DefaultAddr
	}
	if c.DataDir == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			home = "."
		}
		c.DataDir = filepath.Join(home, ".local", "share", "search")
	}
	if c.TokenEnv == "" {
		c.TokenEnv = "SEARCH_TOKEN"
	}
	if c.Search.MaxResults <= 0 {
		c.Search.MaxResults = 10
	}
	if c.Search.MaxEngineTime <= 0 {
		c.Search.MaxEngineTime = 2500 * time.Millisecond
	}
	if c.Search.OverallTimeout <= 0 {
		c.Search.OverallTimeout = 8 * time.Second
	}
	if c.Search.CacheTTL <= 0 {
		c.Search.CacheTTL = 5 * time.Minute
	}
	if c.Fetch.Timeout <= 0 {
		c.Fetch.Timeout = 15 * time.Second
	}
	if c.Fetch.MaxBytes <= 0 {
		c.Fetch.MaxBytes = 4 << 20
	}
	if c.Fetch.MaxRedirects <= 0 {
		c.Fetch.MaxRedirects = 5
	}
	if c.Fetch.CacheTTL <= 0 {
		c.Fetch.CacheTTL = 10 * time.Minute
	}
	if c.Fetch.MaxConcurrency <= 0 {
		c.Fetch.MaxConcurrency = 8
	}
	if c.UserAgent == "" {
		c.UserAgent = "search/1.0 (+https://github.com/fschrhunt/search)"
	}
}

// resolveToken prefers the environment, then falls back to a literal in the file.
func (c *Config) resolveToken() {
	if c.TokenEnv != "" {
		if v := os.Getenv(c.TokenEnv); v != "" {
			c.Token = v
			return
		}
	}
}

// resolveEngineKeys fills each engine's key from KeyEnvs first, then Keys.
func (c *Config) resolveEngineKeys() {
	for name, env := range c.EngineCfg().KeyEnvs {
		if v := os.Getenv(env); v != "" {
			c.EngineCfg().Keys[name] = v
		}
	}
}

// EngineCfg returns the engine configuration, initializing maps as needed.
// It exists so EngineKeys can be mutated during resolution without nil panics.
func (c *Config) EngineCfg() *EngineConfig {
	if c.Engines.Keys == nil {
		c.Engines.Keys = map[string]string{}
	}
	if c.Engines.KeyEnvs == nil {
		c.Engines.KeyEnvs = map[string]string{}
	}
	return &c.Engines
}

// Validate checks the effective settings. It is exported so callers that change
// the address after Load can re-check before binding.
func (c *Config) Validate() error { return c.validate() }

// validate rejects settings that would be unsafe or nonsensical.
func (c *Config) validate() error {
	host, _, err := net.SplitHostPort(c.Addr)
	if err != nil {
		return fmt.Errorf("addr %q is not host:port", c.Addr)
	}
	loopback := false
	switch host {
	case "", "localhost":
		// The empty host means all interfaces (":8642"), which is NOT loopback.
		loopback = host == "localhost"
	default:
		if ip := net.ParseIP(host); ip != nil {
			loopback = ip.IsLoopback()
		}
	}
	if !loopback && c.Token == "" {
		return fmt.Errorf("addr %q is not loopback; a token is required", c.Addr)
	}
	if c.Token != "" && len(c.Token) < 16 {
		return fmt.Errorf("token must be at least 16 characters")
	}
	return nil
}

// LogLevel maps the configured level name to a slog level.
func (c *Config) LogLevel() slog.Level {
	switch strings.ToLower(c.Log) {
	case "debug":
		return slog.LevelDebug
	case "warn":
		return slog.LevelWarn
	case "error":
		return slog.LevelError
	default:
		return slog.LevelInfo
	}
}
