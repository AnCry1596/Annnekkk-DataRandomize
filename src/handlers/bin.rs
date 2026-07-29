use chrono::Utc;
use log::{info, warn, error};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;
use warp::http::StatusCode;
use warp::reply::{self, Reply};

use crate::db::BinDocument;
use crate::handlers::shared::format_elapsed;
use crate::models::{AppState, BinInfo, BinResponse, CachedBinData, CardInfo, ErrorResponse, Issuer, Metadata};

// ── POST body: accepts {"bin": "444444"} or {"bin": 444444} ──────────────────

#[derive(Deserialize)]
pub struct BinBody {
    bin: BinField,
}

/// Accepts both a JSON string and a JSON number for the "bin" field
#[derive(Deserialize)]
#[serde(untagged)]
enum BinField {
    Str(String),
    Num(i64),
}

impl BinField {
    fn as_string(&self) -> String {
        match self {
            BinField::Str(s) => s.clone(),
            BinField::Num(n) => n.to_string(),
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn normalize_bin(bin: &str) -> &str {
    if bin.len() > 6 { &bin[..6] } else { bin }
}

fn err(status: StatusCode, msg: String) -> reply::Response {
    reply::with_status(
        reply::json(&ErrorResponse { success: false, error: msg }),
        status,
    )
    .into_response()
}

pub async fn lookup_bin(raw: String, state: Arc<AppState>) -> reply::Response {
    let start = Instant::now();
    let bin_str_owned = normalize_bin(&raw).to_string();
    let bin_str = bin_str_owned.as_str();

    let bin_id: i64 = match bin_str.parse() {
        Ok(id) => id,
        Err(_) => {
            warn!("BIN lookup rejected — non-numeric input: {:?}", bin_str);
            return err(StatusCode::BAD_REQUEST, "Invalid BIN format. Must be numeric.".to_string());
        }
    };

    // Cache hit
    if let Some(cached) = state.bin_cache.get(&bin_id).await {
        info!("[CACHE HIT ] BIN {} — {}ms", bin_str, start.elapsed().as_millis());
        return reply::json(&BinResponse {
            success: true,
            bin: cached.bin.clone(),
            card_info: cached.card_info.clone(),
            bin_info: cached.bin_info.clone(),
            issuer: cached.issuer.clone(),
            metadata: Metadata {
                cached: true,
                timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                processing_time: format_elapsed(start.elapsed()),
                cache_stats: "hit".to_string(),
                credits: "Data provided by @annnekkk (https://annnekkk.com)".to_string(),
            },
        })
        .into_response();
    }

    // Cache miss — query MongoDB
    match BinDocument::find_by_id(&state.db, bin_id).await {
        Ok(Some(doc)) => {
            let card_info = CardInfo {
                card_type: doc.card_type.unwrap_or_else(|| "Unknown".to_string()),
                sub_type:  doc.sub_type.unwrap_or_else(|| "Unknown".to_string()),
                category:  doc.category.unwrap_or_else(|| "Unknown".to_string()),
                regulated: doc.regulated.unwrap_or_else(|| "N".to_string()),
            };
            let bin_info = BinInfo {
                category: doc.bin_category.unwrap_or_else(|| "Unknown".to_string()),
                length:   doc.bin_length.unwrap_or_else(|| "Unknown".to_string()),
            };
            let issuer = Issuer {
                bank:         doc.bank.unwrap_or_else(|| "Unknown".to_string()),
                country_code: doc.country_code.unwrap_or_else(|| "Unknown".to_string()),
                country:      doc.country.unwrap_or_else(|| "Unknown".to_string()),
            };

            state.bin_cache.insert(bin_id, CachedBinData {
                bin:       bin_str.to_string(),
                card_info: card_info.clone(),
                bin_info:  bin_info.clone(),
                issuer:    issuer.clone(),
            }).await;

            info!("[DB HIT    ] BIN {} — {} ({}) — {}ms",
                bin_str, issuer.bank, issuer.country, start.elapsed().as_millis());

            reply::json(&BinResponse {
                success: true,
                bin: bin_str.to_string(),
                card_info,
                bin_info,
                issuer,
                metadata: Metadata {
                    cached: false,
                    timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                    processing_time: format_elapsed(start.elapsed()),
                    cache_stats: "miss".to_string(),
                    credits: "Data provided by @annnekkk (https://annnekkk.com)".to_string(),
                },
            })
            .into_response()
        }
        Ok(None) => {
            warn!("[NOT FOUND ] BIN {} — {}ms", bin_str, start.elapsed().as_millis());
            err(StatusCode::NOT_FOUND, format!("BIN {} not found", bin_str))
        }
        Err(e) => {
            error!("[DB ERROR  ] BIN {} — {}", bin_str, e);
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
        }
    }
}

// ── Route handlers ────────────────────────────────────────────────────────────

/// GET /bin/{bin} and POST /bin/{bin}
pub async fn bin_path(bin: String, state: Arc<AppState>) -> Result<reply::Response, warp::Rejection> {
    Ok(lookup_bin(bin, state).await)
}

/// POST /bin — body: {"bin": "444444"} or {"bin": 444444}
pub async fn bin_body(body: BinBody, state: Arc<AppState>) -> Result<reply::Response, warp::Rejection> {
    Ok(lookup_bin(body.bin.as_string(), state).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_truncates_to_six() {
        assert_eq!(normalize_bin("4444441234567"), "444444");
        assert_eq!(normalize_bin("444444"), "444444");
        assert_eq!(normalize_bin("4444"), "4444");
    }

    #[test]
    fn bin_field_accepts_string_and_number() {
        let s: BinBody = serde_json::from_str(r#"{"bin":"444444"}"#).unwrap();
        let n: BinBody = serde_json::from_str(r#"{"bin":444444}"#).unwrap();
        assert_eq!(s.bin.as_string(), "444444");
        assert_eq!(n.bin.as_string(), "444444");
    }
}
