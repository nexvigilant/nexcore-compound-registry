// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! PubChem PUG REST API client.
//!
//! Resolves compound names and CIDs via the PubChem REST API.
//!
//! ## API Base
//! `https://pubchem.ncbi.nlm.nih.gov/rest/pug`
//!
//! ## Tier: T2-C (μ + ∃ + λ)
//! Maps (μ) existence (∃) from remote location (λ) — fetches compound identity
//! from PubChem and maps it to `CompoundRecord`.

use serde::Deserialize;

use crate::error::{RegistryError, RegistryResult};
use crate::types::{CompoundRecord, ResolutionSource};

const PUBCHEM_BASE: &str = "https://pubchem.ncbi.nlm.nih.gov/rest/pug";

// ── Internal deserialization types ──────────────────────────────────────────

#[derive(Deserialize)]
struct PugRestResponse {
    #[serde(rename = "PC_Compounds")]
    pc_compounds: Vec<PugCompound>,
}

#[derive(Deserialize)]
struct PugCompound {
    id: PugCompoundId,
    props: Vec<PugProp>,
}

#[derive(Deserialize)]
struct PugCompoundId {
    id: PugIdInner,
}

#[derive(Deserialize)]
struct PugIdInner {
    cid: Option<u64>,
}

#[derive(Deserialize)]
struct PugProp {
    urn: PugUrn,
    value: PugValue,
}

#[derive(Deserialize)]
struct PugUrn {
    label: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct PugValue {
    sval: Option<String>,
    #[allow(
        dead_code,
        reason = "PubChem PUG REST includes fval in the schema; field reserved for future numeric property extraction"
    )]
    fval: Option<f64>,
    #[allow(
        dead_code,
        reason = "PubChem PUG REST includes ival in the schema; field reserved for future integer property extraction"
    )]
    ival: Option<i64>,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Resolve a compound by common name via PubChem.
///
/// Queries `GET /compound/name/{name}/JSON` and parses the first result.
///
/// # Errors
/// - `RegistryError::NotFound` if PubChem returns 404
/// - `RegistryError::Http` on network failure
/// - `RegistryError::InvalidResponse` on unexpected response structure
pub async fn resolve_by_name(
    client: &reqwest::Client,
    name: &str,
) -> RegistryResult<CompoundRecord> {
    let encoded = urlencoding_encode(name);
    let url = format!("{PUBCHEM_BASE}/compound/name/{encoded}/JSON");
    fetch_and_parse(client, &url, name).await
}

/// Resolve a compound by PubChem CID.
///
/// Queries `GET /compound/cid/{cid}/JSON`.
pub async fn resolve_by_cid(client: &reqwest::Client, cid: u64) -> RegistryResult<CompoundRecord> {
    let url = format!("{PUBCHEM_BASE}/compound/cid/{cid}/JSON");
    fetch_and_parse(client, &url, &format!("CID:{cid}")).await
}

/// Simple percent-encoding for URL path segments.
fn urlencoding_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                vec![c]
            } else if c == ' ' {
                vec!['%', '2', '0']
            } else {
                let mut encoded = Vec::new();
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf);
                for byte in bytes.bytes() {
                    encoded.extend(format!("%{byte:02X}").chars());
                }
                encoded
            }
        })
        .collect()
}

async fn fetch_and_parse(
    client: &reqwest::Client,
    url: &str,
    name: &str,
) -> RegistryResult<CompoundRecord> {
    let response = client.get(url).send().await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(RegistryError::NotFound {
            name: name.to_string(),
        });
    }

    if !response.status().is_success() {
        return Err(RegistryError::InvalidResponse {
            message: format!("PubChem returned HTTP {}", response.status()),
        });
    }

    let pug: PugRestResponse = response.json().await?;
    parse_pug_response(name, pug)
}

