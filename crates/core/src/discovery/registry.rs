//! The fan-out: run every selected provider concurrently under its own
//! deadline, then merge the answers into one ranking.
//!
//! Failure is interpretive, not fatal: a provider that errors is reported in its
//! `ProviderState` beside the results, so an empty result set is never mistaken
//! for a broken one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;

use super::{
    default_providers, FailureCause, Finding, Provider, ProviderError, ProviderState,
    ProviderStatus, Query, Response,
};
use crate::config::{EngineSettings, SearchSettings};

/// Holds the enabled providers and the query bounds.
pub struct Registry {
    providers: Vec<Arc<dyn Provider>>,
    settings: SearchSettings,
}

impl Registry {
    /// Build the registry from configuration. A provider whose required key is
    /// missing is dropped rather than failing startup.
    pub fn new(engines: &EngineSettings, search: SearchSettings) -> Self {
        let providers: Vec<Arc<dyn Provider>> = default_providers(engines)
            .into_iter()
            .filter(|p| !p.missing_key())
            .map(Arc::from)
            .collect();
        Registry {
            providers,
            settings: search,
        }
    }

    /// The enabled provider names, for status output.
    pub fn names(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// The server-side ceiling for one query.
    pub fn overall_timeout(&self) -> Duration {
        self.settings.overall_timeout()
    }

    /// Run the query across every selected provider and merge the answers.
    pub async fn search(&self, query: Query) -> Response {
        let started = Instant::now();
        let limit = if query.limit == 0 {
            self.settings.max_results_or_default()
        } else {
            query.limit
        };
        let per_provider = if query.per_provider == 0 {
            limit
        } else {
            query.per_provider
        };

        let selected: Vec<Arc<dyn Provider>> = self.select(&query.providers);
        let per_provider_time = self.settings.max_provider_time();

        let mut set: JoinSet<(usize, ProviderState, Vec<Finding>)> = JoinSet::new();
        for (index, provider) in selected.iter().enumerate() {
            let provider = Arc::clone(provider);
            let text = query.text.clone();
            set.spawn(async move {
                run_provider(index, provider, text, per_provider, per_provider_time).await
            });
        }

        // Ordered by provider index so output is stable regardless of finish order.
        let mut collected: Vec<Option<(ProviderState, Vec<Finding>)>> =
            (0..selected.len()).map(|_| None).collect();
        let mut panicked = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                // The index is produced by `enumerate` over `selected`, so it is
                // always in range; `get_mut` keeps that provable rather than
                // asserted.
                Ok((index, state, results)) => {
                    if let Some(slot) = collected.get_mut(index) {
                        *slot = Some((state, results));
                    }
                }
                // A panicking provider must not take the query down; it is
                // reported as a failed provider beside the results.
                Err(join) => panicked.push(ProviderState {
                    name: "provider".into(),
                    status: ProviderStatus::Error,
                    count: 0,
                    error: Some(format!("provider task failed: {join}")),
                    elapsed_ms: 0,
                }),
            }
        }

        let mut states = Vec::with_capacity(collected.len());
        let mut merged = Vec::new();
        for slot in collected.into_iter().flatten() {
            let (state, results) = slot;
            states.push(state);
            merged.extend(results);
        }
        states.extend(panicked);
        states.sort_by(|a, b| a.name.cmp(&b.name));

        let results = fuse(&merged, limit);
        Response {
            query: query.text,
            results,
            providers: states,
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }

    /// Restrict to the requested names, preserving registry order. A request for
    /// names that match nothing returns no providers rather than silently
    /// running them all — the caller sees the empty result and its cause.
    fn select(&self, names: &[String]) -> Vec<Arc<dyn Provider>> {
        if names.is_empty() {
            return self.providers.clone();
        }
        self.providers
            .iter()
            .filter(|p| names.iter().any(|n| n == p.name()))
            .cloned()
            .collect()
    }
}

/// Run one provider under its deadline and classify the outcome. Each result is
/// stamped with its rank in this provider's own ordering, which is what fusion
/// scores on — a provider's rank-1 must count as rank-1 no matter where its
/// results land in the merged concatenation.
async fn run_provider(
    index: usize,
    provider: Arc<dyn Provider>,
    query: String,
    limit: usize,
    budget: Duration,
) -> (usize, ProviderState, Vec<Finding>) {
    let name = provider.name().to_string();
    let started = Instant::now();
    let outcome = tokio::time::timeout(budget, provider.search(query, limit)).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let (status, mut results, error) = match outcome {
        Ok(Ok(results)) => (ProviderStatus::Ok, results, None),
        Ok(Err(error)) => {
            let status = match error.cause {
                FailureCause::Timeout => ProviderStatus::Timeout,
                _ => ProviderStatus::Error,
            };
            (status, Vec::new(), Some(error.message))
        }
        Err(_) => (
            ProviderStatus::Timeout,
            Vec::new(),
            Some(ProviderError::network("provider exceeded its deadline").message),
        ),
    };
    for (rank, finding) in results.iter_mut().enumerate() {
        finding.rank = rank + 1;
    }

    let state = ProviderState {
        name,
        status,
        count: results.len(),
        error,
        elapsed_ms,
    };
    (index, state, results)
}

