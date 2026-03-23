// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! ChEMBL REST API client.
//!
//! Resolves compound names and fetches bioactivity data via the EBI ChEMBL API.
//!
//! ## API Base
//! `https://www.ebi.ac.uk/chembl/api/data`
//!
//! ## Tier: T2-C (μ + ∃ + λ + ν)
//! Maps (μ) existence (∃) with frequency data (ν) from remote location (λ).

use serde::Deserialize;

use crate::error::{RegistryError, RegistryResult};
use crate::types::{CompoundRecord, ResolutionSource};

const CHEMBL_BASE: &str = "https://www.ebi.ac.uk/chembl/api/data";

// ── Internal deserialization types ──────────────────────────────────────────

#[derive(Deserialize)]
struct ChemblMoleculeSearchResponse {
    molecules: Vec<ChemblMolecule>,
}

#[derive(Deserialize)]
struct ChemblMolecule {
    molecule_chembl_id: String,
    pref_name: Option<String>,
    molecule_structures: Option<ChemblStructures>,
    molecule_synonyms: Option<Vec<ChemblSynonym>>,
}

#[derive(Deserialize)]
struct ChemblStructures {
    canonical_smiles: Option<String>,
    standard_inchi: Option<String>,
    standard_inchi_key: Option<String>,
}

#[derive(Deserialize)]
struct ChemblSynonym {
    molecule_synonym: String,
}

/// A ChEMBL-specific compound record with bioactivity metadata.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ChemblRecord {
    /// ChEMBL molecule identifier (e.g. "CHEMBL25").
    pub chembl_id: String,
    /// Preferred compound name.
    pub pref_name: Option<String>,
    /// Canonical SMILES from ChEMBL.
    pub smiles: Option<String>,
    /// Standard InChI.
    pub inchi: Option<String>,
    /// Standard InChIKey.
    pub inchi_key: Option<String>,
    /// Synonym list.
    pub synonyms: Vec<String>,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Resolve a compound by name via ChEMBL molecule search.
///
/// Queries `GET /molecule/search?q={name}&format=json` and returns the
/// first match as a `CompoundRecord`.
///
/// # Errors
/// - `RegistryError::NotFound` if no molecules match
/// - `RegistryError::Http` on network failure
pub async fn resolve_by_name(
    client: &reqwest::Client,
    name: &str,
) -> RegistryResult<CompoundRecord> {
    let chembl = fetch_chembl_molecule(client, name).await?;
    Ok(chembl_record_to_compound(name, chembl))
}

/// Enrich an existing `CompoundRecord` with ChEMBL identifiers.
///
/// If the record already has a `chembl_id`, returns it unchanged.
/// Otherwise searches ChEMBL by name and merges identifiers and synonyms.
pub async fn enrich_record(
    client: &reqwest::Client,
    mut record: CompoundRecord,
) -> RegistryResult<CompoundRecord> {
    if record.chembl_id.is_some() {
        return Ok(record);
    }

    match fetch_chembl_molecule(client, &record.name.clone()).await {
        Ok(chembl) => {
            record.chembl_id = Some(chembl.chembl_id.clone());
            for syn in &chembl.synonyms {
                if !record.synonyms.contains(syn) {
                    record.synonyms.push(syn.clone());
                }
            }
            if record.smiles.is_none() {
                record.smiles = chembl.smiles;
            }
            if record.inchi.is_none() {
                record.inchi = chembl.inchi;
            }
            if record.inchi_key.is_none() {
                record.inchi_key = chembl.inchi_key;
            }
        }
        Err(RegistryError::NotFound { .. }) => {
            // Not in ChEMBL — acceptable, return record as-is
        }
        Err(e) => return Err(e),
    }

    Ok(record)
}

async fn fetch_chembl_molecule(
    client: &reqwest::Client,
    name: &str,
) -> RegistryResult<ChemblRecord> {
    let url = format!("{CHEMBL_BASE}/molecule/search?q={name}&format=json");
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(RegistryError::InvalidResponse {
            message: format!("ChEMBL returned HTTP {}", response.status()),
        });
    }

    let search: ChemblMoleculeSearchResponse = response.json().await?;
    let molecule = search
        .molecules
        .into_iter()
        .next()
        .ok_or_else(|| RegistryError::NotFound {
            name: name.to_string(),
        })?;

    Ok(parse_chembl_molecule(molecule))
}

