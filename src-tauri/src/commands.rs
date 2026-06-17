use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;

use crate::crawler::{CrawlConfig, Crawler, CrawlControl};

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

pub struct CrawlState {
    control: std::sync::Mutex<Arc<CrawlControl>>,
    task_handle: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
}

impl CrawlState {
    pub fn new() -> Self {
        Self {
            control: std::sync::Mutex::new(Arc::new(CrawlControl::new())),
            task_handle: std::sync::Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn start_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlState>,
) -> Result<(), String> {
    {
        let mut handle = state.task_handle.lock().unwrap();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock().unwrap();
        *ctrl = new_control.clone();
    }

    let emitter = TauriEmitter::new(app);
    let control = new_control;

    let handle = tokio::spawn(async move {
        let crawler = Crawler::new();
        let _ = crawler.run(config, emitter, &control).await;
    });

    {
        let mut h = state.task_handle.lock().unwrap();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

#[tauri::command]
pub fn cancel_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock().unwrap();
    ctrl.cancel();
    Ok(())
}

#[tauri::command]
pub fn pause_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock().unwrap();
    ctrl.pause();
    Ok(())
}

#[tauri::command]
pub fn resume_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock().unwrap();
    ctrl.resume();
    Ok(())
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
