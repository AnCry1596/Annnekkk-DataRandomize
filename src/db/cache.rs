//! In-memory snapshot of the random-source collections.
//!
//! Every one of these collections is read-only reference data that changes only
//! when `seed` runs. Sampling them from MongoDB per request cost 9-12 round
//! trips, and any `$sample` behind a `$match` is a full COLLSCAN + in-memory
//! sort. Loading them once at startup turns all of that into an array index.
//!
//! ponytail: whole-collection load at boot, no TTL refresh. Total is ~50MB of
//! strings. If the data ever becomes mutable at runtime, add a periodic reload
//! that swaps a new Arc<Snapshot> into an ArcSwap.

use anyhow::{Context, Result};
use futures::TryStreamExt;
use log::info;
use mongodb::bson::{doc, Document};
use rand::Rng;
use std::collections::HashMap;

use crate::db::DatabasePool;

/// One phone prefix, pre-split into the lookup keys the handler filters on.
#[derive(Debug, Clone)]
pub struct PhonePrefix {
    pub npa: String,
    pub nxx: String,
}

/// One address, matching the shape `AddressDocument` returns.
#[derive(Debug, Clone)]
pub struct Address {
    pub address1: String,
    pub address2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: Option<String>,
}

/// One country/state record.
#[derive(Debug, Clone)]
pub struct Country {
    pub country_id: Option<i64>,
    pub country_code: String,
    pub country_name: String,
    pub state_id: Option<i64>,
    pub state_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Timezone {
    pub name: String,
    pub offset: i32,
}

/// All reference data, loaded once at startup.
///
/// The `*_by_*` maps hold indices into the matching flat vec, so a filtered
/// random pick is one hash lookup plus one array index — no scanning.
pub struct Snapshot {
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
    pub words: Vec<String>,
    pub domains: Vec<String>,
    pub user_agents: Vec<String>,
    pub comments: Vec<String>,
    pub timezones: Vec<Timezone>,

    pub phone_prefixes: Vec<PhonePrefix>,
    /// lowercased city -> indices into `phone_prefixes`
    phones_by_city: HashMap<String, Vec<u32>>,
    /// uppercased state_code -> indices into `phone_prefixes`
    phones_by_state: HashMap<String, Vec<u32>>,
    /// uppercased country -> indices into `phone_prefixes`
    phones_by_country: HashMap<String, Vec<u32>>,

    pub addresses: Vec<Address>,
    /// uppercased country -> indices into `addresses`
    addresses_by_country: HashMap<String, Vec<u32>>,

