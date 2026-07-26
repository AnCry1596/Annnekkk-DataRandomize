use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Deserialize;
use crate::db::DatabasePool;
use anyhow::Result;


/// A document from the `first_name` or `last_name` collection: { _id: ObjectId, name: "..." }
#[derive(Debug, Deserialize)]
pub struct NameDocument {
    pub name: String,
}

impl NameDocument {
    pub async fn random_first(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<NameDocument>("first_name");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let name: NameDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(name.name));
        }
        Ok(None)
    }

    pub async fn random_last(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<NameDocument>("last_name");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let name: NameDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(name.name));
        }
        Ok(None)
    }
}

/// A document from the `words` collection: { _id: ObjectId, word: "..." }
#[derive(Debug, Deserialize)]
pub struct WordDocument {
    pub word: String,
}

impl WordDocument {
    pub async fn random(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<WordDocument>("words");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let w: WordDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(w.word));
        }
        Ok(None)
    }
}

/// A document from the `domains` collection: { _id: "gmail.com" }
#[derive(Debug, Deserialize)]
pub struct DomainDocument {
    #[serde(rename = "_id")]
    pub domain: String,
}

impl DomainDocument {
    pub async fn random(pool: &DatabasePool) -> Result<Option<String>> {
        let col = pool.database().collection::<DomainDocument>("domains");
        let pipeline = vec![doc! { "$sample": { "size": 1 } }];
        let mut cursor = col.aggregate(pipeline).await?;
        if let Some(doc) = cursor.try_next().await? {
            let d: DomainDocument = mongodb::bson::from_document(doc)?;
            return Ok(Some(d.domain));
        }
        Ok(None)
    }
}

