use mongodb::bson::doc;
use mongodb::IndexModel;

use crate::schema::{CrawlDoc, HashDoc, PageDoc};

#[derive(Clone)]
pub struct ArchiveStore {
    database: mongodb::Database,
}

#[allow(dead_code)]
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
                    .keys(doc! { "crawl_id": 1, "url_normalized": 1 })
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

        // Unordered insert: individual duplicate-key errors are tolerated
        // so that re-crawling a previously-archived domain under a new
        // crawl_id does not reject the entire batch.
        let options = mongodb::options::InsertManyOptions::builder()
            .ordered(false)
            .build();

        let result = store
            .database
            .collection::<mongodb::bson::Document>("pages")
            .insert_many(docs)
            .with_options(options)
            .await;

        match result {
            Ok(_) => {}
            Err(e) => {
                // E11000 duplicate key is expected when a URL was archived
                // in a previous crawl; surface only genuine errors.
                if !e.to_string().contains("E11000") {
                    eprintln!("Warning: batch insert error: {}", e);
                }
            }
        }
    }

    Ok(())
}

pub async fn upsert_hash(store: &ArchiveStore, hash_doc: HashDoc) -> Result<(), String> {
    use mongodb::bson::to_document;

    let doc = to_document(&hash_doc).map_err(|e| format!("BSON encode failed: {}", e))?;

    let filter = doc! {
        "hash": &hash_doc.hash,
        "hash_algorithm": &hash_doc.hash_algorithm,
    };

    let update = doc! {
        "$setOnInsert": doc,
        "$inc": { "occurrences": 1 },
    };

    let options = mongodb::options::UpdateOptions::builder()
        .upsert(true)
        .build();

    store
        .hashes_col()
        .update_one(filter, update)
        .with_options(options)
        .await
        .map_err(|e| format!("Hash upsert failed: {}", e))?;

    Ok(())
}

pub async fn persist_items(
    store: &ArchiveStore,
    items: Vec<serde_json::Value>,
    crawl_id: &str,
) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }

    let mut pages = Vec::with_capacity(items.len());
    let mut hashes = Vec::new();

    for item in &items {
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let depth = item
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // ── Structural status handling (WI-29) ──────────────────────────
        // serde serializes PageStatus as:
        //   unit variants: "Completed" | "Pending" | ...
        //   tuple variants: {"Failed":"reason"} | {"Skipped":"reason"}
        // We preserve the discriminant in `status` and the reason in
        // `status_reason`, never silently defaulting to "Completed".
        let (status, status_reason) = match item.get("status") {
            Some(serde_json::Value::String(s)) => {
                (s.clone(), None)
            }
            Some(serde_json::Value::Object(map)) => {
                if let Some(v) = map.get("Failed").and_then(|v| v.as_str()) {
                    ("Failed".to_string(), Some(v.to_string()))
                } else if let Some(v) = map.get("Skipped").and_then(|v| v.as_str()) {
                    ("Skipped".to_string(), Some(v.to_string()))
                } else {
                    eprintln!("Warning: unknown status object shape for URL {}: {:?}", url, map);
                    ("Unknown".to_string(), None)
                }
            }
            Some(other) => {
                eprintln!("Warning: unexpected status type for URL {}: {:?}", url, other);
                ("Unknown".to_string(), None)
            }
            None => {
                // Derive from status_code when no explicit status is present.
                let code = item
                    .get("status_code")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as i32;
                if (200..300).contains(&code) {
                    ("Completed".to_string(), None)
                } else {
                    let reason = format!("HTTP {}", code);
                    ("Failed".to_string(), Some(reason))
                }
            }
        };

        let status_code = item
            .get("status_code")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as i32;
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content_format = if content.starts_with('<') {
            "html"
        } else {
            "text"
        };
        let hash_val = item.get("hash").and_then(|v| v.as_str()).map(String::from);
        let hash_algorithm = item
            .get("hash_algorithm")
            .and_then(|v| v.as_str())
            .map(String::from);
        let discovered_links = item
            .get("discovered_links")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let timestamp = item
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let url_normalized = url.to_lowercase();
        let search_blob = format!("{} {}", title, url);

        let page = PageDoc {
            crawl_id: crawl_id.to_string(),
            url: url.clone(),
            url_normalized,
            depth,
            title,
            status,
            status_code,
            status_reason,
            content,
            content_format: content_format.to_string(),
            content_bytes: None,
            discovered_links,
            timestamp: if timestamp.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                timestamp
            },
            duplicate_group_id: 0,
            search_blob,
        };
        pages.push(page);

        if let (Some(hash), Some(algo)) = (&hash_val, &hash_algorithm) {
            hashes.push(HashDoc {
                hash: hash.clone(),
                hash_algorithm: algo.clone(),
                first_seen_crawl_id: crawl_id.to_string(),
                first_seen_url: url.clone(),
                first_seen_at: chrono::Utc::now().to_rfc3339(),
                occurrences: 1,
            });
        }
    }

    if !pages.is_empty() {
        if let Err(e) = persist_batch(store, pages).await {
            eprintln!("Warning: persist_batch failed: {}", e);
        }
    }

    for hash_doc in hashes {
        if let Err(e) = upsert_hash(store, hash_doc).await {
            eprintln!("Warning: upsert_hash failed: {}", e);
        }
    }

    Ok(())
}
