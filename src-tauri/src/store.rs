use mongodb::bson::doc;
use mongodb::IndexModel;

use crate::schema::{CrawlDoc, HashDoc, PageDoc};

#[derive(Clone)]
pub struct ArchiveStore {
    database: mongodb::Database,
}

impl ArchiveStore {
    pub async fn from_uri(uri: &str) -> Result<Self, String> {
        let client = mongodb::Client::with_uri_str(uri)
            .await
            .map_err(|e| format!("MongoDB connection failed: {}", e))?;

        let database = client.database("crasp");

        Ok(Self { database })
    }

    pub fn pages_col(&self) -> mongodb::Collection<PageDoc> {
        self.database.collection("pages")
    }

    pub fn crawls_col(&self) -> mongodb::Collection<CrawlDoc> {
        self.database.collection("crawls")
    }

    pub fn hashes_col(&self) -> mongodb::Collection<HashDoc> {
        self.database.collection("content_hashes")
    }

    pub async fn ensure_indexes(&self) -> Result<(), String> {
        let pages = self.database.collection::<mongodb::bson::Document>("pages");
        let crawls = self.database.collection::<mongodb::bson::Document>("crawls");
        let hashes = self.database.collection::<mongodb::bson::Document>("content_hashes");

        pages
            .create_indexes(vec![
                IndexModel::builder()
                    .keys(doc! { "crawl_id": 1, "depth": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "url_normalized": 1 })
                    .options(mongodb::options::IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "crawl_id": 1, "status": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "duplicate_group_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "timestamp": -1 })
                    .build(),
            ])
            .await
            .map_err(|e| format!("Failed to create pages indexes: {}", e))?;

        crawls
            .create_indexes(vec![
                IndexModel::builder()
                    .keys(doc! { "crawl_id": 1 })
                    .options(mongodb::options::IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "started_at": -1 })
                    .build(),
            ])
            .await
            .map_err(|e| format!("Failed to create crawls indexes: {}", e))?;

        hashes
            .create_indexes(vec![
                IndexModel::builder()
                    .keys(doc! { "hash": 1, "hash_algorithm": 1 })
                    .options(mongodb::options::IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "first_seen_crawl_id": 1 })
                    .build(),
            ])
            .await
            .map_err(|e| format!("Failed to create content_hashes indexes: {}", e))?;

        Ok(())
    }
}

pub async fn persist_batch(
    store: &ArchiveStore,
    pages: Vec<PageDoc>,
) -> Result<(), String> {
    use mongodb::bson::to_document;

    let chunk_size = 50;
    for chunk in pages.chunks(chunk_size) {
        let docs: Vec<mongodb::bson::Document> = chunk
            .iter()
            .filter_map(|p| to_document(p).ok())
            .collect();

        if docs.is_empty() {
            continue;
        }

        store
            .database
            .collection::<mongodb::bson::Document>("pages")
            .insert_many(docs)
            .await
            .map_err(|e| format!("Batch insert failed: {}", e))?;
    }

    Ok(())
}
