// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! Layered compound resolution pipeline.
//!
//! Resolves a compound name through three ordered sources:
//!
//! 1. **Local cache** — instant, no network
//! 2. **PubChem** — free public API, primary structure source
//! 3. **ChEMBL** — bioactivity enrichment merged into PubChem record
//!
//! Once resolved from any remote source, the result is persisted to cache.
//!
//! ## Tier: T3 (σ + → + π + μ)
//! Sequence (σ) of causal (→) steps that persist (π) a mapped (μ) compound.

use crate::cache::CacheStore;
use crate::error::{RegistryError, RegistryResult};
use crate::types::{CompoundRecord, ResolutionSource};

/// Resolve a compound name through the three-layer pipeline.
///
/// ## Resolution order
/// 1. Local SQLite cache (zero network cost)
/// 2. PubChem PUG REST API
/// 3. ChEMBL molecule search (enrichment layer, non-fatal on failure)
///
/// The enriched record is persisted to cache before returning.
///
/// # Errors
/// - `RegistryError::ResolutionExhausted` if all sources find nothing
/// - `RegistryError::Database` on cache access failure
/// - `RegistryError::Http` / `RegistryError::InvalidResponse` on API errors
///
/// # Note on `Send`
/// This future is `!Send` because `rusqlite::Connection` uses `RefCell` internally.
/// Use one `CacheStore` per async task; wrap in `tokio::sync::Mutex` for shared access.
#[allow(
    clippy::future_not_send,
    reason = "rusqlite::Connection wraps RefCell and is intentionally !Send; callers must not share CacheStore across tasks without a Mutex"
)]
pub async fn resolve(
    name: &str,
    store: &CacheStore,
    client: &reqwest::Client,
) -> RegistryResult<CompoundRecord> {
    // Layer 1: Local cache — O(1) lookup, no network
    if let Some(cached) = store.get(name)? {
        tracing::debug!(compound = %name, "Cache hit");
        return Ok(cached);
    }
    tracing::debug!(compound = %name, "Cache miss — querying PubChem");

    // Layer 2: PubChem
    let pubchem_result = crate::pubchem::resolve_by_name(client, name).await;

    match pubchem_result {
        Ok(pubchem_record) => {
            tracing::debug!(compound = %name, "PubChem resolved");

            // Layer 3: ChEMBL enrichment (non-fatal — PubChem record is sufficient)
            let final_record =
                match crate::chembl::enrich_record(client, pubchem_record.clone()).await {
                    Ok(enriched) => enriched,
                    Err(e) => {
                        tracing::warn!(
                            compound = %name,
                            error = %e,
                            "ChEMBL enrichment failed — using PubChem-only record"
                        );
                        pubchem_record
                    }
                };

            if let Err(e) = store.put(&final_record) {
                tracing::warn!(compound = %name, error = %e, "Failed to cache resolved record");
            }

            Ok(final_record)
        }

        Err(RegistryError::NotFound { .. }) => {
            tracing::debug!(compound = %name, "PubChem: not found — trying ChEMBL direct");

            match crate::chembl::resolve_by_name(client, name).await {
                Ok(record) => {
                    if let Err(e) = store.put(&record) {
                        tracing::warn!(
                            compound = %name,
                            error = %e,
                            "Failed to cache ChEMBL record"
                        );
                    }
                    Ok(record)
                }
                Err(RegistryError::NotFound { .. }) => Err(RegistryError::ResolutionExhausted {
                    name: name.to_string(),
                }),
                Err(e) => Err(e),
            }
        }

        Err(e) => Err(e),
    }
}

