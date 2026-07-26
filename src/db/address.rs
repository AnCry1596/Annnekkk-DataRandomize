use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;

/// A document from the `addresses` collection
#[derive(Debug, Deserialize)]
pub struct AddressDocument {
    pub address1: String,
    pub address2: Option<String>,
    pub city: String,
    pub state: String,
    #[serde(rename = "postalCode")]
    pub postal_code: String,
    pub country: Option<String>,
}

impl AddressDocument {
    /// Returns a random address filtered by country code (e.g. "US").
    /// Returns None if no address exists for that country — does NOT fall back to a different country.
    pub async fn random_by_country(pool: &DatabasePool, country_code: &str) -> Result<Option<Self>> {
        let col = pool.database().collection::<AddressDocument>("addresses");
        let pipeline = vec![
            doc! { "$match": { "country": country_code.to_uppercase() } },
            doc! { "$sample": { "size": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let a: AddressDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(a));
        }
        Ok(None)
    }
}
