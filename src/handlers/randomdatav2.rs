use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use warp::http::HeaderMap;
use warp::reply;

use crate::models::AppState;
use crate::handlers::shared::{
    BrowserInfo, LocationInfo, MetadataInfo, MiscInfo, PersonalInfo, RandomData,
    SecurityInfo, build_random_data,
};

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RandomDataQuery {
    country: Option<String>,
}

// ── Response ──────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct RandomDataV2Response {
    pub success: bool,
    pub personal: PersonalInfo,
    pub security: SecurityInfo,
    pub browser: BrowserInfo,
    pub location: LocationInfo,
    pub misc: MiscInfo,
    pub metadata: MetadataInfo,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// GET /randomdatav2/new?country=XX  (default: US)
pub async fn get_randomdata_v2(
    query: RandomDataQuery,
    headers: HeaderMap,
    peer: Option<SocketAddr>,
    state: Arc<AppState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let start = Instant::now();
    let pool = &state.db;

    let country_code = query
        .country
        .as_deref()
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| "US".to_string());

    let RandomData { personal, security, browser, location, misc, metadata } =
        build_random_data(pool, &country_code, &headers, peer, start.elapsed(), "2.2-rust").await;

    Ok(reply::json(&RandomDataV2Response {
        success: true,
        personal,
        security,
        browser,
        location,
        misc,
        metadata,
    }))
}