/// Resolve multiple compounds sequentially.
///
/// Each name is resolved independently. Failures per-compound are collected
/// rather than short-circuiting the batch.
///
/// Returns a `Vec<(name, result)>` preserving input order.
///
/// # Note on `Send`
/// This future is `!Send` for the same reason as [`resolve`]: `rusqlite::Connection`
/// is `!Send`. Use one `CacheStore` per async task.
#[allow(
    clippy::future_not_send,
    reason = "rusqlite::Connection wraps RefCell and is intentionally !Send; callers must not share CacheStore across tasks without a Mutex"
)]
pub async fn resolve_batch(
    names: &[&str],
    store: &CacheStore,
    client: &reqwest::Client,
) -> Vec<(String, RegistryResult<CompoundRecord>)> {
    let mut results = Vec::with_capacity(names.len());
    for &name in names {
        let result = resolve(name, store, client).await;
        results.push((name.to_string(), result));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheStore;
    use crate::types::ResolutionSource;

    fn make_cached_aspirin() -> CompoundRecord {
        CompoundRecord {
            name: "aspirin".to_string(),
            smiles: Some("CC(=O)Oc1ccccc1C(=O)O".to_string()),
            inchi: None,
            inchi_key: Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N".to_string()),
            cas_number: None,
            pubchem_cid: Some(2244),
            chembl_id: Some("CHEMBL25".to_string()),
            synonyms: vec!["Acetylsalicylic acid".to_string()],
            source: ResolutionSource::LocalCache,
            resolved_at: nexcore_chrono::DateTime::now(),
        }
    }

    #[test]
    fn test_cache_hit_returns_correct_record() {
        let store_result = CacheStore::new_in_memory();
        assert!(store_result.is_ok());
        if let Ok(store) = store_result {
            let record = make_cached_aspirin();
            assert!(store.put(&record).is_ok());

            // Layer 1 directly: store.get should return the cached record
            let get_result = store.get("aspirin");
            assert!(get_result.is_ok());
            if let Ok(Some(cached)) = get_result {
                assert_eq!(cached.name, "aspirin");
                assert_eq!(cached.pubchem_cid, Some(2244));
                assert_eq!(cached.source, ResolutionSource::LocalCache);
            } else {
                assert!(false, "Expected cached record");
            }
        }
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let store_result = CacheStore::new_in_memory();
        assert!(store_result.is_ok());
        if let Ok(store) = store_result {
            let result = store.get("not_cached_compound");
            assert!(result.is_ok());
            if let Ok(val) = result {
                assert!(val.is_none());
            }
        }
    }

    #[tokio::test]
    async fn test_resolve_batch_empty_input_returns_empty() {
        let store_result = CacheStore::new_in_memory();
        assert!(store_result.is_ok());
        if let Ok(store) = store_result {
            let client = reqwest::Client::new();
            let results = resolve_batch(&[], &store, &client).await;
            assert!(results.is_empty());
        }
    }

    #[tokio::test]
    async fn test_resolve_batch_preserves_order() {
        // Seed cache with two compounds so no HTTP calls needed
        let store_result = CacheStore::new_in_memory();
        assert!(store_result.is_ok());
        if let Ok(store) = store_result {
            let aspirin = make_cached_aspirin();
            let ibuprofen = CompoundRecord {
                name: "ibuprofen".to_string(),
                smiles: Some("CC(C)Cc1ccc(cc1)C(C)C(=O)O".to_string()),
                inchi: None,
                inchi_key: None,
                cas_number: None,
                pubchem_cid: Some(3672),
                chembl_id: None,
                synonyms: Vec::new(),
                source: ResolutionSource::LocalCache,
                resolved_at: nexcore_chrono::DateTime::now(),
            };
            assert!(store.put(&aspirin).is_ok());
            assert!(store.put(&ibuprofen).is_ok());

            let client = reqwest::Client::new();
            let results = resolve_batch(&["aspirin", "ibuprofen"], &store, &client).await;

            assert_eq!(results.len(), 2);
            assert_eq!(results[0].0, "aspirin");
            assert_eq!(results[1].0, "ibuprofen");
            assert!(results[0].1.is_ok());
            assert!(results[1].1.is_ok());
        }
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_aspirin_full_pipeline() {
        let store = CacheStore::new_in_memory().expect("in-memory DB");
        let client = reqwest::Client::new();

        let result = resolve("aspirin", &store, &client).await;
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert!(record.smiles.is_some());
            assert_eq!(record.pubchem_cid, Some(2244));
        }

        // Second call must hit cache
        let cached = resolve("aspirin", &store, &client).await;
        assert!(cached.is_ok());
        if let Ok(record) = cached {
            assert_eq!(record.source, ResolutionSource::LocalCache);
        }
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_invalid_exhausts_pipeline() {
        let store = CacheStore::new_in_memory().expect("in-memory DB");
        let client = reqwest::Client::new();

        let result = resolve("xyzzy_not_a_compound_99999", &store, &client).await;
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(RegistryError::ResolutionExhausted { .. })
        ));
    }
}
