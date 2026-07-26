use mongodb::{Client, Database, options::ClientOptions};
use anyhow::Result;
use log::info;

#[derive(Clone)]
pub struct DatabasePool {
    pub db: Database,
}

impl DatabasePool {
    pub async fn new(uri: &str, db_name: &str) -> Result<Self> {
        info!("Connecting to MongoDB: {}", uri);

        let client_options = ClientOptions::parse(uri).await?;
        let client = Client::with_options(client_options)?;
        let db = client.database(db_name);

        // Verify connection
        db.list_collection_names().await?;

        info!("MongoDB connected successfully to database: {}", db_name);

        Ok(Self { db })
    }

    pub fn database(&self) -> &Database {
        &self.db
    }
}
