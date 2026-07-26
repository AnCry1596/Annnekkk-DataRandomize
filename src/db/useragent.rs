use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use crate::db::DatabasePool;
use anyhow::Result;

const COLLECTION: &str = "user_agents";

/// A document in the `user_agents` collection: { _id: ObjectId, ua: "Mozilla/5.0 ..." }
#[derive(Debug, Serialize, Deserialize)]
pub struct UserAgentDocument {
    pub ua: String,
}

impl UserAgentDocument {
    /// Return a random user-agent string.
    pub async fn random(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<UserAgentDocument>(COLLECTION);
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let ua: UserAgentDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(ua.ua));
        }
        Ok(None)
    }
}
