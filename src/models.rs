use moka::future::Cache;
use serde::{Deserialize, Serialize};

use crate::db::DatabasePool;

/// Card type and classification info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardInfo {
    #[serde(rename = "type")]
    pub card_type: String,
    pub sub_type: String,
    pub category: String,
    pub regulated: String,
}

/// BIN-level metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinInfo {
    pub category: String,
    pub length: String,
}

/// Issuing bank and country details
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issuer {
    pub bank: String,
    pub country_code: String,
    pub country: String,
}

/// Response metadata (cache status, timing)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub cached: bool,
    pub timestamp: String,
    pub processing_time: String,
    pub cache_stats: String,
    pub credits: String,
}

/// Successful BIN lookup response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinResponse {
    pub success: bool,
    pub bin: String,
    pub card_info: CardInfo,
    pub bin_info: BinInfo,
    pub issuer: Issuer,
    pub metadata: Metadata,
}

/// Error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
}

/// In-memory cached BIN data (metadata excluded — added on response)
#[derive(Debug, Clone)]
pub struct CachedBinData {
    pub bin: String,
    pub card_info: CardInfo,
    pub bin_info: BinInfo,
    pub issuer: Issuer,
}

/// Shared application state
pub struct AppState {
    pub db: DatabasePool,
    pub bin_cache: Cache<i64, CachedBinData>,
}
