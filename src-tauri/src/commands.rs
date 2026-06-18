use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::crawler::{CrawlConfig, Crawler, CrawlControl};
use crate::runtime::AppContext;
use crate::store::{self, AppStatus};
use crate::zyte::{ZyteClient, ZyteJobRequest, ZyteJobArguments, ZyteProgress};

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
    control: parking_lot::Mutex<Arc<CrawlControl>>,
    task_handle: parking_lot::Mutex<Option<tokio::task::AbortHandle>>,
}

impl CrawlState {
    pub fn new() -> Self {
        Self {
            control: parking_lot::Mutex::new(Arc::new(CrawlControl::new())),
            task_handle: parking_lot::Mutex::new(None),
        }
    }
}

pub async fn persist_items(ctx: &Arc<AppContext>, items: Vec<serde_json::Value>) {
    if items.is_empty() {
        return;
    }
    if let Some(store) = &ctx.store {
        let crawl_id = format!("crawl_{}", chrono::Utc::now().timestamp_millis());
        if let Err(e) = store::persist_items(store, items, &crawl_id).await {
            eprintln!("persist_items error: {}", e);
        }
    }
}

#[tauri::command]
pub async fn get_app_status(
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<AppStatus, String> {
    Ok(AppStatus {
        mongo_ok: ctx.mongo_ok(),
        zyte_available: ctx.zyte_available(),
        zyte_project: ctx.zyte_project.clone(),
    })
}

#[tauri::command]
pub async fn start_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlState>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<(), String> {
    {
        let mut handle = state.task_handle.lock();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let emitter = TauriEmitter::new(app);
    let control = new_control;

    control.reset();

    let _ = ctx.inner().clone();

    let handle = tokio::spawn(async move {
        let crawler = Crawler::new();
        let _ = crawler.run(config, emitter, &control).await;
    });

    {
        let mut h = state.task_handle.lock();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

#[tauri::command]
pub async fn start_cloud_crawl(
    app: AppHandle,
    config: CrawlConfig,
    api_key: String,
    project_id: String,
    state: tauri::State<'_, CrawlState>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<(), String> {
    if ctx.zyte.is_none() && api_key.is_empty() {
        return Err("Zyte engine not configured: set ZYTE_API_KEY and CRASP_ZYTE_PROJECT".to_string());
    }

    let effective_api_key = if api_key.is_empty() {
        ctx.zyte
            .as_ref()
            .map(|c| c.api_key().to_string())
            .unwrap_or_default()
    } else {
        api_key
    };

    let effective_project_id = if project_id.is_empty() {
        ctx.zyte_project.clone().unwrap_or_default()
    } else {
        project_id
    };

    {
        let mut handle = state.task_handle.lock();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let emitter = TauriEmitter::new(app);
    let ctx_arc = ctx.inner().clone();

    let handle = tokio::spawn(async move {
        let _control = new_control;
        let client = ZyteClient::new(effective_api_key);

        let job_req = ZyteJobRequest {
            project: effective_project_id,
            spider: "crasp_archive".to_string(),
            add_arguments: ZyteJobArguments {
                seed_url: config.seed_url.clone(),
                max_depth: config.max_depth,
                max_pages: config.max_pages,
                css_selectors: config.css_selectors.join(","),
                preserve_html: config.preserve_html.to_string(),
                hash_algorithm: match config.hash_algorithm {
                    crate::crawler::HashAlgorithm::Md5 => "md5".to_string(),
                    crate::crawler::HashAlgorithm::Sha256 => "sha256".to_string(),
                },
            },
        };

        let job_key = match client.run_job(&job_req).await {
            Ok(k) => k,
            Err(e) => {
                let _ = emitter.emit("crawl-done", &serde_json::json!({
                    "pages_archived": 0,
                    "cancelled": false,
                    "error": e,
                })).await;
                return;
            }
        };

        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<ZyteProgress>(100);
        let (items_tx, mut items_rx) = tokio::sync::mpsc::channel::<serde_json::Value>(1000);

        let wait_client = client.clone();
        let wait_job_key = job_key.clone();
        let wait_progress_tx = progress_tx.clone();

        let wait_task = tokio::spawn(async move {
            wait_client.wait_for_job(&wait_job_key, wait_progress_tx).await
        });

        let fetch_client = client.clone();
        let fetch_job_key = job_key.clone();
        let fetch_items_tx = items_tx.clone();

        let fetch_task = tokio::spawn(async move {
            fetch_client.fetch_items(&fetch_job_key, 100, fetch_items_tx).await
        });

        drop(progress_tx);
        drop(items_tx);

        let emitter_progress = emitter.clone();
        let progress_emitter = tokio::spawn(async move {
            while let Some(prog) = progress_rx.recv().await {
                let _ = emitter_progress.emit("cloud-progress", &serde_json::json!({
                    "job_key": prog.job_key,
                    "state": prog.state,
                    "items_scraped": prog.items_scraped,
                })).await;
            }
        });

        let emitter_items = emitter.clone();
        let ctx_items = ctx_arc.clone();
        let items_emitter = tokio::spawn(async move {
            let mut count = 0u64;
            while let Some(item) = items_rx.recv().await {
                count += 1;
                let _ = emitter_items.emit("archive-success", &item).await;
                persist_items(&ctx_items, vec![item]).await;
            }
            count
        });

        let _ = wait_task.await;
        let _ = fetch_task.await;

        let _ = progress_emitter.await;
        let count = items_emitter.await.unwrap_or(0);

        let _ = emitter.emit("crawl-done", &serde_json::json!({
            "pages_archived": count,
            "cancelled": false,
        })).await;
    });

    {
        let mut h = state.task_handle.lock();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

#[tauri::command]
pub async fn local_scrapy_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlState>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<(), String> {
    {
        let mut handle = state.task_handle.lock();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let control = new_control;
    let emitter = TauriEmitter::new(app.clone());
    let ctx_arc = ctx.inner().clone();

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let out_path = format!("{}/crasp_{}.jl", app_data_dir, chrono::Utc::now().timestamp());

    let handle = tokio::spawn(async move {
        crate::local_scrapy::run_local_spider_streaming(
            &config,
            &out_path,
            &emitter,
            &control,
            &ctx_arc,
        )
        .await
    });

    {
        let mut h = state.task_handle.lock();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

#[tauri::command]
pub fn cancel_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock();
    ctrl.cancel();
    Ok(())
}

#[tauri::command]
pub fn pause_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock();
    ctrl.pause();
    Ok(())
}

#[tauri::command]
pub fn resume_crawl(state: tauri::State<'_, CrawlState>) -> Result<(), String> {
    let ctrl = state.control.lock();
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
