//! The SQLite schema and the FTS query builder.
//!
//! Search input reaches SQLite as a MATCH expression, which has its own syntax.
//! `fts_query` is the only place raw user text becomes that expression: it keeps
//! only word characters and quotes every term, so punctuation cannot inject
//! operators. That single choke point is the reason the index is safe.

use rusqlite::Connection;

use super::StoreError;

/// Create the tables, the full-text index, and the triggers that keep them
/// consistent. Idempotent, so `Store::open` may run it every time.
pub(super) fn apply(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS pages (
             url        TEXT PRIMARY KEY,
             title      TEXT NOT NULL DEFAULT '',
             text       TEXT NOT NULL DEFAULT '',
             host       TEXT NOT NULL DEFAULT '',
             fetched_at INTEGER NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
             title, text, url UNINDEXED, host UNINDEXED,
             content='pages', content_rowid='rowid', tokenize='porter unicode61'
         );
         CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
             INSERT INTO pages_fts(rowid, title, text, url, host)
             VALUES (new.rowid, new.title, new.text, new.url, new.host);
         END;
         CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
             INSERT INTO pages_fts(pages_fts, rowid, title, text, url, host)
             VALUES ('delete', old.rowid, old.title, old.text, old.url, old.host);
         END;
         CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
             INSERT INTO pages_fts(pages_fts, rowid, title, text, url, host)
             VALUES ('delete', old.rowid, old.title, old.text, old.url, old.host);
             INSERT INTO pages_fts(rowid, title, text, url, host)
             VALUES (new.rowid, new.title, new.text, new.url, new.host);
         END;
         CREATE INDEX IF NOT EXISTS pages_host ON pages(host);
         CREATE INDEX IF NOT EXISTS pages_fetched ON pages(fetched_at);",
    )?;
    Ok(())
}

/// Turn free text into a safe FTS5 MATCH expression. Terms are reduced to word
/// characters and individually quoted, then joined with a space (an implicit
/// AND). Returns `None` when nothing usable remains — an empty query must not
/// become an empty MATCH, which SQLite rejects.
pub(super) fn fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !(c == '_' || c == '-' || c == '.' || c.is_alphanumeric()))
        .map(|term| term.trim_matches(|c| c == '-' || c == '.'))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Summary of the corpus for status output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Stats {
    pub documents: usize,
    pub hosts: usize,
    pub bytes: u64,
    /// Unix seconds of the oldest and newest documents, if any.
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every term is quoted, so FTS operators in user input stay literal
    /// search terms rather than becoming match syntax.
    #[test]
    fn fts_operators_cannot_inject() {
        assert_eq!(fts_query("a OR b").as_deref(), Some("\"a\" \"OR\" \"b\""));
        assert_eq!(fts_query("NEAR(x)").as_deref(), Some("\"NEAR\" \"x\""));
        assert_eq!(fts_query("\"quoted\"").as_deref(), Some("\"quoted\""));
        assert_eq!(fts_query("title:x*").as_deref(), Some("\"title\" \"x\""));
        // Every produced term is wrapped in quotes, so no operator can survive.
        let expression = fts_query("a OR b NEAR(x) title:y*").unwrap();
        assert!(expression
            .split(' ')
            .all(|term| term.starts_with('"') && term.ends_with('"')));
    }

    #[test]
    fn empty_input_yields_no_expression() {
        assert!(fts_query("").is_none());
        assert!(fts_query("   ").is_none());
        assert!(fts_query("!!!").is_none());
    }
}
