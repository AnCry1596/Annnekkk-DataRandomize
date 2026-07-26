use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;

#[derive(Debug, Deserialize)]
pub struct CommentDocument {
    pub comment: String,
}

impl CommentDocument {
    pub async fn random(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<CommentDocument>("comments");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let c: CommentDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(c.comment));
        }
        Ok(None)
    }
}
