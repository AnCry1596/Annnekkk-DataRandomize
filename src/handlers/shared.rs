use chrono::Utc;
use serde::Serialize;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use warp::http::HeaderMap;

use crate::db::cache::Snapshot;
use crate::utils::email_generator::generate_email_from;
use crate::utils::password_generator::generate_password;
use crate::utils::phone_generator::generate_phone;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct PersonalInfo {
    pub first: String,
    pub last: String,
    pub fullname: String,
    pub email: String,
    pub phone: String,
    #[serde(rename = "phoneFormatted")]
    pub phone_formatted: PhoneFormatted,
}

#[derive(Serialize)]
pub struct PhoneFormatted {
    pub parentheses: String,
    pub dashes: String,
    pub dots: String,
    pub international: String,
}

#[derive(Serialize)]
pub struct SecurityInfo {
    pub password: String,
}

#[derive(Serialize)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
    #[serde(rename = "type")]
    pub screen_type: String,
}

#[derive(Serialize)]
pub struct BrowserInfo {
    #[serde(rename = "userAgent")]
    pub user_agent: String,
    pub language: String,
    #[serde(rename = "colorDepth")]
    pub color_depth: u32,
    pub screen: ScreenInfo,
}

#[derive(Serialize)]
pub struct AddressInfo {
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub state: String,
    #[serde(rename = "state_name")]
    pub state_name: Option<String>,
    pub region: Option<String>,
    #[serde(rename = "regionId")]
    pub region_id: Option<i64>,
    #[serde(rename = "postalCode")]
    pub postal_code: String,
    pub country_id: Option<i64>,
    pub country_code: String,
    pub country_name: Option<String>,
}

#[derive(Serialize)]
pub struct LocationInfo {
    #[serde(rename = "timeZone")]
    pub time_zone: String,
    pub offset: i32,
    pub address: AddressInfo,
}

#[derive(Serialize)]
pub struct MiscInfo {
    pub comment: String,
}

#[derive(Serialize)]
pub struct MetadataInfo {
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    #[serde(rename = "processingTime")]
    pub processing_time: String,
    #[serde(rename = "clientIp")]
    pub client_ip: String,
    pub version: String,
    pub format: String,
    pub credits: String,
    pub cached: bool,
}

// ── Timing ────────────────────────────────────────────────────────────────────

/// Format a duration for the `processingTime` field.
///
/// Whole milliseconds floor to "0ms" for anything sub-millisecond, which is now
/// the normal case — requests are served from memory. Sub-millisecond values
/// keep three decimals so the number stays meaningful.
pub fn format_elapsed(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.3}ms", ms)
    } else {
        format!("{:.0}ms", ms)
    }
}

// ── Screen helper ─────────────────────────────────────────────────────────────

static SCREEN_SIZES: &[(u32, u32, &str)] = &[
    (1920, 1080, "desktop"),
    (1366, 768,  "laptop"),
    (1280, 800,  "laptop"),
    (1440, 900,  "desktop"),
    (2560, 1440, "desktop"),
    (390,  844,  "mobile"),
    (414,  896,  "mobile"),
    (375,  667,  "mobile"),
    (1280, 800,  "tablet"),
    (1024, 768,  "tablet"),
];

pub fn random_screen() -> ScreenInfo {
    let idx = rand::random::<usize>() % SCREEN_SIZES.len();
    let (w, h, t) = SCREEN_SIZES[idx];
    ScreenInfo { width: w, height: h, screen_type: t.to_string() }
}

// ── IP extraction ─────────────────────────────────────────────────────────────

pub fn extract_client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if let Some(v) = headers.get("CF-Connecting-IP") {
        if let Ok(s) = v.to_str() {
            let ip = s.trim().to_string();
            if !ip.is_empty() { return ip; }
        }
    }
    if let Some(v) = headers.get("X-Forwarded-For") {
        if let Ok(s) = v.to_str() {
            if let Some(first) = s.split(',').next() {
                let ip = first.trim().to_string();
                if !ip.is_empty() { return ip; }
            }
        }
    }
    peer.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── Assembled random data ─────────────────────────────────────────────────────

pub struct RandomData {
    pub personal: PersonalInfo,
    pub security: SecurityInfo,
    pub browser: BrowserInfo,
    pub location: LocationInfo,
    pub misc: MiscInfo,
    pub metadata: MetadataInfo,
}

