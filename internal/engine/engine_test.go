package engine

import (
	"os"
	"path/filepath"
	"testing"
)

// TestRankFusesEngines pins reciprocal-rank fusion: a URL two engines rank
// highly must beat a URL only one engine liked, and duplicates collapse to one.
func TestRankFusesEngines(t *testing.T) {
	results := []Result{
		{URL: "https://a.example/x", Engine: "brave", EngineRank: 1},
		{URL: "https://a.example/x", Engine: "wikipedia", EngineRank: 1},
		{URL: "https://b.example/y", Engine: "brave", EngineRank: 2},
		{URL: "https://c.example/z", Engine: "mwmbl", EngineRank: 1},
	}
	ranked := rank(results, 10)
	if len(ranked) != 3 {
		t.Fatalf("expected 3 unique results, got %d", len(ranked))
	}
	if ranked[0].URL != "https://a.example/x" {
		t.Fatalf("expected the two-engine URL first, got %s", ranked[0].URL)
	}
	// c and b each have one vote; c's rank-1 vote should edge b's rank-2.
	if ranked[1].URL != "https://c.example/z" {
		t.Fatalf("expected rank-1 single vote ahead of rank-2, got %s", ranked[1].URL)
	}
}

// TestCanonicalURLDedupesNoise pins that tracking parameters and fragments do
// not produce duplicate results.
func TestCanonicalURLDedupesNoise(t *testing.T) {
	a := canonicalURL("https://Example.com/Path/?utm_source=news&id=7#frag")
	b := canonicalURL("https://example.com/Path?id=7")
	if a != b {
		t.Fatalf("expected %q == %q", a, b)
	}
	if canonicalURL("javascript:alert(1)") != "" {
		t.Fatal("expected non-http URLs to be rejected")
	}
}

// TestParseBraveFixtures pins the Brave HTML scraper against a saved page.
func TestParseBraveFixtures(t *testing.T) {
	body := readFixture(t, "brave.html")
	got := parseBrave(body, 10)
	if len(got) == 0 {
		t.Fatal("expected results from the Brave fixture")
	}
	for _, r := range got {
		if r.Title == "" || r.URL == "" {
			t.Fatalf("result missing title or URL: %+v", r)
		}
	}
	if got[0].Snippet == "" {
		t.Error("expected the first Brave result to carry a snippet")
	}
}

// TestParseMarginaliaFixtures pins the Marginalia scraper.
func TestParseMarginaliaFixtures(t *testing.T) {
	got := parseMarginalia(readFixture(t, "marginalia.html"), 10)
	if len(got) == 0 {
		t.Fatal("expected results from the Marginalia fixture")
	}
	for _, r := range got {
		if r.URL == "" {
			t.Fatalf("result missing URL: %+v", r)
		}
	}
}

// TestHTMLEntitiesDecoded pins that scraped text is human-readable.
func TestHTMLEntitiesDecoded(t *testing.T) {
	if got := text("<b>a &amp; b</b>"); got != "a & b" {
		t.Fatalf("text = %q", got)
	}
}

func readFixture(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("testdata", name))
	if err != nil {
		t.Fatalf("read fixture %s: %v", name, err)
	}
	return data
}
