// Package fetch retrieves URLs on behalf of callers and turns HTML into readable
// text. Because URLs are model-chosen, every request is treated as hostile:
// destinations are resolved and refused if they point inside the network, each
// redirect hop is re-checked, bodies are size-capped, and deadlines are enforced.
// Every successfully fetched page is indexed, so the private corpus grows from
// real use rather than from crawling.
package fetch

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/fschrhunt/search/internal/config"
	"github.com/fschrhunt/search/internal/index"
)

// Fetcher owns the guarded HTTP client and the index.
type Fetcher struct {
	cfg    config.FetchConfig
	client *http.Client
	store  *index.Store
	log    *slog.Logger
	sem    chan struct{}
	cache  *ttlCache
}

// New builds the fetcher and its client. The client never reuses the default
// transport, so the connection pool and dialer are ours to constrain.
func New(cfg config.FetchConfig, store *index.Store, log *slog.Logger) (*Fetcher, error) {
	f := &Fetcher{
		cfg:   cfg,
		store: store,
		log:   log,
		sem:   make(chan struct{}, cfg.MaxConcurrency),
		cache: newTTLCache(cfg.CacheTTL),
	}
	dialer := &net.Dialer{Timeout: 8 * time.Second, KeepAlive: 30 * time.Second}
	transport := &http.Transport{
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			host, port, err := net.SplitHostPort(addr)
			if err != nil {
				return nil, err
			}
			ip, err := f.resolvePublic(ctx, host)
			if err != nil {
				return nil, err
			}
			// Dial the vetted IP directly, so a DNS answer that changed between
			// the check and the dial cannot slip a private address through.
			return dialer.DialContext(ctx, network, net.JoinHostPort(ip.String(), port))
		},
		MaxIdleConns:        20,
		MaxIdleConnsPerHost: 2,
		IdleConnTimeout:     30 * time.Second,
		TLSHandshakeTimeout: 8 * time.Second,
		DisableCompression:  true,
		TLSClientConfig:     &tls.Config{MinVersion: tls.VersionTLS12},
	}
	f.client = &http.Client{
		Transport: transport,
		Timeout:   cfg.Timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= cfg.MaxRedirects {
				return fmt.Errorf("stopped after %d redirects", cfg.MaxRedirects)
			}
			if err := f.checkHost(req.URL.Hostname()); err != nil {
				return err
			}
			return nil
		},
	}
	return f, nil
}

// Result is the outcome of one fetch.
type Result struct {
	URL         string `json:"url"`
	FinalURL    string `json:"finalUrl,omitempty"`
	Status      int    `json:"status"`
	ContentType string `json:"contentType"`
	Title       string `json:"title,omitempty"`
	Text        string `json:"text"`
	Truncated   bool   `json:"truncated,omitempty"`
	Indexed     bool   `json:"indexed,omitempty"`
	Cached      bool   `json:"cached,omitempty"`
	NotModified bool   `json:"notModified,omitempty"`
	Error       string `json:"error,omitempty"`
}

// Fetch retrieves one URL. A per-URL failure is reported in the Result, never as
// a panic, so batch callers can include partial success.
func (f *Fetcher) Fetch(ctx context.Context, rawURL string) Result {
	res := Result{URL: rawURL}
	parsed, err := url.Parse(rawURL)
	if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		res.Error = "only http and https URLs are supported"
		return res
	}
	if err := f.checkHost(parsed.Hostname()); err != nil {
		res.Error = err.Error()
		return res
	}

	if cached, ok := f.cache.Get(rawURL); ok {
		cached.Cached = true
		return cached
	}

	select {
	case f.sem <- struct{}{}:
		defer func() { <-f.sem }()
	case <-ctx.Done():
		res.Error = ctx.Err().Error()
		return res
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		res.Error = err.Error()
		return res
	}
	req.Header.Set("User-Agent", "search/1.0 (+https://github.com/fschrhunt/search)")
	req.Header.Set("Accept", "text/html,application/xhtml+xml,application/json;q=0.9,text/plain;q=0.8,*/*;q=0.5")
	req.Header.Set("Accept-Language", "en-US,en;q=0.9")

	resp, err := f.client.Do(req)
	if err != nil {
		res.Error = classify(err).Error()
		return res
	}
	defer resp.Body.Close()

	res.Status = resp.StatusCode
	res.FinalURL = resp.Request.URL.String()
	res.ContentType = resp.Header.Get("Content-Type")
	if resp.StatusCode >= 400 {
		res.Error = fmt.Sprintf("HTTP %d", resp.StatusCode)
		return res
	}

	res.FinalURL = resp.Request.URL.String()
	res.ContentType = resp.Header.Get("Content-Type")
	if resp.StatusCode >= 400 {
		res.Error = fmt.Sprintf("HTTP %d", resp.StatusCode)
		return res
	}

	body, truncated, err := readCapped(resp.Body, f.cfg.MaxBytes)
	if err != nil {
		res.Error = err.Error()
		return res
	}
	res.Truncated = truncated

	if isHTML(res.ContentType) {
		res.Title, res.Text = extract(resp.Request.URL, body)
	} else {
		res.Text = sanitizeText(body)
	}
	if res.Text == "" {
		res.Error = "no readable text in response"
		return res
	}

	if f.cfg.ShouldIndex() {
		doc := index.Doc{
			URL:       res.FinalURL,
			Title:     res.Title,
			Text:      res.Text,
			Host:      resp.Request.URL.Host,
			FetchedAt: time.Now().UTC(),
		}
		if err := f.store.Put(doc); err != nil {
			f.log.Warn("index write failed", "url", res.FinalURL, "err", err)
		} else {
			res.Indexed = true
		}
	}
	f.cache.Put(rawURL, res)
	return res
}

