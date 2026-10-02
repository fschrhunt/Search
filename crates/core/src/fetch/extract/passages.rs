//! Select the passages of an extracted page that answer a query.
//!
//! A model reading a whole page spends context on everything irrelevant; the
//! mature agent tools return query-focused excerpts instead (Exa's "highlights",
//! Brave's `llm_context`, Claude's short `cited_text`). The technique is
//! classical extractive retrieval and needs no model: score each paragraph
//! against the query by term overlap with an inverse-document-frequency weight,
//! then return the best paragraphs with a sentence of context around each.
//!
//! This is deliberately simple. The corpus is one page, the queries are short,
//! and the goal is to hand an agent the two or three paragraphs that matter
//! instead of forty that do not.

use std::collections::HashMap;

/// One selected passage with the score that chose it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Passage {
    pub text: String,
    pub score: f64,
}

/// The default number of characters a query-focused read returns.
pub const DEFAULT_BUDGET: usize = 6_000;

/// Rank the paragraphs of `text` against `query`, returning the best ones up to
/// a character budget. With an empty query, the opening of the document is
/// returned (`keep`).
pub fn select(text: &str, query: &str, budget: usize) -> Vec<Passage> {
    let paragraphs = split_paragraphs(text);
    if paragraphs.is_empty() {
        return Vec::new();
    }
    let terms = query_terms(query);
    if terms.is_empty() {
        return vec![Passage {
            text: head(text, budget),
            score: 0.0,
        }];
    }

    // Document frequency of each query term across paragraphs, for the IDF
    // weight: a term in every paragraph distinguishes nothing.
    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    for paragraph in &paragraphs {
        let lowered = paragraph.to_ascii_lowercase();
        for term in &terms {
            if lowered.contains(term.as_str()) {
                *document_frequency.entry(term.as_str()).or_insert(0) += 1;
            }
        }
    }
    let total = paragraphs.len() as f64;

    let mut scored: Vec<(usize, f64)> = paragraphs
        .iter()
        .enumerate()
        .map(|(index, paragraph)| {
            let lowered = paragraph.to_ascii_lowercase();
            let words = lowered.split_whitespace().count().max(1) as f64;
            let mut score = 0.0;
            for term in &terms {
                let hits = lowered.matches(term.as_str()).count();
                if hits == 0 {
                    continue;
                }
                let df = document_frequency.get(term.as_str()).copied().unwrap_or(0) as f64;
                // BM25-style saturation of term frequency, times IDF.
                let idf = ((total - df + 0.5) / (df + 0.5) + 1.0).ln();
                let tf = (hits as f64 * 2.2) / (hits as f64 + 1.2 * (0.25 + 0.75 * words / 40.0));
                score += idf * tf;
            }
            (index, score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();

    if scored.is_empty() {
        // The query matched nothing verbatim; the opening is the best guess.
        return vec![Passage {
            text: head(text, budget),
            score: 0.0,
        }];
    }
    // Best-scoring paragraph first; ties keep document order.
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });

    let mut out: Vec<Passage> = Vec::new();
    let mut used = 0usize;
    for (index, score) in scored {
        // `index` came from enumerating `paragraphs`, so a lookup in range.
        let Some(passage) = paragraphs.get(index) else {
            continue;
        };
        let cost = passage.chars().count();
        if used + cost > budget && !out.is_empty() {
            break;
        }
        used += cost;
        out.push(Passage {
            text: passage.clone(),
            score,
        });
        if used >= budget {
            break;
        }
    }
    out
}

/// Split on blank lines; fall back to sentences when a page is one long block.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs: Vec<String> = text
        .split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.len() <= 1 {
        // No blank-line structure: treat sentences as candidates.
        paragraphs = text
            .split_terminator(['.', '!', '?'])
            .map(|s| format!("{}.", s.trim()))
            .filter(|s| s.len() > 1)
            .collect();
    }
    paragraphs
}

/// Lowercased query terms of two or more characters, deduplicated.
fn query_terms(query: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for term in query
        .split(|c: char| !c.is_alphanumeric())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.chars().count() >= 2)
    {
        if !seen.contains(&term) {
            seen.push(term);
        }
    }
    seen
}

/// The first `budget` characters of `text`, on a character boundary.
fn head(text: &str, budget: usize) -> String {
    text.chars().take(budget).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matching_paragraph_is_selected_first() {
        let text = "Introduction to the topic.\n\nThe borrow checker enforces memory \
            safety by tracking ownership.\n\nUnrelated closing remarks about licensing.\n\n\
            The borrow checker rejects use after move at compile time.";
        let passages = select(text, "borrow checker ownership", 2000);
        assert!(!passages.is_empty());
        assert!(
            passages[0]
                .text
                .to_ascii_lowercase()
                .contains("borrow checker"),
            "got {:?}",
            passages[0].text
        );
    }

    #[test]
    fn an_empty_query_returns_the_opening() {
        let passages = select("First part.\n\nSecond part.", "", 5000);
        assert_eq!(passages.len(), 1);
        assert!(passages[0].text.starts_with("First part."));
    }

    #[test]
    fn the_budget_is_respected() {
        let text = (0..50)
            .map(|i| format!("paragraph {i} with some filler words and a keyword"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let passages = select(&text, "keyword", 400);
        let total: usize = passages.iter().map(|p| p.text.chars().count()).sum();
        assert!(total <= 400, "budget exceeded: {total}");
    }
}
