// Package index is the private, on-disk corpus: pages the fetcher has seen,
// searchable offline with full-text search. It is deliberately separate from
// discovery. Discovery finds URLs anywhere on the web; the index makes content
// we have already fetched instant and independent of any upstream engine.
//
// Storage is SQLite in WAL mode through the pure-Go modernc driver, so the
// binary stays static and cgo-free. The store is opened exclusively: a second
// process against the same data directory fails rather than corrupting it.
package index

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "modernc.org/sqlite"
)

// Doc is one indexed page.
type Doc struct {
	URL       string    `json:"url"`
	Title     string    `json:"title"`
	Text      string    `json:"text"`
	Host      string    `json:"host"`
	FetchedAt time.Time `json:"fetchedAt"`
}

// Hit is a search result from the index, with a rank score for fusion.
type Hit struct {
	Doc
	Rank float64 `json:"rank"`
}

// Store owns the SQLite handle and its schema.
type Store struct {
	db   *sql.DB
	path string
}

// Open creates the data directory and opens the database, applying the schema.
func Open(dir string) (*Store, error) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("create data dir: %w", err)
	}
	path := filepath.Join(dir, "search.db")
	db, err := sql.Open("sqlite", path+"?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)")
	if err != nil {
		return nil, err
	}
	// A single writer plus WAL readers; SQLite serializes writes anyway, and a
	// small pool avoids lock churn.
	db.SetMaxOpenConns(4)
	s := &Store{db: db, path: path}
	if err := s.migrate(); err != nil {
		db.Close()
		return nil, err
	}
	return s, nil
}

// Close releases the database handle.
func (s *Store) Close() error { return s.db.Close() }

// Path returns the database file path, for status output.
func (s *Store) Path() string { return s.path }

const schema = `
CREATE TABLE IF NOT EXISTS pages (
    url        TEXT PRIMARY KEY,
    title      TEXT NOT NULL DEFAULT '',
    text       TEXT NOT NULL DEFAULT '',
    host       TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL,
    etag       TEXT NOT NULL DEFAULT '',
    meta       TEXT NOT NULL DEFAULT ''
);
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
    title, text, url UNINDEXED, host UNINDEXED,
    content='pages', content_rowid='rowid', tokenize='porter unicode61'
);
CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
    INSERT INTO pages_fts(rowid, title, text, url, host) VALUES (new.rowid, new.title, new.text, new.url, new.host);
END;
CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, title, text, url, host) VALUES ('delete', old.rowid, old.title, old.text, old.url, old.host);
END;
CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, title, text, url, host) VALUES ('delete', old.rowid, old.title, old.text, old.url, old.host);
    INSERT INTO pages_fts(rowid, title, text, url, host) VALUES (new.rowid, new.title, new.text, new.url, new.host);
END;
CREATE INDEX IF NOT EXISTS pages_host ON pages(host);
CREATE INDEX IF NOT EXISTS pages_fetched ON pages(fetched_at);
`

func (s *Store) migrate() error {
	_, err := s.db.Exec(schema)
	return err
}

// Put upserts one document. Empty or non-text content is dropped by callers
// before reaching here.
func (s *Store) Put(doc Doc) error {
	host := doc.Host
	if host == "" {
		host = hostOf(doc.URL)
	}
	_, err := s.db.Exec(`
        INSERT INTO pages (url, title, text, host, fetched_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(url) DO UPDATE SET
            title=excluded.title, text=excluded.text, host=excluded.host, fetched_at=excluded.fetched_at`,
		doc.URL, doc.Title, doc.Text, host, doc.FetchedAt.Unix())
	return err
}

// Get returns a stored document, or false if it is not indexed.
func (s *Store) Get(url string) (Doc, bool, error) {
	row := s.db.QueryRow(`SELECT url, title, text, host, fetched_at FROM pages WHERE url = ?`, url)
	var d Doc
	var ts int64
	if err := row.Scan(&d.URL, &d.Title, &d.Text, &d.Host, &ts); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return Doc{}, false, nil
		}
		return Doc{}, false, err
	}
	d.FetchedAt = time.Unix(ts, 0)
	return d, true, nil
}

// Search runs a full-text query and returns up to limit hits ranked by BM25
// (lower is better, so negate for a higher-is-better score). The query is
// sanitized to a safe MATCH expression.
func (s *Store) Search(query string, limit int) ([]Hit, error) {
	if limit <= 0 {
		limit = 10
	}
	match := ftsQuery(query)
	if match == "" {
		return nil, nil
	}
	rows, err := s.db.Query(`
        SELECT p.url, p.title, p.text, p.host, p.fetched_at, bm25(pages_fts) AS rank
        FROM pages_fts
        JOIN pages p ON p.rowid = pages_fts.rowid
        WHERE pages_fts MATCH ?
        ORDER BY rank
        LIMIT ?`, match, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []Hit
	for rows.Next() {
		var h Hit
		var ts int64
		if err := rows.Scan(&h.URL, &h.Title, &h.Text, &h.Host, &ts, &h.Rank); err != nil {
			return nil, err
		}
		h.FetchedAt = time.Unix(ts, 0)
		h.Rank = -h.Rank // bm25 returns negative values already; normalize sign
		out = append(out, h)
	}
	return out, rows.Err()
}

// Stats summarizes the corpus for status output.
type Stats struct {
	Documents int    `json:"documents"`
	Hosts     int    `json:"hosts"`
	Bytes     int64  `json:"bytes"`
	Oldest    string `json:"oldest,omitempty"`
	Newest    string `json:"newest,omitempty"`
}

// Stats reads corpus counts.
func (s *Store) Stats() (Stats, error) {
	var st Stats
	row := s.db.QueryRow(`SELECT count(*), count(DISTINCT host), coalesce(sum(length(text)),0), coalesce(min(fetched_at),0), coalesce(max(fetched_at),0) FROM pages`)
	var oldest, newest int64
	if err := row.Scan(&st.Documents, &st.Hosts, &st.Bytes, &oldest, &newest); err != nil {
		return st, err
	}
	if oldest > 0 {
		st.Oldest = time.Unix(oldest, 0).UTC().Format(time.RFC3339)
	}
	if newest > 0 {
		st.Newest = time.Unix(newest, 0).UTC().Format(time.RFC3339)
	}
	return st, nil
}

// ftsQuery turns free text into a safe FTS5 MATCH expression: terms are quoted
// (so punctuation cannot inject syntax) and ANDed, with a prefix wildcard on the
// final term for as-you-type behavior.
func ftsQuery(query string) string {
	fields := strings.FieldsFunc(query, func(r rune) bool {
		return !(r == '_' || r == '-' || r == '.' ||
			(r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9'))
	})
	var terms []string
	for _, f := range fields {
		f = strings.Trim(f, "-.")
		if f == "" {
			continue
		}
		terms = append(terms, `"`+strings.ReplaceAll(f, `"`, `""`)+`"`)
	}
	return strings.Join(terms, " ")
}

// hostOf extracts the host from a URL without importing net/url twice.
func hostOf(raw string) string {
	if i := strings.Index(raw, "://"); i >= 0 {
		rest := raw[i+3:]
		if j := strings.IndexAny(rest, "/?#"); j >= 0 {
			rest = rest[:j]
		}
		return rest
	}
	return ""
}

// MarshalJSON keeps Doc JSON stable for tests.
var _ = json.Marshal
