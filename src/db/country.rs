use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;

const COLLECTION: &str = "countries";

/// A document from the `countries` collection.
/// Fields state_id/state_code/state_name may be null for non-US countries.
#[derive(Debug, Deserialize)]
pub struct CountryDocument {
    pub country_id: Option<i64>,
    pub country_code: String,
    pub country_name: String,
    pub state_id: Option<i64>,
    pub state_name: Option<String>,
}

impl CountryDocument {
    /// Return a country record matching `country_code` AND `state_code`.
    /// Falls back to any record matching just `country_code` if the state is not found.
    pub async fn by_code_and_state(
        pool: &DatabasePool,
        country_code: &str,
        state_code: &str,
    ) -> Result<Option<Self>> {
        let col = pool.database().collection::<CountryDocument>(COLLECTION);

        // Try exact country + state match first
        let pipeline = vec![
            doc! { "$match": { "country_code": country_code.to_uppercase(), "state_code": state_code.to_uppercase() } },
            doc! { "$limit": 1 },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let c: CountryDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(c));
        }

        // Fallback: any record for this country (e.g. non-US countries with null state)
        let pipeline = vec![
            doc! { "$match": { "country_code": country_code.to_uppercase() } },
            doc! { "$limit": 1 },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let c: CountryDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(c));
        }

        Ok(None)
    }
}
