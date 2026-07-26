use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use crate::db::DatabasePool;
use anyhow::Result;

/// Raw BIN data from MongoDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinDocument {
    #[serde(rename = "_id")]
    pub id: i64,
    #[serde(rename = "cardType")]
    pub card_type: Option<String>,
    #[serde(rename = "cardSubType")]
    pub sub_type: Option<String>,
    #[serde(rename = "cardCategory")]
    pub category: Option<String>,
    #[serde(rename = "cardRegulated")]
    pub regulated: Option<String>,
    #[serde(rename = "binCategory")]
    pub bin_category: Option<String>,
    #[serde(rename = "binLength")]
    pub bin_length: Option<String>,
    #[serde(rename = "issuingBank")]
    pub bank: Option<String>,
    #[serde(rename = "issuingCountryCode")]
    pub country_code: Option<String>,
    pub country: Option<String>,
}

impl BinDocument {
    /// Find a BIN document by its _id
    pub async fn find_by_id(pool: &DatabasePool, bin_id: i64) -> Result<Option<Self>> {
        let collection = pool.database().collection::<BinDocument>("bin_data");
        let filter = doc! { "_id": bin_id };
        let result = collection.find_one(filter).await?;
        Ok(result)
    }
}
