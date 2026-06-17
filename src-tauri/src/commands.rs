use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::AppHandle;
use tauri::Emitter;

use crate::crawler::{ArchivedPage, CrawlConfig, Crawler, HashAlgorithm};

#[derive(Clone)]
pub struct TauriEmitter {
    app: AppHandle,
}

impl TauriEmitter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub async fn emit(&self, event: &str, payload: &serde_json::Value) -> Result<(), String> {
        self.app
            .emit(event, payload)
            .map_err(|e| e.to_string())
    }
}

pub struct CrawlerHandle {
    crawler: Arc<Mutex<Option<Crawler>>>,
}

impl CrawlerHandle {
    pub fn new() -> Self {
        Self {
            crawler: Arc::new(Mutex::new(None)),
        }
    }
}

#[tauri::command]
pub async fn start_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlerHandle>,
) -> Result<Vec<ArchivedPage>, String> {
    let crawler = Crawler::new();
    let emitter = TauriEmitter::new(app);

    {
        let mut guard = state.crawler.lock().await;
        *guard = Some(crawler);
    }

    let crawler_clone = state.crawler.clone();
    let result = {
        let guard = crawler_clone.lock().await;
        match guard.as_ref() {
            Some(c) => c.run(config, emitter).await,
            None => Err("no crawler instance".to_string()),
        }
    };

    let mut guard = state.crawler.lock().await;
    *guard = None;

    result
}

#[tauri::command]
pub async fn cancel_crawl(
    state: tauri::State<'_, CrawlerHandle>,
) -> Result<(), String> {
    let guard = state.crawler.lock().await;
    if let Some(crawler) = guard.as_ref() {
        crawler.cancel();
        Ok(())
    } else {
        Err("no active crawl".to_string())
    }
}

#[tauri::command]
pub async fn pause_crawl(
    state: tauri::State<'_, CrawlerHandle>,
) -> Result<(), String> {
    let guard = state.crawler.lock().await;
    if let Some(crawler) = guard.as_ref() {
        crawler.pause();
        Ok(())
    } else {
        Err("no active crawl".to_string())
    }
}

#[tauri::command]
pub async fn resume_crawl(
    state: tauri::State<'_, CrawlerHandle>,
) -> Result<(), String> {
    let guard = state.crawler.lock().await;
    if let Some(crawler) = guard.as_ref() {
        crawler.resume();
        Ok(())
    } else {
        Err("no active crawl".to_string())
    }
}

#[tauri::command]
pub fn validate_url(url: String) -> Result<String, String> {
    url::Url::parse(&url)
        .map(|_| url)
        .map_err(|e| format!("Invalid URL: {}", e))
}

#[tauri::command]
pub fn default_config() -> CrawlConfig {
    CrawlConfig::default()
}
