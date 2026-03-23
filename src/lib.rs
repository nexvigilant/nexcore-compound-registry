// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! # nexcore-compound-registry
//!
//! Compound resolution library with PubChem/ChEMBL REST clients and
//! local SQLite cache.
//!
//! ## Pipeline
//!
//! ```text
//! resolve(name)
//!   ├─ Layer 1: SQLite cache    (instant, no network)
//!   ├─ Layer 2: PubChem API     (structure, identifiers)
//!   └─ Layer 3: ChEMBL API      (bioactivity enrichment)
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use nexcore_compound_registry::CacheStore;
//!
//! // Open in-memory cache (returns RegistryResult)
//! let store = CacheStore::new_in_memory();
//! assert!(store.is_ok());
//! ```
//!
//! ## Tier: T3 (σ + → + π + μ + ∃)
//! Sequence (σ) of causal (→) resolution steps that persist (π) mapped (μ)
//! existing (∃) compounds.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod cache;
pub mod chembl;
pub mod error;
pub mod pubchem;
pub mod resolver;
pub mod types;

// ── Convenience re-exports ───────────────────────────────────────────────────

pub use cache::CacheStore;
pub use error::{RegistryError, RegistryResult};
pub use resolver::{resolve, resolve_batch};
pub use types::{CompoundRecord, ResolutionSource};
