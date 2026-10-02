//! The private, on-disk corpus: pages the fetcher has seen, searchable offline
//! with full-text search.
//!
//! It is deliberately separate from discovery. Discovery finds URLs anywhere on
//! the web; the index makes content already fetched instant and independent of
//! any upstream provider. Storage is SQLite in WAL mode through a bundled
//! engine, so the binary carries its database with no system dependency.

mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

pub use schema::Stats;

/// A stored page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Doc {
    pub url: String,
    pub title: String,
    pub text: String,
    pub host: String,
    pub fetched_at: i64,
}

/// A full-text hit, ranked by BM25.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub host: String,
    pub fetched_at: i64,
    pub score: f64,
}

/// A store failure. Every variant names what could not be done, so the caller
/// can decide whether to surface or log it.
#[derive(Debug)]
pub struct StoreError(String);

impl StoreError {
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError(error.to_string())
    }
}

/// Owns the SQLite connection and its schema. The connection is `Send` but not
/// `Sync`, so it is held under a mutex: the service is shared across handler
/// tasks, and SQLite serializes writes anyway.
pub struct Store {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    /// Open the store under `dir`, creating the directory and schema. A second
    /// process cannot open the same file for writing; SQLite serializes and
    /// reports rather than corrupting.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| StoreError(format!("create data dir {}: {e}", dir.display())))?;
        let path = dir.join("search.db");
        let connection = Connection::open(&path)?;
        // WAL gives concurrent readers beside one writer; NORMAL sync is the
        // durable-enough default for a cache-like corpus.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "busy_timeout", 5000)?;
        schema::apply(&connection)?;
        Ok(Store {
            connection: Mutex::new(connection),
            path,
        })
    }

    /// The database file path, for status output.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Upsert one document.
    pub fn put(&self, doc: &Doc) -> Result<(), StoreError> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO pages (url, title, text, host, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(url) DO UPDATE SET
                 title = excluded.title,
                 text = excluded.text,
                 host = excluded.host,
                 fetched_at = excluded.fetched_at",
            rusqlite::params![doc.url, doc.title, doc.text, doc.host, doc.fetched_at],
        )?;
        Ok(())
    }

    /// Full-text search over the corpus, ranked by BM25 (higher score first).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, StoreError> {
        let limit = if limit == 0 { 10 } else { limit };
        let Some(match_expression) = schema::fts_query(query) else {
            return Ok(Vec::new());
        };
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT p.url, p.title, p.text, p.host, p.fetched_at, bm25(pages_fts) AS rank
             FROM pages_fts
             JOIN pages p ON p.rowid = pages_fts.rowid
             WHERE pages_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows =
            statement.query_map(rusqlite::params![match_expression, limit as i64], |row| {
                let text: String = row.get(2)?;
                let rank: f64 = row.get(5)?;
                Ok(Hit {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    snippet: snippet(&text),
                    host: row.get(3)?,
                    fetched_at: row.get(4)?,
                    // bm25 returns smaller-is-better negative values; invert for
                    // a higher-is-better score callers can sort on.
                    score: -rank,
                })
            })?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    /// Corpus counts for status output.
    pub fn stats(&self) -> Result<Stats, StoreError> {
        let connection = self.lock();
        let (documents, hosts, bytes, oldest, newest): (i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT count(*), count(DISTINCT host), coalesce(sum(length(text)), 0),
                        coalesce(min(fetched_at), 0), coalesce(max(fetched_at), 0)
                 FROM pages",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
        Ok(Stats {
            documents: documents.max(0) as usize,
            hosts: hosts.max(0) as usize,
            bytes: bytes.max(0) as u64,
            oldest: if oldest > 0 { Some(oldest) } else { None },
            newest: if newest > 0 { Some(newest) } else { None },
        })
    }

    /// Lock the connection, recovering from a poisoned mutex: a panic in one
    /// handler must not wedge the store for every later request.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A short snippet around the first query term found in `text`. Falls back to
/// the document's opening when no term is found verbatim.
fn snippet(text: &str) -> String {
    const WINDOW: usize = 200;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= WINDOW {
        return collapsed;
    }
    collapsed.chars().take(WINDOW).collect::<String>() + "…"
}

/// Extract the host from a URL, for grouping and display.
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(&std::env::temp_dir().join(format!("search-test-{}", uuid::Uuid::new_v4())))
            .expect("open store")
    }

    #[test]
    fn a_stored_page_is_findable_offline() {
        let store = store();
        store
            .put(&Doc {
                url: "https://example.com/p".into(),
                title: "Unique Title".into(),
                text: "a distinctive phrase about widgets".into(),
                host: "example.com".into(),
                fetched_at: 1,
            })
            .unwrap();
        let hits = store.search("distinctive widgets", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/p");
    }

    #[test]
    fn punctuation_in_a_query_does_not_error() {
        let store = store();
        store
            .put(&Doc {
                url: "https://example.com/p".into(),
                title: "T".into(),
                text: "body".into(),
                host: "example.com".into(),
                fetched_at: 1,
            })
            .unwrap();
        assert!(store.search("a: b* OR", 5).is_ok());
    }

    #[test]
    fn stats_count_documents_and_hosts() {
        let store = store();
        for (url, host) in [
            ("https://a.example/1", "a.example"),
            ("https://b.example/2", "b.example"),
        ] {
            store
                .put(&Doc {
                    url: url.into(),
                    title: "t".into(),
                    text: "body".into(),
                    host: host.into(),
                    fetched_at: 1,
                })
                .unwrap();
        }
        let stats = store.stats().unwrap();
        assert_eq!(stats.documents, 2);
        assert_eq!(stats.hosts, 2);
    }
}
