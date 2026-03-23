// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! Core types for compound resolution.
//!
//! ## Tier: T2-C (π + μ + ∃)
//! Persistence (π) of mapped (μ) existence (∃) — a compound that has been
//! found and persisted with all its identity mappings.

use serde::{Deserialize, Serialize};

/// A resolved compound record.
///
/// Central data structure carrying all identity information for a resolved
/// chemical compound, regardless of which source provided it.
///
/// ## Tier: T2-C (π + μ + ∃)
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundRecord {
    /// Primary name used for resolution (query term).
    pub name: String,
    /// Canonical SMILES string.
    pub smiles: Option<String>,
    /// IUPAC International Chemical Identifier.
    pub inchi: Option<String>,
    /// InChI hash key (27-character standard form).
    pub inchi_key: Option<String>,
    /// CAS Registry Number.
    pub cas_number: Option<String>,
    /// PubChem Compound ID.
    pub pubchem_cid: Option<u64>,
    /// ChEMBL molecule identifier.
    pub chembl_id: Option<String>,
    /// Known synonyms and trade names.
    pub synonyms: Vec<String>,
    /// Which source resolved this record.
    pub source: ResolutionSource,
    /// UTC timestamp of resolution.
    pub resolved_at: nexcore_chrono::DateTime,
}

impl CompoundRecord {
    /// Create a minimal compound record with optional SMILES and a source tag.
    #[must_use]
    pub fn new(name: impl Into<String>, smiles: Option<String>, source: ResolutionSource) -> Self {
        Self {
            name: name.into(),
            smiles,
            inchi: None,
            inchi_key: None,
            cas_number: None,
            pubchem_cid: None,
            chembl_id: None,
            synonyms: Vec::new(),
            source,
            resolved_at: nexcore_chrono::DateTime::now(),
        }
    }
}

/// Where a compound record was resolved from.
///
/// ## Tier: T2-P (λ + ∃)
/// Location (λ) where existence (∃) was confirmed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionSource {
    /// Found in local SQLite cache.
    LocalCache,
    /// Fetched from PubChem REST API.
    PubChem,
    /// Fetched from ChEMBL REST API.
    ChEMBL,
    /// Manually specified.
    Manual,
}

impl std::fmt::Display for ResolutionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolutionSource::LocalCache => write!(f, "local_cache"),
            ResolutionSource::PubChem => write!(f, "pubchem"),
            ResolutionSource::ChEMBL => write!(f, "chembl"),
            ResolutionSource::Manual => write!(f, "manual"),
        }
    }
}

impl std::str::FromStr for ResolutionSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local_cache" => Ok(ResolutionSource::LocalCache),
            "pubchem" => Ok(ResolutionSource::PubChem),
            "chembl" => Ok(ResolutionSource::ChEMBL),
            "manual" => Ok(ResolutionSource::Manual),
            other => Err(format!("Unknown resolution source: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolution_source_display() {
        assert_eq!(ResolutionSource::PubChem.to_string(), "pubchem");
        assert_eq!(ResolutionSource::ChEMBL.to_string(), "chembl");
        assert_eq!(ResolutionSource::LocalCache.to_string(), "local_cache");
        assert_eq!(ResolutionSource::Manual.to_string(), "manual");
    }

    #[test]
    fn test_resolution_source_from_str() {
        use std::str::FromStr;
        assert_eq!(
            ResolutionSource::from_str("pubchem"),
            Ok(ResolutionSource::PubChem)
        );
        assert_eq!(
            ResolutionSource::from_str("chembl"),
            Ok(ResolutionSource::ChEMBL)
        );
        assert!(ResolutionSource::from_str("unknown").is_err());
    }

    #[test]
    fn test_compound_record_roundtrip_json() {
        let record = CompoundRecord {
            name: "aspirin".to_string(),
            smiles: Some("CC(=O)Oc1ccccc1C(=O)O".to_string()),
            inchi: None,
            inchi_key: Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N".to_string()),
            cas_number: Some("50-78-2".to_string()),
            pubchem_cid: Some(2244),
            chembl_id: Some("CHEMBL25".to_string()),
            synonyms: vec!["Aspirin".to_string(), "Acetylsalicylic acid".to_string()],
            source: ResolutionSource::PubChem,
            resolved_at: nexcore_chrono::DateTime::now(),
        };

        let json = serde_json::to_string(&record);
        assert!(json.is_ok());

        if let Ok(json_str) = json {
            let parsed: Result<CompoundRecord, _> = serde_json::from_str(&json_str);
            assert!(parsed.is_ok());
            if let Ok(parsed_record) = parsed {
                assert_eq!(parsed_record.name, "aspirin");
                assert_eq!(parsed_record.pubchem_cid, Some(2244));
                assert_eq!(parsed_record.synonyms.len(), 2);
            }
        }
    }
}
