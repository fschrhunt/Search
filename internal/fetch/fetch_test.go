package fetch

import (
	"context"
	"net"
	"strings"
	"testing"
	"time"

	"github.com/fschrhunt/search/internal/config"
	"github.com/fschrhunt/search/internal/index"
)

// TestPrivateAddressesRefused pins the SSRF guard: every class of address that
// could reach infrastructure must be classified private.
func TestPrivateAddressesRefused(t *testing.T) {
	private := []string{
		"127.0.0.1", "10.0.0.1", "172.16.5.4", "192.168.1.1",
		"169.254.169.254", // cloud metadata
		"100.64.0.1",      // CGNAT
		"0.0.0.0", "::1", "fc00::1", "fe80::1",
	}
	for _, s := range private {
		if !isPrivate(net.ParseIP(s)) {
			t.Errorf("%s should be private", s)
		}
	}
	public := []string{"1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:4700:4700::1111"}
	for _, s := range public {
		if isPrivate(net.ParseIP(s)) {
			t.Errorf("%s should be public", s)
		}
	}
}

// TestCheckHostRefusesLocalNames pins that loopback names and metadata hosts are
// rejected before any DNS lookup happens.
func TestCheckHostRefusesLocalNames(t *testing.T) {
	f := &Fetcher{cfg: config.FetchConfig{}}
	for _, host := range []string{"localhost", "foo.local", "x.internal", "metadata.google.internal", "127.0.0.1", "10.0.0.1"} {
		if err := f.checkHost(host); err == nil {
			t.Errorf("checkHost(%q) should refuse", host)
		}
	}
	if err := f.checkHost("example.com"); err != nil {
		t.Errorf("checkHost(example.com) refused: %v", err)
	}
}

// TestFetchRefusesPrivateBeforeDialing pins that a fetch of a private URL fails
// with a refusal and never touches the network.
func TestFetchRefusesPrivateBeforeDialing(t *testing.T) {
	store := openTempStore(t)
	f, err := New(config.FetchConfig{MaxBytes: 1024, MaxRedirects: 3, MaxConcurrency: 2}, store, discardLogger())
	if err != nil {
		t.Fatal(err)
	}
	res := f.Fetch(context.Background(), "http://169.254.169.254/computeMetadata/v1/")
	if res.Error == "" || !strings.Contains(res.Error, "private") {
		t.Fatalf("expected a private-address refusal, got %+v", res)
	}
	res = f.Fetch(context.Background(), "file:///etc/passwd")
	if res.Error == "" || !strings.Contains(res.Error, "http") {
		t.Fatalf("expected a scheme refusal, got %+v", res)
	}
}

// TestExtractReadsTitleAndText pins the HTML-to-text path.
func TestExtractReadsTitleAndText(t *testing.T) {
	doc := []byte(`<html><head><title>Doc Title</title><style>x{}</style></head>
<body><nav>skip</nav><script>evil()</script><main><h1>Heading</h1>
<p>First paragraph.</p><p>Second paragraph.</p></main></body></html>`)
	title, text := extract(nil, doc)
	if title != "Doc Title" {
		t.Errorf("title = %q", title)
	}
	if strings.Contains(text, "evil") || strings.Contains(text, "skip") {
		t.Errorf("text should exclude script and nav: %q", text)
	}
	if !strings.Contains(text, "First paragraph") || !strings.Contains(text, "Second paragraph") {
		t.Errorf("text missing content: %q", text)
	}
}

// TestReadCappedBoundsBody pins that a huge body is truncated, not buffered whole.
func TestReadCappedBoundsBody(t *testing.T) {
	big := strings.NewReader(strings.Repeat("x", 10_000))
	body, truncated, err := readCapped(big, 1024)
	if err != nil {
		t.Fatal(err)
	}
	if len(body) != 1024 || !truncated {
		t.Fatalf("expected 1024 bytes truncated, got %d truncated=%v", len(body), truncated)
	}
}

// TestIndexRoundTrip pins that a fetched page is stored and findable offline.
func TestIndexRoundTrip(t *testing.T) {
	store := openTempStore(t)
	if err := store.Put(index.Doc{URL: "https://e.example/p", Title: "Unique Title", Text: "a distinctive phrase about widgets", FetchedAt: time.Now()}); err != nil {
		t.Fatal(err)
	}
	hits, err := store.Search("distinctive widgets", 5)
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) != 1 || hits[0].URL != "https://e.example/p" {
		t.Fatalf("expected one hit, got %+v", hits)
	}
	// A query with FTS punctuation must not error.
	if _, err := store.Search("a: b* OR", 5); err != nil {
		t.Fatalf("punctuation query errored: %v", err)
	}
}

func openTempStore(t *testing.T) *index.Store {
	t.Helper()
	store, err := index.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	return store
}