/// Parse a `PugRestResponse` into a `CompoundRecord`.
///
/// Extracts SMILES, InChI, and InChIKey from the props array.
/// Separated for unit testability without HTTP calls.
fn parse_pug_response(name: &str, pug: PugRestResponse) -> RegistryResult<CompoundRecord> {
    let compound = pug
        .pc_compounds
        .into_iter()
        .next()
        .ok_or_else(|| RegistryError::NotFound {
            name: name.to_string(),
        })?;

    let cid = compound.id.id.cid;
    let mut smiles: Option<String> = None;
    let mut inchi: Option<String> = None;
    let mut inchi_key: Option<String> = None;

    for prop in &compound.props {
        match (prop.urn.label.as_str(), prop.urn.name.as_deref()) {
            ("SMILES", Some("Canonical")) => smiles = prop.value.sval.clone(),
            ("InChI", Some("Standard")) => inchi = prop.value.sval.clone(),
            ("InChIKey", Some("Standard")) => inchi_key = prop.value.sval.clone(),
            _ => {}
        }
    }

    Ok(CompoundRecord {
        name: name.to_string(),
        smiles,
        inchi,
        inchi_key,
        cas_number: None,
        pubchem_cid: cid,
        chembl_id: None,
        synonyms: Vec::new(),
        source: ResolutionSource::PubChem,
        resolved_at: nexcore_chrono::DateTime::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aspirin_pug_response() -> PugRestResponse {
        PugRestResponse {
            pc_compounds: vec![PugCompound {
                id: PugCompoundId {
                    id: PugIdInner { cid: Some(2244) },
                },
                props: vec![
                    PugProp {
                        urn: PugUrn {
                            label: "SMILES".to_string(),
                            name: Some("Canonical".to_string()),
                        },
                        value: PugValue {
                            sval: Some("CC(=O)Oc1ccccc1C(=O)O".to_string()),
                            fval: None,
                            ival: None,
                        },
                    },
                    PugProp {
                        urn: PugUrn {
                            label: "InChI".to_string(),
                            name: Some("Standard".to_string()),
                        },
                        value: PugValue {
                            sval: Some(
                                "InChI=1S/C9H8O4/c1-6(10)13-8-5-3-2-4-7(8)9(11)12/h2-5H,1H3,(H,11,12)"
                                    .to_string(),
                            ),
                            fval: None,
                            ival: None,
                        },
                    },
                    PugProp {
                        urn: PugUrn {
                            label: "InChIKey".to_string(),
                            name: Some("Standard".to_string()),
                        },
                        value: PugValue {
                            sval: Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N".to_string()),
                            fval: None,
                            ival: None,
                        },
                    },
                ],
            }],
        }
    }

    #[test]
    fn test_parse_aspirin_record() {
        let pug = aspirin_pug_response();
        let result = parse_pug_response("aspirin", pug);
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert_eq!(record.name, "aspirin");
            assert_eq!(record.pubchem_cid, Some(2244));
            assert_eq!(record.smiles.as_deref(), Some("CC(=O)Oc1ccccc1C(=O)O"));
            assert_eq!(
                record.inchi_key.as_deref(),
                Some("BSYNRYMUTXBXSQ-UHFFFAOYSA-N")
            );
            assert_eq!(record.source, ResolutionSource::PubChem);
        }
    }

    #[test]
    fn test_parse_empty_compound_list() {
        let pug = PugRestResponse {
            pc_compounds: Vec::new(),
        };
        let result = parse_pug_response("nonexistent", pug);
        assert!(result.is_err());
        assert!(matches!(result, Err(RegistryError::NotFound { .. })));
    }

    #[test]
    fn test_parse_compound_no_smiles() {
        let pug = PugRestResponse {
            pc_compounds: vec![PugCompound {
                id: PugCompoundId {
                    id: PugIdInner { cid: Some(999) },
                },
                props: Vec::new(),
            }],
        };
        let result = parse_pug_response("unknown", pug);
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert!(record.smiles.is_none());
            assert_eq!(record.pubchem_cid, Some(999));
        }
    }

    #[test]
    fn test_parse_ignores_non_canonical_smiles() {
        let pug = PugRestResponse {
            pc_compounds: vec![PugCompound {
                id: PugCompoundId {
                    id: PugIdInner { cid: Some(1) },
                },
                props: vec![PugProp {
                    urn: PugUrn {
                        label: "SMILES".to_string(),
                        name: Some("Isomeric".to_string()),
                    },
                    value: PugValue {
                        sval: Some("should-be-ignored".to_string()),
                        fval: None,
                        ival: None,
                    },
                }],
            }],
        };
        let result = parse_pug_response("test", pug);
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert!(record.smiles.is_none());
        }
    }

    #[test]
    fn test_urlencoding_spaces() {
        let encoded = urlencoding_encode("acetyl salicylic acid");
        assert!(encoded.contains("%20"), "Space should be encoded as %20");
        assert!(!encoded.contains(' '), "No raw spaces in encoded output");
    }

    #[test]
    fn test_urlencoding_alphanumeric_passthrough() {
        let encoded = urlencoding_encode("aspirin");
        assert_eq!(encoded, "aspirin");
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_aspirin_by_name() {
        let client = reqwest::Client::new();
        let result = resolve_by_name(&client, "aspirin").await;
        assert!(result.is_ok());
        if let Ok(record) = result {
            assert_eq!(record.pubchem_cid, Some(2244));
            assert!(record.smiles.is_some());
        }
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_aspirin_by_cid() {
        let client = reqwest::Client::new();
        let result = resolve_by_cid(&client, 2244).await;
        assert!(result.is_ok());
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn integration_resolve_invalid_name() {
        let client = reqwest::Client::new();
        let result = resolve_by_name(&client, "xyzzy_not_a_compound_12345").await;
        assert!(result.is_err());
    }
}