// FetchMany fetches URLs concurrently, preserving input order.
func (f *Fetcher) FetchMany(ctx context.Context, urls []string) []Result {
	out := make([]Result, len(urls))
	done := make(chan int, len(urls))
	for i, u := range urls {
		go func(i int, u string) {
			out[i] = f.Fetch(ctx, u)
			done <- i
		}(i, u)
	}
	for range urls {
		select {
		case <-done:
		case <-ctx.Done():
			for i := range out {
				if out[i].URL == "" {
					out[i] = Result{URL: urls[i], Error: ctx.Err().Error()}
				}
			}
			return out
		}
	}
	return out
}

// checkHost refuses hostnames that are local by name.
func (f *Fetcher) checkHost(host string) error {
	if f.cfg.AllowPrivate {
		return nil
	}
	h := strings.ToLower(strings.TrimSuffix(host, "."))
	if h == "" {
		return errors.New("empty host")
	}
	if h == "localhost" || h == "metadata.google.internal" {
		return fmt.Errorf("refusing local host %q", host)
	}
	for _, suffix := range []string{".localhost", ".local", ".internal", ".home.arpa"} {
		if strings.HasSuffix(h, suffix) {
			return fmt.Errorf("refusing local host %q", host)
		}
	}
	if ip := net.ParseIP(h); ip != nil {
		if isPrivate(ip) {
			return fmt.Errorf("refusing private address %s", ip)
		}
	}
	return nil
}

// resolvePublic resolves a host to one public IP, refusing any host whose
// records include a private address. All answers are checked, so a split
// answer cannot hide an internal address behind a public one.
func (f *Fetcher) resolvePublic(ctx context.Context, host string) (net.IP, error) {
	if ip := net.ParseIP(host); ip != nil {
		if f.cfg.AllowPrivate || !isPrivate(ip) {
			return ip, nil
		}
		return nil, fmt.Errorf("refusing private address %s", ip)
	}
	ips, err := net.DefaultResolver.LookupIP(ctx, "ip", host)
	if err != nil {
		return nil, fmt.Errorf("resolve %s: %w", host, err)
	}
	if len(ips) == 0 {
		return nil, fmt.Errorf("resolve %s: no addresses", host)
	}
	for _, ip := range ips {
		if isPrivate(ip) && !f.cfg.AllowPrivate {
			return nil, fmt.Errorf("refusing %s: resolves to private %s", host, ip)
		}
	}
	return ips[0], nil
}