/// Assemble a random identity. Every field is served from the in-memory
/// snapshot, so this performs zero MongoDB queries.
/// `start` is taken by value and read at the end, so `processingTime` covers the
/// work below rather than the zero-length span before it.
pub fn build_random_data(
    snap: &Snapshot,
    country_code: &str,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    start: Instant,
    version: &str,
) -> RandomData {
    let first = snap.first_name().unwrap_or("John").to_string();
    let last  = snap.last_name().unwrap_or("Smith").to_string();
    let fullname = format!("{} {}", first, last);

    let addr = snap.address_by_country(country_code);

    let email = generate_email_from(snap, &first, &last);

    let addr_city    = addr.map(|a| a.city.as_str()).unwrap_or_default();
    let addr_state   = addr.map(|a| a.state.as_str()).unwrap_or_default();
    let addr_country = addr.and_then(|a| a.country.as_deref()).unwrap_or(country_code);

    let phone = generate_phone(snap, Some(addr_city), Some(addr_state), Some(addr_country));

    let tz = snap.timezone();
    let time_zone = tz.map(|t| t.name.clone())
        .unwrap_or_else(|| "America/New_York".to_string());
    let tz_offset = tz.map(|t| t.offset).unwrap_or(-300);

    let comment = snap.comment().unwrap_or("All the best!").to_string();

    let ua = snap.user_agent().unwrap_or(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    ).to_string();

    let country_doc = snap.country_by_code_and_state(country_code, addr_state);

    let (country_id, resolved_country_code, country_name) = match country_doc {
        Some(c) => (c.country_id, c.country_code.clone(), Some(c.country_name.clone())),
        None    => (None, country_code.to_string(), None),
    };

    let address_info = AddressInfo {
        address1:    addr.map(|a| a.address1.clone()).unwrap_or_default(),
        address2:    addr.and_then(|a| a.address2.clone()).unwrap_or_default(),
        city:        addr.map(|a| a.city.clone()).unwrap_or_default(),
        state:       addr.map(|a| a.state.clone()).unwrap_or_default(),
        state_name:  country_doc.and_then(|c| c.state_name.clone()),
        region:      country_doc.and_then(|c| c.state_name.clone()),
        region_id:   country_doc.and_then(|c| c.state_id),
        postal_code: addr.map(|a| a.postal_code.clone()).unwrap_or_default(),
        country_id,
        country_code: resolved_country_code,
        country_name,
    };

    RandomData {
        personal: PersonalInfo {
            first,
            last,
            fullname,
            email,
            phone: phone.raw.clone(),
            phone_formatted: PhoneFormatted {
                parentheses:   phone.formatted.clone(),
                dashes:        phone.dashed.clone(),
                dots:          phone.dotted.clone(),
                international: phone.international.clone(),
            },
        },
        security: SecurityInfo {
            password: generate_password(),
        },
        browser: BrowserInfo {
            user_agent: ua,
            language: "en-US".to_string(),
            color_depth: 24,
            screen: random_screen(),
        },
        location: LocationInfo {
            time_zone,
            offset: tz_offset,
            address: address_info,
        },
        misc: MiscInfo { comment },
        metadata: MetadataInfo {
            generated_at:    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            // Read last, so it reflects the work done above.
            processing_time: format_elapsed(start.elapsed()),
            client_ip:       extract_client_ip(headers, peer),
            version:         version.to_string(),
            format:          "structured".to_string(),
            credits:         "Data generated by @annnekkk (https://annnekkk.com)".to_string(),
            cached:          false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_millisecond_does_not_floor_to_zero() {
        // The bug this guards: as_millis() reported "0ms" for all of these.
        assert_eq!(format_elapsed(Duration::from_micros(1)), "0.001ms");
        assert_eq!(format_elapsed(Duration::from_micros(250)), "0.250ms");
        assert_eq!(format_elapsed(Duration::from_micros(999)), "0.999ms");
    }

    #[test]
    fn millisecond_and_above_stay_whole() {
        assert_eq!(format_elapsed(Duration::from_millis(1)), "1ms");
        assert_eq!(format_elapsed(Duration::from_millis(274)), "274ms");
        assert_eq!(format_elapsed(Duration::from_secs(2)), "2000ms");
    }

    #[test]
    fn zero_is_reported_as_zero() {
        assert_eq!(format_elapsed(Duration::ZERO), "0.000ms");
    }
}
