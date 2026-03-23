// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! Local SQLite cache for compound records.
//!
//! Provides a persistence layer so resolved compounds are not re-fetched
//! from remote APIs on subsequent queries.
//!
//! ## Schema
//! ```sql
//! CREATE TABLE compounds (
//!     name TEXT PRIMARY KEY,
//!     smiles TEXT,
//!     inchi TEXT,
//!     inchi_key TEXT,
//!     cas_number TEXT,
//!     pubchem_cid INTEGER,
//!     chembl_id TEXT,
//!     synonyms TEXT,      -- JSON array
//!     source TEXT,
//!     resolved_at TEXT    -- ISO 8601 UTC
//! );
//! ```
//!
//! ## Tier: T2-C (π + μ + ς)
//! Persistence (π) mapping (μ) state (ς) — durable storage of resolved states.

use std::str::FromStr;

use rusqlite::{Connection, params};

use crate::error::{RegistryError, RegistryResult};
use crate::types::{CompoundRecord, ResolutionSource};

const CREATE_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS compounds (
        name        TEXT PRIMARY KEY,
        smiles      TEXT,
        inchi       TEXT,
        inchi_key   TEXT,
        cas_number  TEXT,
        pubchem_cid INTEGER,
        chembl_id   TEXT,
        synonyms    TEXT,
        source      TEXT NOT NULL DEFAULT 'local_cache',
        resolved_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_inchi_key  ON compounds(inchi_key);
    CREATE INDEX IF NOT EXISTS idx_pubchem_cid ON compounds(pubchem_cid);
    CREATE INDEX IF NOT EXISTS idx_chembl_id  ON compounds(chembl_id);
";

/// Local SQLite compound cache.
///
/// Thread-safety: `Connection` is `!Send`. Use one `CacheStore` per task/thread.
/// For shared access across async tasks, wrap in `tokio::sync::Mutex`.
pub struct CacheStore {
    conn: Connection,
}

impl CacheStore {
    /// Open or create a SQLite database at the given path.
    ///
    /// Creates the schema on first open.
    ///
    /// # Errors
    /// Returns `RegistryError::Database` if the database cannot be opened.
    pub fn new(db_path: &str) -> RegistryResult<Self> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    /// Open an in-memory SQLite database (useful for testing).
    ///
    /// # Errors
    /// Returns `RegistryError::Database` on internal SQLite error.
    pub fn new_in_memory() -> RegistryResult<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    fn initialize_schema(&self) -> RegistryResult<()> {
        self.conn.execute_batch(CREATE_TABLE_SQL)?;
        Ok(())
    }

    /// Look up a compound by exact name (case-insensitive).
    ///
    /// Returns `None` if not in cache.
    ///
    /// # Errors
    /// Returns `RegistryError::Database` on query failure.
    pub fn get(&self, name: &str) -> RegistryResult<Option<CompoundRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, smiles, inchi, inchi_key, cas_number, pubchem_cid,
                    chembl_id, synonyms, source, resolved_at
             FROM compounds
             WHERE lower(name) = lower(?1)
             LIMIT 1",
        )?;

        let mut rows = stmt.query(params![name])?;

        match rows.next()? {
            Some(row) => {
                let record = row_to_record(row)?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    /// Look up a compound by InChIKey (exact match).
    pub fn get_by_inchi_key(&self, inchi_key: &str) -> RegistryResult<Option<CompoundRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, smiles, inchi, inchi_key, cas_number, pubchem_cid,
                    chembl_id, synonyms, source, resolved_at
             FROM compounds
             WHERE inchi_key = ?1
             LIMIT 1",
        )?;

        let mut rows = stmt.query(params![inchi_key])?;

        match rows.next()? {
            Some(row) => Ok(Some(row_to_record(row)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace a compound record.
    ///
    /// Uses `INSERT OR REPLACE` so repeated calls with the same name update in place.
    ///
    /// # Errors
    /// Returns `RegistryError::Database` on write failure.
    pub fn put(&self, record: &CompoundRecord) -> RegistryResult<()> {
        let synonyms_json = serde_json::to_string(&record.synonyms).map_err(RegistryError::Json)?;
        let resolved_at = record.resolved_at.to_rfc3339();
        let source = record.source.to_string();

        self.conn.execute(
            "INSERT OR REPLACE INTO compounds
             (name, smiles, inchi, inchi_key, cas_number, pubchem_cid,
              chembl_id, synonyms, source, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.name,
                record.smiles,
                record.inchi,
                record.inchi_key,
                record.cas_number,
                record.pubchem_cid.and_then(|v| i64::try_from(v).ok()),
                record.chembl_id,
                synonyms_json,
                source,
                resolved_at,
            ],
        )?;
        Ok(())
    }

    /// Search compounds by partial name match (case-insensitive LIKE).
    ///
    /// Returns up to `limit` results ordered by name.
    ///
    /// # Errors
    /// Returns `RegistryError::Database` on query failure.
    pub fn search(&self, query: &str, limit: usize) -> RegistryResult<Vec<CompoundRecord>> {
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = self.conn.prepare(
            "SELECT name, smiles, inchi, inchi_key, cas_number, pubchem_cid,
                    chembl_id, synonyms, source, resolved_at
             FROM compounds
             WHERE lower(name) LIKE ?1
                OR lower(synonyms) LIKE ?1
             ORDER BY name
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(
            params![pattern, i64::try_from(limit).unwrap_or(i64::MAX)],
            |row| Ok(row_to_record_rusqlite(row)),
        )?;

        let mut records = Vec::new();
        for row_result in rows {
            match row_result {
                Ok(Ok(record)) => records.push(record),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(RegistryError::Database(e)),
            }
        }

        Ok(records)
    }

    /// Count total compounds in cache.
    pub fn count(&self) -> RegistryResult<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM compounds", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}

/// Convert a rusqlite `Row` reference to `CompoundRecord`.
fn row_to_record(row: &rusqlite::Row<'_>) -> RegistryResult<CompoundRecord> {
    let name: String = row.get(0)?;
    let smiles: Option<String> = row.get(1)?;
    let inchi: Option<String> = row.get(2)?;
    let inchi_key: Option<String> = row.get(3)?;
    let cas_number: Option<String> = row.get(4)?;
    let pubchem_cid_raw: Option<i64> = row.get(5)?;
    let chembl_id: Option<String> = row.get(6)?;
    let synonyms_json: String = row.get(7)?;
    let source_str: String = row.get(8)?;
    let resolved_at_str: String = row.get(9)?;

    let synonyms: Vec<String> = serde_json::from_str(&synonyms_json).unwrap_or_default();
    let source = ResolutionSource::from_str(&source_str).unwrap_or(ResolutionSource::LocalCache);
    let resolved_at = nexcore_chrono::DateTime::parse_from_rfc3339(&resolved_at_str)
        .unwrap_or_else(|_| nexcore_chrono::DateTime::now());

    Ok(CompoundRecord {
        name,
        smiles,
        inchi,
        inchi_key,
        cas_number,
        pubchem_cid: pubchem_cid_raw.and_then(|v| u64::try_from(v).ok()),
        chembl_id,
        synonyms,
        source,
        resolved_at,
    })
}

/// Version for use inside `query_map` closures (takes `&rusqlite::Row`).
fn row_to_record_rusqlite(row: &rusqlite::Row<'_>) -> RegistryResult<CompoundRecord> {
    row_to_record(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aspirin_record() -> CompoundRecord {
        CompoundRecord {
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
        }
    }

    #[test]
    fn test_new_in_memory_creates_schema() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let count = store.count();
            assert!(count.is_ok());
            assert_eq!(count.unwrap_or(u64::MAX), 0);
        }
    }

    #[test]
    fn test_put_and_get() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let record = aspirin_record();
            let put_result = store.put(&record);
            assert!(put_result.is_ok());

            let get_result = store.get("aspirin");
            assert!(get_result.is_ok());
            if let Ok(Some(fetched)) = get_result {
                assert_eq!(fetched.name, "aspirin");
                assert_eq!(fetched.pubchem_cid, Some(2244));
                assert_eq!(fetched.smiles.as_deref(), Some("CC(=O)Oc1ccccc1C(=O)O"));
                assert_eq!(fetched.synonyms.len(), 2);
            } else {
                assert!(false, "Expected Some(record)");
            }
        }
    }

    #[test]
    fn test_get_missing_returns_none() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let result = store.get("nonexistent_drug_xyz");
            assert!(result.is_ok());
            if let Ok(val) = result {
                assert!(val.is_none());
            }
        }
    }

    #[test]
    fn test_put_overwrites_existing() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let mut record = aspirin_record();
            let _ = store.put(&record);

            // Update SMILES and re-insert
            record.smiles = Some("updated_smiles".to_string());
            let put_result = store.put(&record);
            assert!(put_result.is_ok());

            let get_result = store.get("aspirin");
            assert!(get_result.is_ok());
            if let Ok(Some(fetched)) = get_result {
                assert_eq!(fetched.smiles.as_deref(), Some("updated_smiles"));
            }
        }
    }

    #[test]
    fn test_case_insensitive_lookup() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let record = aspirin_record();
            let _ = store.put(&record);

            // Lookup with different case
            let result = store.get("ASPIRIN");
            assert!(result.is_ok());
            if let Ok(val) = result {
                assert!(val.is_some(), "Case-insensitive lookup should find record");
            }
        }
    }

    #[test]
    fn test_search_by_name() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let _ = store.put(&aspirin_record());

            let ibuprofen = CompoundRecord {
                name: "ibuprofen".to_string(),
                smiles: Some("CC(C)Cc1ccc(cc1)C(C)C(=O)O".to_string()),
                inchi: None,
                inchi_key: None,
                cas_number: None,
                pubchem_cid: Some(3672),
                chembl_id: None,
                synonyms: Vec::new(),
                source: ResolutionSource::PubChem,
                resolved_at: nexcore_chrono::DateTime::now(),
            };
            let _ = store.put(&ibuprofen);

            let results = store.search("irin", 10);
            assert!(results.is_ok());
            if let Ok(found) = results {
                assert_eq!(found.len(), 1);
                assert_eq!(found[0].name, "aspirin");
            }
        }
    }

    #[test]
    fn test_count_increments() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            assert_eq!(store.count().unwrap_or(u64::MAX), 0);
            let _ = store.put(&aspirin_record());
            assert_eq!(store.count().unwrap_or(0), 1);
        }
    }

    #[test]
    fn test_get_by_inchi_key() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let _ = store.put(&aspirin_record());
            let result = store.get_by_inchi_key("BSYNRYMUTXBXSQ-UHFFFAOYSA-N");
            assert!(result.is_ok());
            if let Ok(val) = result {
                assert!(val.is_some());
            }
        }
    }

    #[test]
    fn test_source_roundtrip() {
        let store = CacheStore::new_in_memory();
        assert!(store.is_ok());
        if let Ok(store) = store {
            let mut record = aspirin_record();
            record.source = ResolutionSource::ChEMBL;
            let _ = store.put(&record);

            let fetched = store.get("aspirin");
            assert!(fetched.is_ok());
            if let Ok(Some(r)) = fetched {
                assert_eq!(r.source, ResolutionSource::ChEMBL);
            }
        }
    }
}
