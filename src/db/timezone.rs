use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;

/// A document from the `timezone` collection: { name: "America/New_York", offset: -300 }
#[derive(Debug, Deserialize)]
pub struct TimezoneDocument {
    pub name: String,
    pub offset: i32,
}

impl TimezoneDocument {
    pub async fn random(pool: &DatabasePool) -> Result<Option<Self>> {
        let col = pool.database().collection::<TimezoneDocument>("timezone");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let t: TimezoneDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(t));
        }
        Ok(None)
    }
}