    /// (country_code, state_code) both uppercased -> index into `countries`
    countries_by_code_state: HashMap<(String, String), u32>,
    /// uppercased country_code -> index into `countries` (first record wins)
    countries_by_code: HashMap<String, u32>,
    pub countries: Vec<Country>,
}

/// Pull every document from `name`, projecting only the fields we need.
async fn load_all(pool: &DatabasePool, name: &str, projection: Document) -> Result<Vec<Document>> {
    let col = pool.database().collection::<Document>(name);
    let cursor = col
        .find(doc! {})
        .projection(projection)
        .await
        .with_context(|| format!("querying collection `{}`", name))?;
    let docs: Vec<Document> = cursor
        .try_collect()
        .await
        .with_context(|| format!("reading collection `{}`", name))?;
    Ok(docs)
}

/// Load a collection of `{ <field>: "..." }` into a flat string vec.
async fn load_strings(pool: &DatabasePool, name: &str, field: &str) -> Result<Vec<String>> {
    let docs = load_all(pool, name, doc! { field: 1, "_id": 0 }).await?;
    Ok(docs
        .into_iter()
        .filter_map(|d| d.get_str(field).ok().map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect())
}

impl Snapshot {
    /// An empty snapshot. Every picker returns None, so callers fall back to
    /// their hardcoded defaults — which is what the routing tests exercise.
    #[cfg(test)]
    pub fn empty() -> Self {
        Snapshot {
            first_names: Vec::new(),
            last_names: Vec::new(),
            words: Vec::new(),
            domains: Vec::new(),
            user_agents: Vec::new(),
            comments: Vec::new(),
            timezones: Vec::new(),
            phone_prefixes: Vec::new(),
            phones_by_city: HashMap::new(),
            phones_by_state: HashMap::new(),
            phones_by_country: HashMap::new(),
            addresses: Vec::new(),
            addresses_by_country: HashMap::new(),
            countries_by_code_state: HashMap::new(),
            countries_by_code: HashMap::new(),
            countries: Vec::new(),
        }
    }

    pub async fn load(pool: &DatabasePool) -> Result<Self> {
        // Collections are independent, so fetch them concurrently.
        let (
            first_names,
            last_names,
            words,
            domains,
            user_agents,
            comments,
            tz_docs,
            phone_docs,
            address_docs,
            country_docs,
        ) = tokio::try_join!(
            load_strings(pool, "first_name", "name"),
            load_strings(pool, "last_name", "name"),
            load_strings(pool, "words", "word"),
            // domains store the value in _id
            async {
                let docs = load_all(pool, "domains", doc! { "_id": 1 }).await?;
                Ok(docs
                    .into_iter()
                    .filter_map(|d| d.get_str("_id").ok().map(str::to_string))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>())
            },
            load_strings(pool, "user_agents", "ua"),
            load_strings(pool, "comments", "comment"),
            load_all(pool, "timezone", doc! { "name": 1, "offset": 1, "_id": 0 }),
            load_all(
                pool,
                "phone_prefixes",
                doc! { "npa": 1, "nxx": 1, "city": 1, "state_code": 1, "country": 1, "_id": 0 },
            ),
            load_all(
                pool,
                "addresses",
                doc! { "address1": 1, "address2": 1, "city": 1, "state": 1, "postalCode": 1, "country": 1, "_id": 0 },
            ),
            load_all(
                pool,
                "countries",
                doc! { "country_id": 1, "country_code": 1, "country_name": 1, "state_id": 1, "state_code": 1, "state_name": 1, "_id": 0 },
            ),
        )?;

        let timezones = tz_docs
            .into_iter()
            .filter_map(|d| {
                Some(Timezone {
                    name: d.get_str("name").ok()?.to_string(),
                    offset: get_i32(&d, "offset").unwrap_or(-300),
                })
            })
            .collect();

        // ── phone prefixes + their filter indices ──
        let mut phone_prefixes = Vec::with_capacity(phone_docs.len());
        let mut phones_by_city: HashMap<String, Vec<u32>> = HashMap::new();
        let mut phones_by_state: HashMap<String, Vec<u32>> = HashMap::new();
        let mut phones_by_country: HashMap<String, Vec<u32>> = HashMap::new();

        for d in phone_docs {
            let (npa, nxx) = match (d.get_str("npa"), d.get_str("nxx")) {
                (Ok(a), Ok(b)) => (a.to_string(), b.to_string()),
                _ => continue,
            };
            let idx = phone_prefixes.len() as u32;
            if let Ok(city) = d.get_str("city") {
                if !city.is_empty() {
                    phones_by_city.entry(city.to_lowercase()).or_default().push(idx);
                }
            }
            if let Ok(state) = d.get_str("state_code") {
                if !state.is_empty() {
                    phones_by_state.entry(state.to_uppercase()).or_default().push(idx);
                }
            }
            if let Ok(country) = d.get_str("country") {
                if !country.is_empty() {
                    phones_by_country.entry(country.to_uppercase()).or_default().push(idx);
                }
            }
            phone_prefixes.push(PhonePrefix { npa, nxx });
        }

        // ── addresses + country index ──
        let mut addresses = Vec::with_capacity(address_docs.len());
        let mut addresses_by_country: HashMap<String, Vec<u32>> = HashMap::new();

        for d in address_docs {
            let addr = Address {
                address1: d.get_str("address1").unwrap_or_default().to_string(),
                address2: d.get_str("address2").ok().map(str::to_string),
                city: d.get_str("city").unwrap_or_default().to_string(),
                state: d.get_str("state").unwrap_or_default().to_string(),
                postal_code: d.get_str("postalCode").unwrap_or_default().to_string(),
                country: d.get_str("country").ok().map(str::to_string),
            };
            let idx = addresses.len() as u32;
            if let Some(c) = addr.country.as_deref().filter(|c| !c.is_empty()) {
                addresses_by_country.entry(c.to_uppercase()).or_default().push(idx);
            }
            addresses.push(addr);
        }

        // ── countries, keyed the same two ways the old queries looked them up ──
        let mut countries = Vec::with_capacity(country_docs.len());
        let mut countries_by_code_state: HashMap<(String, String), u32> = HashMap::new();
        let mut countries_by_code: HashMap<String, u32> = HashMap::new();

        for d in country_docs {
            let country_code = match d.get_str("country_code") {
                Ok(c) if !c.is_empty() => c.to_uppercase(),
                _ => continue,
            };
            let country = Country {
                country_id: get_i64(&d, "country_id"),
                country_code: d.get_str("country_code").unwrap_or_default().to_string(),
                country_name: d.get_str("country_name").unwrap_or_default().to_string(),
                state_id: get_i64(&d, "state_id"),
                state_name: d.get_str("state_name").ok().map(str::to_string),
            };
            let idx = countries.len() as u32;
            if let Ok(state_code) = d.get_str("state_code") {
                if !state_code.is_empty() {
                    countries_by_code_state
                        .entry((country_code.clone(), state_code.to_uppercase()))
                        .or_insert(idx);
                }
            }
            // First record for a country is the fallback, matching the old `$limit: 1`.
            countries_by_code.entry(country_code).or_insert(idx);
            countries.push(country);
        }

        let snap = Snapshot {
            first_names,
            last_names,
            words,
            domains,
            user_agents,
            comments,
            timezones,
            phone_prefixes,
            phones_by_city,
            phones_by_state,
            phones_by_country,
            addresses,
            addresses_by_country,
            countries_by_code_state,
            countries_by_code,
            countries,
        };

        info!(
            "cache loaded: {} first, {} last, {} words, {} domains, {} ua, {} comments, {} tz, {} phone, {} addr, {} country",
            snap.first_names.len(),
            snap.last_names.len(),
            snap.words.len(),
            snap.domains.len(),
            snap.user_agents.len(),
            snap.comments.len(),
            snap.timezones.len(),
            snap.phone_prefixes.len(),
            snap.addresses.len(),
            snap.countries.len(),
        );

        Ok(snap)
    }

    // ── random pickers ───────────────────────────────────────────────────────

    pub fn first_name(&self) -> Option<&str> {
        pick(&self.first_names).map(String::as_str)
    }

    pub fn last_name(&self) -> Option<&str> {
        pick(&self.last_names).map(String::as_str)
    }

    pub fn word(&self) -> Option<&str> {
        pick(&self.words).map(String::as_str)
    }

    pub fn domain(&self) -> Option<&str> {
        pick(&self.domains).map(String::as_str)
    }

    pub fn user_agent(&self) -> Option<&str> {
        pick(&self.user_agents).map(String::as_str)
    }

    pub fn comment(&self) -> Option<&str> {
        pick(&self.comments).map(String::as_str)
    }

    pub fn timezone(&self) -> Option<&Timezone> {
        pick(&self.timezones)
    }

    pub fn address_by_country(&self, country_code: &str) -> Option<&Address> {
        let idxs = self.addresses_by_country.get(&country_code.to_uppercase())?;
        pick(idxs).map(|&i| &self.addresses[i as usize])
    }

    /// City match is a case-insensitive *prefix*, as the old regex was. Exact
    /// hits use the hash index; only a miss falls back to a prefix scan.
    pub fn phone_by_city(&self, city: &str, state_code: Option<&str>) -> Option<&PhonePrefix> {
        let city_key = city.to_lowercase();
        let state_key = state_code
            .filter(|s| !s.is_empty())
            .map(|s| s.to_uppercase());

        let in_state = |i: u32| match &state_key {
            Some(s) => self
                .phones_by_state
                .get(s)
                .is_some_and(|v| v.binary_search(&i).is_ok()),
            None => true,
        };

        if let Some(idxs) = self.phones_by_city.get(&city_key) {
            let matches: Vec<u32> = idxs.iter().copied().filter(|&i| in_state(i)).collect();
            if let Some(&i) = pick(&matches) {
                return Some(&self.phone_prefixes[i as usize]);
            }
        }

        // Prefix fallback — the old query was `^city`, not an exact match.
        let matches: Vec<u32> = self
            .phones_by_city
            .iter()
            .filter(|(k, _)| k.starts_with(&city_key))
            .flat_map(|(_, v)| v.iter().copied())
            .filter(|&i| in_state(i))
            .collect();
        pick(&matches).map(|&i| &self.phone_prefixes[i as usize])
    }

    pub fn phone_by_state(&self, state_code: &str) -> Option<&PhonePrefix> {
        let idxs = self.phones_by_state.get(&state_code.to_uppercase())?;
        pick(idxs).map(|&i| &self.phone_prefixes[i as usize])
    }

    pub fn phone_by_country(&self, country: &str) -> Option<&PhonePrefix> {
        let idxs = self.phones_by_country.get(&country.to_uppercase())?;
        pick(idxs).map(|&i| &self.phone_prefixes[i as usize])
    }

    pub fn phone_any(&self) -> Option<&PhonePrefix> {
        pick(&self.phone_prefixes)
    }

    /// Country+state exact match, falling back to any record for the country.
    pub fn country_by_code_and_state(&self, country_code: &str, state_code: &str) -> Option<&Country> {
        let cc = country_code.to_uppercase();
        if !state_code.is_empty() {
            if let Some(&i) = self
                .countries_by_code_state
                .get(&(cc.clone(), state_code.to_uppercase()))
            {
                return Some(&self.countries[i as usize]);
            }
        }
        self.countries_by_code.get(&cc).map(|&i| &self.countries[i as usize])
    }
}

/// Uniform random element, or None if empty.
fn pick<T>(v: &[T]) -> Option<&T> {
    if v.is_empty() {
        return None;
    }
    Some(&v[rand::thread_rng().gen_range(0..v.len())])
}

fn get_i32(d: &Document, key: &str) -> Option<i32> {
    d.get_i32(key).ok().or_else(|| d.get_i64(key).ok().map(|v| v as i32))
}

fn get_i64(d: &Document, key: &str) -> Option<i64> {
    d.get_i64(key).ok().or_else(|| d.get_i32(key).ok().map(i64::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        let phone_prefixes = vec![
            PhonePrefix { npa: "403".into(), nxx: "201".into() }, // 0 Calgary/AB/CA
            PhonePrefix { npa: "213".into(), nxx: "555".into() }, // 1 Los Angeles/CA/US
            PhonePrefix { npa: "212".into(), nxx: "555".into() }, // 2 New York/NY/US
        ];
        let mut phones_by_city = HashMap::new();
        phones_by_city.insert("calgary".to_string(), vec![0u32]);
        phones_by_city.insert("los angeles".to_string(), vec![1u32]);
        phones_by_city.insert("new york".to_string(), vec![2u32]);

        let mut phones_by_state = HashMap::new();
        phones_by_state.insert("AB".to_string(), vec![0u32]);
        phones_by_state.insert("CA".to_string(), vec![1u32]);
        phones_by_state.insert("NY".to_string(), vec![2u32]);

        let mut phones_by_country = HashMap::new();
        phones_by_country.insert("CA".to_string(), vec![0u32]);
        phones_by_country.insert("US".to_string(), vec![1u32, 2u32]);

        let addresses = vec![
            Address {
                address1: "1 Main St".into(),
                address2: None,
                city: "Los Angeles".into(),
                state: "CA".into(),
                postal_code: "90001".into(),
                country: Some("US".into()),
            },
            Address {
                address1: "2 King St".into(),
                address2: None,
                city: "Calgary".into(),
                state: "AB".into(),
                postal_code: "T2P".into(),
                country: Some("CA".into()),
            },
        ];
        let mut addresses_by_country = HashMap::new();
        addresses_by_country.insert("US".to_string(), vec![0u32]);
        addresses_by_country.insert("CA".to_string(), vec![1u32]);

        let countries = vec![
            Country {
                country_id: Some(1),
                country_code: "US".into(),
                country_name: "United States".into(),
                state_id: Some(5),
                state_name: Some("California".into()),
            },
            Country {
                country_id: Some(1),
                country_code: "US".into(),
                country_name: "United States".into(),
                state_id: Some(33),
                state_name: Some("New York".into()),
            },
        ];
        let mut countries_by_code_state = HashMap::new();
        countries_by_code_state.insert(("US".to_string(), "CA".to_string()), 0u32);
        countries_by_code_state.insert(("US".to_string(), "NY".to_string()), 1u32);
        let mut countries_by_code = HashMap::new();
        countries_by_code.insert("US".to_string(), 0u32);

        Snapshot {
            first_names: vec!["John".into()],
            last_names: vec!["Smith".into()],
            words: vec!["cool".into()],
            domains: vec!["gmail.com".into()],
            user_agents: vec!["Mozilla/5.0".into()],
            comments: vec!["hi".into()],
            timezones: vec![Timezone { name: "America/New_York".into(), offset: -300 }],
            phone_prefixes,
            phones_by_city,
            phones_by_state,
            phones_by_country,
            addresses,
            addresses_by_country,
            countries_by_code_state,
            countries_by_code,
            countries,
        }
    }

    #[test]
    fn pick_is_none_on_empty_and_some_otherwise() {
        let empty: Vec<u8> = vec![];
        assert!(pick(&empty).is_none());
        assert_eq!(pick(&[7u8]), Some(&7));
    }

    #[test]
    fn address_filter_never_crosses_countries() {
        // The old query returned None rather than another country's address.
        for _ in 0..50 {
            assert_eq!(snap().address_by_country("us").unwrap().state, "CA");
            assert_eq!(snap().address_by_country("CA").unwrap().state, "AB");
        }
        assert!(snap().address_by_country("ZZ").is_none());
    }

    #[test]
    fn phone_city_lookup_is_case_insensitive_and_state_scoped() {
        let s = snap();
        assert_eq!(s.phone_by_city("los angeles", None).unwrap().npa, "213");
        assert_eq!(s.phone_by_city("LOS ANGELES", Some("CA")).unwrap().npa, "213");
        // City in a different state must not match.
        assert!(s.phone_by_city("Los Angeles", Some("NY")).is_none());
    }

    #[test]
    fn phone_city_matches_on_prefix() {
        // Old behaviour was a `^city` regex, so a prefix must still hit.
        assert_eq!(snap().phone_by_city("calg", None).unwrap().npa, "403");
        assert!(snap().phone_by_city("nowhere", None).is_none());
    }

    #[test]
    fn phone_state_and_country_filters() {
        let s = snap();
        assert_eq!(s.phone_by_state("ab").unwrap().npa, "403");
        assert_eq!(s.phone_by_country("ca").unwrap().npa, "403");
        for _ in 0..50 {
            // US has two prefixes; both are valid, neither is Canadian.
            assert_ne!(s.phone_by_country("US").unwrap().npa, "403");
        }
        assert!(s.phone_by_state("ZZ").is_none());
    }

    #[test]
    fn country_state_match_then_fallback() {
        let s = snap();
        assert_eq!(s.country_by_code_and_state("US", "NY").unwrap().state_id, Some(33));
        // Unknown state falls back to the first US record, as `$limit: 1` did.
        assert_eq!(s.country_by_code_and_state("US", "ZZ").unwrap().state_id, Some(5));
        assert!(s.country_by_code_and_state("ZZ", "").is_none());
    }
}