/// Merge provider answers with reciprocal-rank fusion: each provider votes
/// `1/(k + rank)`, so a URL several independent providers rank well rises above
/// one that only a single provider liked. Duplicates collapse by normalized URL.
fn fuse(results: &[Finding], limit: usize) -> Vec<Finding> {
    const K: f64 = 10.0;
    use std::collections::HashMap;

    struct Aggregate {
        result: Finding,
        score: f64,
        order: usize,
    }

    let mut seen: HashMap<String, Aggregate> = HashMap::new();
    let mut order = 0usize;
    for incoming in results {
        let key = normalize_url(&incoming.url);
        if key.is_empty() {
            continue;
        }
        // The rank comes from the provider's own ordering, stamped before the
        // merge — not from this concatenation's position, which would penalize
        // whichever provider happened to be appended later.
        let rank = incoming.rank.max(1);
        match seen.get_mut(&key) {
            Some(existing) => {
                existing.score += 1.0 / (K + rank as f64);
                if existing.result.snippet.is_none() {
                    existing.result.snippet = incoming.snippet.clone();
                }
                if existing.result.title.is_empty() && !incoming.title.is_empty() {
                    existing.result.title = incoming.title.clone();
                }
                if !existing.result.providers.contains(&incoming.providers) {
                    existing.result.providers.push(',');
                    existing.result.providers.push_str(&incoming.providers);
                }
            }
            None => {
                let result = incoming.clone();
                seen.insert(
                    key,
                    Aggregate {
                        result,
                        score: 1.0 / (K + rank as f64),
                        order,
                    },
                );
                order += 1;
            }
        }
    }

    let mut out: Vec<Aggregate> = seen.into_values().collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.order.cmp(&b.order))
    });
    out.into_iter()
        .take(limit)
        .map(|mut aggregate| {
            aggregate.result.score = aggregate.score;
            aggregate.result
        })
        .collect()
}

/// Normalize a URL for deduplication: lowercased scheme and host, no fragment,
/// no tracking parameters, no trailing slash. It never drops a meaningful query.
pub(super) fn normalize_url(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw.trim()) else {
        return String::new();
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return String::new();
    }
    let _ = url.set_scheme(&url.scheme().to_ascii_lowercase());
    if let Some(host) = url.host_str() {
        let _ = url.set_host(Some(&host.to_ascii_lowercase()));
    }
    url.set_fragment(None);
    let tracking = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "ref",
        "fbclid",
        "gclid",
        "mc_cid",
        "mc_eid",
    ];
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !tracking.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        for (k, v) in &kept {
            pairs.append_pair(k, v);
        }
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(url: &str, providers: &str, rank: usize) -> Finding {
        Finding {
            title: url.into(),
            url: url.into(),
            snippet: None,
            providers: providers.into(),
            score: 0.0,
            rank,
        }
    }

    /// A URL two providers rank highly must beat one only a single provider
    /// liked, and two providers' rank-1s must score the same regardless of which
    /// provider was merged first — the bug where a later provider's rank-1 was
    /// scored as if it were deep in the list.
    #[test]
    fn fusion_prefers_agreement() {
        let merged = vec![
            result("https://a.example/x", "brave", 1),
            result("https://a.example/x", "wikipedia", 1),
            result("https://b.example/y", "brave", 2),
            result("https://c.example/z", "mwmbl", 1),
        ];
        let ranked = fuse(&merged, 10);
        assert_eq!(ranked.len(), 3, "duplicates collapse");
        assert_eq!(ranked[0].url, "https://a.example/x");
        assert!(
            ranked[0].providers.contains("brave") && ranked[0].providers.contains("wikipedia"),
            "both providers are named"
        );
        // The single-vote rank-1 (mwmbl) must outrank the single-vote rank-2.
        let single_rank_one = ranked
            .iter()
            .position(|r| r.url == "https://c.example/z")
            .unwrap();
        let single_rank_two = ranked
            .iter()
            .position(|r| r.url == "https://b.example/y")
            .unwrap();
        assert!(
            single_rank_one < single_rank_two,
            "a later provider's rank-1 must not be penalized by merge order"
        );
    }

    /// Rank is taken from the provider's own ordering, so a rank-1 finding does
    /// not inherit the concatenation position.
    #[test]
    fn fusion_scores_on_provider_rank_not_position() {
        let merged = vec![
            result("https://first.example/a", "brave", 5),
            result("https://second.example/b", "wikipedia", 1),
        ];
        let ranked = fuse(&merged, 10);
        assert_eq!(
            ranked[0].url, "https://second.example/b",
            "the rank-1 finding wins even though it was merged second"
        );
    }

    /// Tracking parameters and fragments must not fragment a result.
    #[test]
    fn url_normalization_dedupes_noise() {
        assert_eq!(
            normalize_url("https://Example.com/Path/?utm_source=news&id=7#frag"),
            normalize_url("https://example.com/Path?id=7")
        );
        assert!(normalize_url("javascript:alert(1)").is_empty());
    }
}