// isPrivate reports whether an address is not routable on the public internet:
// loopback, RFC1918, link-local (including cloud metadata), CGNAT, multicast,
// unspecified, documentation, and every IPv6 form that embeds an IPv4 address
// (NAT64, 6to4, IPv4-compatible), so a v6 literal cannot tunnel to a blocked v4.
func isPrivate(ip net.IP) bool {
	if ip.IsLoopback() || ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() ||
		ip.IsMulticast() || ip.IsUnspecified() || ip.IsPrivate() {
		return true
	}
	if v4 := ip.To4(); v4 != nil {
		return isPrivateV4(v4)
	}
	// IPv6: reject transition and documentation ranges that embed or stand in
	// for IPv4, and any address whose low 32 bits decode to a blocked IPv4.
	if len(ip) == net.IPv6len {
		switch {
		case ip[0] == 0x00 && ip[1] == 0x64 && ip[2] == 0xff && ip[3] == 0x9b: // 64:ff9b::/96 NAT64
			return isPrivateV4(net.IPv4(ip[12], ip[13], ip[14], ip[15]))
		case ip[0] == 0x20 && ip[1] == 0x02: // 2002::/16 6to4
			return isPrivateV4(net.IPv4(ip[2], ip[3], ip[4], ip[5]))
		case ip[0] == 0x20 && ip[1] == 0x01 && ip[2] == 0x00 && ip[3] == 0x00: // 2001::/32 Teredo
			return true
		case ip[0] == 0x20 && ip[1] == 0x01 && ip[2] == 0x0d && ip[3] == 0xb8: // 2001:db8::/32 documentation
			return true
		}
		// IPv4-compatible ::/96 (the first 96 bits are zero) embeds the v4 in the last 32.
		allZero := true
		for i := 0; i < 12; i++ {
			if ip[i] != 0 {
				allZero = false
				break
			}
		}
		if allZero && (ip[12] != 0 || ip[13] != 0 || ip[14] != 0 || ip[15] != 0) {
			return isPrivateV4(net.IPv4(ip[12], ip[13], ip[14], ip[15]))
		}
	}
	return false
}

// isPrivateV4 classifies a 4-byte address.
func isPrivateV4(v4 net.IP) bool {
	if v4.IsLoopback() || v4.IsLinkLocalUnicast() || v4.IsLinkLocalMulticast() ||
		v4.IsMulticast() || v4.IsUnspecified() || v4.IsPrivate() {
		return true
	}
	switch {
	case v4[0] == 100 && v4[1] >= 64 && v4[1] <= 127: // 100.64/10 CGNAT
		return true
	case v4[0] == 192 && v4[1] == 0 && v4[2] == 0: // 192.0.0/24
		return true
	case v4[0] == 192 && v4[1] == 0 && v4[2] == 2: // TEST-NET-1
		return true
	case v4[0] == 198 && (v4[1] == 18 || v4[1] == 19): // benchmarking
		return true
	case v4[0] == 198 && v4[1] == 51 && v4[2] == 100: // TEST-NET-2
		return true
	case v4[0] == 203 && v4[1] == 0 && v4[2] == 113: // TEST-NET-3
		return true
	case v4[0] >= 240: // reserved
		return true
	}
	return false
}

// classify turns network errors into concise, non-leaking messages.
func classify(err error) error {
	var ne net.Error
	if errors.As(err, &ne) && ne.Timeout() {
		return errors.New("request timed out")
	}
	msg := err.Error()
	switch {
	case strings.Contains(msg, "refusing"), strings.Contains(msg, "stopped after"):
		return err
	case strings.Contains(msg, "no such host"), strings.Contains(msg, "resolve"):
		return errors.New("host could not be resolved")
	default:
		return errors.New("request failed")
	}
}

// readCapped reads at most max bytes and reports whether more remained.
func readCapped(r io.Reader, max int64) ([]byte, bool, error) {
	buf := make([]byte, 0, 64<<10)
	tmp := make([]byte, 64<<10)
	var total int64
	for {
		n, err := r.Read(tmp)
		if n > 0 {
			remaining := max - total
			if int64(n) > remaining {
				buf = append(buf, tmp[:remaining]...)
				total = max
				return buf, true, nil
			}
			buf = append(buf, tmp[:n]...)
			total += int64(n)
		}
		if err == io.EOF {
			return buf, false, nil
		}
		if err != nil {
			return buf, total >= max, err
		}
	}
}

// sanitizeText makes any byte stream safe to store and return as UTF-8 text,
// dropping control characters that could confuse a terminal or a model.
func sanitizeText(b []byte) string {
	if !utf8.Valid(b) {
		b = []byte(strings.ToValidUTF8(string(b), ""))
	}
	var sb strings.Builder
	sb.Grow(len(b))
	for _, r := range string(b) {
		switch {
		case r == '\n' || r == '\t':
			sb.WriteRune(r)
		case r < 0x20 || r == 0x7f:
			// drop other control characters
		default:
			sb.WriteRune(r)
		}
	}
	return sb.String()
}

// isHTML reports whether a content type denotes an HTML document.
func isHTML(contentType string) bool {
	ct := strings.ToLower(contentType)
	return strings.Contains(ct, "html") || strings.Contains(ct, "xhtml")
}

// jsonNumber is a compile-time reference so config stays linked in tests.
var _ = json.Number("")