/// Parse a `ChemblMolecule` into a `ChemblRecord`.
///
/// Extracted for unit testability.
fn parse_chembl_molecule(molecule: ChemblMolecule) -> ChemblRecord {
    let synonyms = molecule
        .molecule_synonyms
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.molecule_synonym)
        .collect();

    let (smiles, inchi, inchi_key) = match molecule.molecule_structures {
        Some(s) => (s.canonical_smiles, s.standard_inchi, s.standard_inchi_key),
        None => (None, None, None),
    };

    ChemblRecord {
        chembl_id: molecule.molecule_chembl_id,
        pref_name: molecule.pref_name,
        smiles,
        inchi,
        inchi_key,
        synonyms,
    }
}

fn chembl_record_to_compound(name: &str, chembl: ChemblRecord) -> CompoundRecord {
    CompoundRecord {
        name: name.to_string(),
        smiles: chembl.smiles,
        inchi: chembl.inchi,
        inchi_key: chembl.inchi_key,
        cas_number: None,
        pubchem_cid: None,
        chembl_id: Some(chembl.chembl_id),
        synonyms: chembl.synonyms,
        source: ResolutionSource::ChEMBL,
        resolved_at: nexcore_chrono::DateTime::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aspirin_chembl_molecule() -> ChemblMolecule {
        ChemblMolecule {
            molecule_chembl_id: "CHEMBL25".to_string(),
            pref_name: Some("ASPIRIN".to_string()),
            molecule_structures: Some(ChemblStructures {
                canonical_smiles: Some("CC(=O)Oc1ccccc1C(=O)O".to_string()),
                standard_inchi: Some(
                    "InChI=1S/C9H8O4/c1-6(10)13-8-5-3-2-4-7(8)9(11)12/h2-5H,1H3,(H,11,12)"
                        .to_string(),
                ),
                standard_inchi_key: Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N".to_string()),
            }),
            molecule_synonyms: Some(vec![
                ChemblSynonym {
                    molecule_synonym: "Aspirin".to_string(),
                },
                ChemblSynonym {
                    molecule_synonym: "Acetylsalicylic acid".to_string(),
                },
            ]),
        }
    }

    #[test]
    fn test_parse_aspirin_chembl() {
        let molecule = aspirin_chembl_molecule();
        let record = parse_chembl_molecule(molecule);

        assert_eq!(record.chembl_id, "CHEMBL25");
        assert_eq!(record.pref_name.as_deref(), Some("ASPIRIN"));
        assert_eq!(record.smiles.as_deref(), Some("CC(=O)Oc1ccccc1C(=O)O"));
        assert_eq!(
            record.inchi_key.as_deref(),
            Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N")
        );
        assert_eq!(record.synonyms.len(), 2);
        assert!(record.synonyms.contains(&"Aspirin".to_string()));
    }

    #[test]
    fn test_parse_molecule_no_structures() {
        let molecule = ChemblMolecule {
            molecule_chembl_id: "CHEMBL999".to_string(),
            pref_name: None,
            molecule_structures: None,
            molecule_synonyms: None,
        };
        let record = parse_chembl_molecule(molecule);
        assert_eq!(record.chembl_id, "CHEMBL999");
        assert!(record.smiles.is_none());
        assert!(record.synonyms.is_empty());
    }

    #[test]
    fn test_parse_molecule_empty_synonyms() {
        let molecule = ChemblMolecule {
            molecule_chembl_id: "CHEMBL1".to_string(),
            pref_name: Some("TEST".to_string()),
            molecule_structures: None,
            molecule_synonyms: Some(Vec::new()),
        };
        let record = parse_chembl_molecule(molecule);
        assert!(record.synonyms.is_empty());
    }

    #[test]
    fn test_chembl_record_maps_to_compound() {
        let molecule = aspirin_chembl_molecule();
        let chembl = parse_chembl_molecule(molecule);
        let compound = chembl_record_to_compound("aspirin", chembl);

        assert_eq!(compound.name, "aspirin");
        assert_eq!(compound.chembl_id.as_deref(), Some("CHEMBL25"));
        assert_eq!(compound.source, ResolutionSource::ChEMBL);
        assert!(compound.pubchem_cid.is_none());
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_aspirin_chembl() {
        let client = reqwest::Client::new();
        let result = resolve_by_name(&client, "aspirin").await;
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert!(record.chembl_id.is_some());
            assert!(record.smiles.is_some());
        }
    }
}
