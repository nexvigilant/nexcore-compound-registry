// Copyright © 2026 NexVigilant LLC. All Rights Reserved.
// Intellectual Property of Matthew Alexander Campion, PharmD

//! Error types for compound registry operations.

use nexcore_error::Error;

/// Errors from compound registry operations.
///
/// ## Tier: T2-P (∂ + ∃)
/// Boundary errors (∂) asserting existence failures (∃).
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Compound not found in any source.
    #[error("Compound not found: {name}")]
    NotFound { name: String },

    /// HTTP request to external API failed.
    #[error("API request failed: {source}")]
    Http {
        #[from]
        source: reqwest::Error,
    },

    /// JSON deserialization error.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// Database error.
    #[error("Cache database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Invalid response structure from external API.
    #[error("Invalid API response: {message}")]
    InvalidResponse { message: String },

    /// Rate limit exceeded on external API.
    #[error("Rate limit exceeded for {service}")]
    RateLimit { service: String },

    /// Resolution pipeline exhausted all sources.
    #[error("Resolution pipeline exhausted for compound: {name}")]
    ResolutionExhausted { name: String },
}

/// Result type for compound registry operations.
pub type RegistryResult<T> = Result<T, RegistryError>;
