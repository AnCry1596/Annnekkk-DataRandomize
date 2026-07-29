mod db;
mod handlers;
mod models;
mod utils;

use anyhow::Result;
use log::info;
use moka::future::Cache;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use warp::Filter;

use db::DatabasePool;
use handlers::{bin_body, bin_path, get_randomdata_v2};
use models::{AppState, CachedBinData};

fn with_state(
    state: Arc<AppState>,
) -> impl Filter<Extract = (Arc<AppState>,), Error = Infallible> + Clone {
    warp::any().map(move || state.clone())
}

/// All routes for this server: GET|POST /bin/{bin}, POST /bin, GET /randomdatav2/new
fn routes(
    app_state: Arc<AppState>,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["content-type", "authorization"]);

    // GET /bin/{bin}
    let bin_get = warp::get()
        .and(warp::path("bin"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(with_state(app_state.clone()))
        .and_then(bin_path);

    // POST /bin/{bin}
    let bin_post = warp::post()
        .and(warp::path("bin"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(with_state(app_state.clone()))
        .and_then(bin_path);

    // POST /bin  — body: {"bin": "444444"} or {"bin": 444444}
    let bin_by_body = warp::post()
        .and(warp::path("bin"))
        .and(warp::path::end())
        .and(warp::body::json())
        .and(with_state(app_state.clone()))
        .and_then(bin_body);

    // GET /randomdatav2/new?country=XX
    let randomdata = warp::get()
        .and(warp::path("randomdatav2"))
        .and(warp::path("new"))
        .and(warp::path::end())
        .and(warp::query::<handlers::randomdatav2::RandomDataQuery>())
        .and(warp::header::headers_cloned())
        .and(warp::addr::remote())
        .and(with_state(app_state))
        .and_then(get_randomdata_v2);

    bin_get
        .or(bin_post)
        .or(bin_by_body)
        .or(randomdata)
        .with(cors)
        .with(warp::log("http"))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env first so RUST_LOG can be set there if desired
    dotenv::dotenv().ok();

    // Default to info level — no need to set RUST_LOG manually
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mongo_uri = std::env::var("MONGODB_URI")
        .unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
    let db_name = std::env::var("MONGODB_DB").unwrap_or_else(|_| "random_server".to_string());
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);

    let db = DatabasePool::new(&mongo_uri, &db_name).await?;

    let bin_cache: Cache<i64, CachedBinData> = Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(3600))
        .build();

    // Reference data is read-only between seeds, so load it once instead of
    // running 9-12 $sample aggregations per request.
    let snapshot = db::cache::Snapshot::load(&db).await?;

    let app_state = Arc::new(AppState { db, bin_cache, snapshot });

    let ip: IpAddr = host.parse().unwrap_or_else(|_| [127, 0, 0, 1].into());
    let addr = SocketAddr::new(ip, port);

    info!("Starting server at http://{}", addr);
    warp::serve(routes(app_state)).run(addr).await;

    Ok(())
}

// ── Routing tests ─────────────────────────────────────────────────────────────
// These exercise path/method/query/body matching without a live MongoDB.
// A reachable DB would change the success-path bodies, not the routing.

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::Client;
    use warp::http::StatusCode;

    /// State pointing at an unreachable MongoDB — fine for routing assertions,
    /// since a route that matches still returns a response (500 on DB failure).
    async fn test_state() -> Arc<AppState> {
        // Port 1 is closed; the short serverSelectionTimeout keeps these tests quick
        // instead of waiting out the 30s default per query.
        let client = Client::with_uri_str(
            "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=150&connectTimeoutMS=150",
        )
        .await
        .expect("uri parse");
        Arc::new(AppState {
            db: DatabasePool { db: client.database("test") },
            bin_cache: Cache::builder().max_capacity(10).build(),
            // Empty: randomdatav2 falls back to defaults, as it did when the
            // per-request queries failed against an unreachable DB.
            snapshot: db::cache::Snapshot::empty(),
        })
    }

    #[tokio::test]
    async fn get_bin_non_numeric_is_400() {
        let res = warp::test::request()
            .method("GET")
            .path("/bin/abcdef")
            .reply(&routes(test_state().await))
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = String::from_utf8_lossy(res.body());
        assert!(body.contains("Invalid BIN format"), "body: {}", body);
    }

    #[tokio::test]
    async fn post_bin_path_non_numeric_is_400() {
        let res = warp::test::request()
            .method("POST")
            .path("/bin/abcdef")
            .reply(&routes(test_state().await))
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_bin_body_accepts_string_and_number() {
        // Routed and parsed: reaches the DB (unreachable here) rather than 400/404/405.
        for body in [r#"{"bin":"444444"}"#, r#"{"bin":444444}"#] {
            let res = warp::test::request()
                .method("POST")
                .path("/bin")
                .header("content-type", "application/json")
                .body(body)
                .reply(&routes(test_state().await))
                .await;
            assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR, "body {}", body);
        }
    }

    #[tokio::test]
    async fn bin_over_six_digits_truncates_and_routes() {
        let res = warp::test::request()
            .method("GET")
            .path("/bin/4444441234567")
            .reply(&routes(test_state().await))
            .await;
        // Numeric and truncated to 6 → routed through to the DB lookup.
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn randomdatav2_routes_with_and_without_country() {
        for path in ["/randomdatav2/new", "/randomdatav2/new?country=gb"] {
            let res = warp::test::request()
                .method("GET")
                .path(path)
                .reply(&routes(test_state().await))
                .await;
            // Falls back to defaults when the DB is unreachable, so it still 200s.
            assert_eq!(res.status(), StatusCode::OK, "path {}", path);
            let body = String::from_utf8_lossy(res.body());
            assert!(body.contains("\"success\":true"), "body: {}", body);
            assert!(body.contains("phoneFormatted"), "body: {}", body);
        }
    }

    /// Only /bin/* and /randomdatav2/new are served; nothing else may return
    /// a success status.
    #[tokio::test]
    async fn undefined_routes_are_not_served() {
        for path in ["/license/new", "/admin", "/admin/keys", "/status", "/fraud", "/risk"] {
            let res = warp::test::request()
                .method("GET")
                .path(path)
                .reply(&routes(test_state().await))
                .await;
            assert!(
                res.status().is_client_error(),
                "path {} unexpectedly served: {}",
                path,
                res.status()
            );
        }
    }
}
