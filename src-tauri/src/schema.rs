use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlDoc {
    pub crawl_id: String,
    pub seed_url: String,
    pub config: CrawlConfigEmbedded,
    pub source: String,
    pub zyte_job_key: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub stats: CrawlStatsEmbedded,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlConfigEmbedded {
    pub max_depth: u32,
    pub max_pages: u32,
    pub concurrency: usize,
    pub css_selectors: Vec<String>,
    pub preserve_html: bool,
    pub hash_algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStatsEmbedded {
    pub discovered: u32,
    pub archived: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDoc {
    pub crawl_id: String,
    pub url: String,
    pub url_normalized: String,
    pub depth: u32,
    pub title: String,
    pub status: String,
    pub status_code: i32,
    pub content: String,
    pub content_format: String,
    pub content_bytes: Option<mongodb::bson::Binary>,
    pub discovered_links: u32,
    pub timestamp: String,
    pub duplicate_group_id: i32,
    pub search_blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashDoc {
    pub hash: String,
    pub hash_algorithm: String,
    pub first_seen_crawl_id: String,
    pub first_seen_url: String,
    pub first_seen_at: String,
    pub occurrences: u32,
}
