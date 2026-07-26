use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;

/// A document from the `phone_prefixes` collection:
/// { npa: "403", nxx: "201", city: "Calgary", state_code: "AB", country: "CA" }
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PhonePrefixDocument {
    pub npa: String,
    pub nxx: String,
    pub city: Option<String>,
    pub state_code: Option<String>,
    pub country: Option<String>,
}

impl PhonePrefixDocument {
    /// Random NPA+NXX from the entire collection.
    pub async fn random(pool: &DatabasePool) -> Result<Option<Self>> {
        let col = pool.database().collection::<PhonePrefixDocument>("phone_prefixes");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let p: PhonePrefixDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(p));
        }
        Ok(None)
    }

    /// Random NPA+NXX filtered by city + state (case-insensitive city prefix match).
    /// If state is provided it must also match; otherwise city alone is used.
    pub async fn random_by_city(pool: &DatabasePool, city: &str, state_code: Option<&str>) -> Result<Option<Self>> {
        let col = pool.database().collection::<PhonePrefixDocument>("phone_prefixes");
        let mut match_doc = doc! {
            "city": { "$regex": format!("^{}", regex_escape(city)), "$options": "i" }
        };
        if let Some(s) = state_code.filter(|s| !s.is_empty()) {
            match_doc.insert("state_code", s.to_uppercase());
        }
        let pipeline = vec![
            doc! { "$match": match_doc },
            doc! { "$sample": { "size": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let p: PhonePrefixDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(p));
        }
        Ok(None)
    }

    /// Random NPA+NXX filtered by state code (e.g. `"AB"`, `"TX"`).
    pub async fn random_by_state(pool: &DatabasePool, state_code: &str) -> Result<Option<Self>> {
        let col = pool.database().collection::<PhonePrefixDocument>("phone_prefixes");
        let pipeline = vec![
            doc! { "$match": { "state_code": state_code.to_uppercase() } },
            doc! { "$sample": { "size": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let p: PhonePrefixDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(p));
        }
        Ok(None)
    }

    /// Random NPA+NXX filtered by country (e.g. `"US"`, `"CA"`).
    pub async fn random_by_country(pool: &DatabasePool, country: &str) -> Result<Option<Self>> {
        let col = pool.database().collection::<PhonePrefixDocument>("phone_prefixes");
        let pipeline = vec![
            doc! { "$match": { "country": country.to_uppercase() } },
            doc! { "$sample": { "size": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let p: PhonePrefixDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(p));
        }
        Ok(None)
    }
}

/// Escape special regex characters in a city name.
fn regex_escape(s: &str) -> String {
    s.chars().flat_map(|c| {
        if "^$.*+?()[]{}|\\".contains(c) {
            vec!['\\', c]
        } else {
            vec![c]
        }
    }).collect()
}
