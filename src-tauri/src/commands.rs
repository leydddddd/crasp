use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::TryStreamExt;
use tauri::{AppHandle, Emitter, Manager};

use crate::progress::CraspEmitter;
use crate::crawler::{CrawlConfig, Crawler, CrawlControl, PageStage, PersistTarget};
use crate::logging::emit_log;
use crate::runtime::{AppContext, ServiceState, AppStatus as StoreAppStatus};
use crate::store;
use crate::zyte::{ZyteClient, ZyteJobRequest, ZyteJobArguments, ZyteProgress, ZyteConnectionStatus};

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
    crawl_id: parking_lot::Mutex<Option<String>>,
    outcomes: Arc<SharedCrawlOutcomes>,
}

#[derive(Debug, Default)]
pub struct SharedCrawlOutcomes {
    pub pages_completed: std::sync::atomic::AtomicU64,
    pub pages_failed: std::sync::atomic::AtomicU64,
    pub pages_skipped: std::sync::atomic::AtomicU64,
    pub used_mongo: std::sync::atomic::AtomicBool,
    pub local_file_path: parking_lot::Mutex<Option<String>>,
    pub deep_fetched_count: std::sync::atomic::AtomicU64,
}

impl CrawlState {
    pub fn new() -> Self {
        Self {
            control: parking_lot::Mutex::new(Arc::new(CrawlControl::new())),
            task_handle: parking_lot::Mutex::new(None),
            crawl_id: parking_lot::Mutex::new(None),
            outcomes: Arc::new(SharedCrawlOutcomes::default()),
        }
    }

    pub fn set_crawl_id(&self, id: String) {
        *self.crawl_id.lock() = Some(id);
        self.outcomes.reset();
    }

    #[allow(dead_code)]
    pub fn get_crawl_id(&self) -> Option<String> {
        self.crawl_id.lock().clone()
    }

    pub fn outcomes_arc(&self) -> Arc<SharedCrawlOutcomes> {
        self.outcomes.clone()
    }
}

impl SharedCrawlOutcomes {
    pub fn reset(&self) {
        self.pages_completed.store(0, std::sync::atomic::Ordering::SeqCst);
        self.pages_failed.store(0, std::sync::atomic::Ordering::SeqCst);
        self.pages_skipped.store(0, std::sync::atomic::Ordering::SeqCst);
        self.used_mongo.store(false, std::sync::atomic::Ordering::SeqCst);
        *self.local_file_path.lock() = None;
        self.deep_fetched_count.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn build_crawl_done_summary(&self, pages_archived: u64, cancelled: bool, crawl_id: &str) -> CrawlDoneSummary {
        let used_mongo = self.used_mongo.load(std::sync::atomic::Ordering::SeqCst);
        let local_path = self.local_file_path.lock().clone();
        let storage_used = match (used_mongo, local_path) {
            (true, Some(path)) => Some(StorageUsed::Both { local_path: path }),
            (true, None) => Some(StorageUsed::Mongo),
            (false, Some(path)) => Some(StorageUsed::LocalFile { path }),
            (false, None) => None,
        };
        CrawlDoneSummary {
            pages_archived,
            pages_completed: self.pages_completed.load(std::sync::atomic::Ordering::SeqCst),
            pages_failed: self.pages_failed.load(std::sync::atomic::Ordering::SeqCst),
            pages_skipped: self.pages_skipped.load(std::sync::atomic::Ordering::SeqCst),
            cancelled,
            crawl_id: crawl_id.to_string(),
            storage_used,
            deep_fetched_count: self.deep_fetched_count.load(std::sync::atomic::Ordering::SeqCst),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum PersistOutcome {
    Mongo { db: String, collection: String },
    LocalFile { path: String },
    Failed { reason: String },
}

pub fn local_fallback_path(app_data_dir: &str, crawl_id: &str) -> PathBuf {
    let dir = PathBuf::from(app_data_dir);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("crawl-{}.jl", crawl_id))
}

pub fn append_to_jl(path: &PathBuf, items: &[serde_json::Value]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open JL file {:?}: {}", path, e))?;
    for item in items {
        let line = serde_json::to_string(item)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;
        file.write_all(line.as_bytes())
            .map_err(|e| format!("Write failed: {}", e))?;
        file.write_all(b"\n")
            .map_err(|e| format!("Write newline failed: {}", e))?;
    }
    Ok(())
}

pub async fn persist_items_with_outcome(
    ctx: &Arc<AppContext>,
    items: Vec<serde_json::Value>,
    crawl_id: &str,
    app_data_dir: &str,
    fallback_active: &std::sync::atomic::AtomicBool,
) -> Vec<(serde_json::Value, PersistOutcome)> {
    if items.is_empty() {
        return Vec::new();
    }

    let jl_path = local_fallback_path(app_data_dir, crawl_id);

    let already_fallen_back = fallback_active.load(std::sync::atomic::Ordering::SeqCst);

    if already_fallen_back {
        let result = append_to_jl(&jl_path, &items);
        match result {
            Ok(_) => items
                .iter()
                .cloned()
                .map(|item| {
                    (
                        item,
                        PersistOutcome::LocalFile {
                            path: jl_path.to_string_lossy().to_string(),
                        },
                    )
                })
                .collect(),
            Err(e) => items
                .iter()
                .cloned()
                .map(|item| (item, PersistOutcome::Failed { reason: e.clone() }))
                .collect(),
        }
    } else if let Some(store) = &ctx.store {
        let result = store::persist_items(store, items.clone(), crawl_id).await;
        match result {
            Ok(_) => {
                ctx.set_mongo_state(ServiceState::Connected, Some("crasp".to_string()));
                items
                    .iter()
                    .cloned()
                    .map(|item| {
                        (
                            item,
                            PersistOutcome::Mongo {
                                db: "crasp".to_string(),
                                collection: "pages".to_string(),
                            },
                        )
                    })
                    .collect()
            }
            Err(e) => {
                let err_msg = e.clone();
                ctx.set_mongo_state(ServiceState::Unreachable, Some(e));
                fallback_active.store(true, std::sync::atomic::Ordering::SeqCst);
                eprintln!("Mongo persist failed, falling back to local file: {}", err_msg);
                let result = append_to_jl(&jl_path, &items);
                match result {
                    Ok(_) => items
                        .iter()
                        .cloned()
                        .map(|item| {
                            (
                                item,
                                PersistOutcome::LocalFile {
                                    path: jl_path.to_string_lossy().to_string(),
                                },
                            )
                        })
                        .collect(),
                    Err(e2) => items
                        .iter()
                        .cloned()
                        .map(|item| (item, PersistOutcome::Failed { reason: e2.clone() }))
                        .collect(),
                }
            }
        }
    } else {
        let result = append_to_jl(&jl_path, &items);
        match result {
            Ok(_) => items
                .iter()
                .cloned()
                .map(|item| {
                    (
                        item,
                        PersistOutcome::LocalFile {
                            path: jl_path.to_string_lossy().to_string(),
                        },
                    )
                })
                .collect(),
            Err(e) => items
                .iter()
                .cloned()
                .map(|item| (item, PersistOutcome::Failed { reason: e.clone() }))
                .collect(),
        }
    }
}

#[tauri::command]
pub async fn get_app_status(
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<StoreAppStatus, String> {
    Ok(ctx.to_app_status())
}

#[tauri::command]
pub async fn start_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlState>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<(), String> {
    if let Err(e) = crate::ssrf::validate_seed_url(&config.seed_url).await {
        let te = TauriEmitter::new(app);
        let emitter = CraspEmitter::Tauri(te);
        let _ = emit_log(&emitter, "error", "system", &format!("SSRF validation rejected seed URL {}: {}", config.seed_url, e)).await;
        return Err(e);
    }

    {
        let mut handle = state.task_handle.lock();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let crawl_id = format!("crawl_{}", chrono::Utc::now().timestamp_millis());
    state.set_crawl_id(crawl_id.clone());

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let emitter = CraspEmitter::Tauri(TauriEmitter::new(app));
    let control = new_control;
    let ctx_arc = ctx.inner().clone();

    control.reset();

    ctx.deep_fetch_queue.reset_counter();

    let _ = emit_log(&emitter, "info", "local", &format!("Crawl started: id={}, seed={}", crawl_id, config.seed_url));

    let shared = state.outcomes_arc();

    let handle = tokio::spawn(async move {
        let crawler = Crawler::new();
        let result = crawler.run(config, emitter.clone(), &control, &crawl_id, &ctx_arc, &app_data_dir, Some(shared.clone())).await;

        let (pages_archived, cancelled_flag, deep_fetched) = match &result {
            Ok(pages) => {
                let df = pages.iter().filter(|p| p.deep_fetched == Some(true)).count() as u64;
                (pages.len() as u64, control.is_cancelled(), df)
            },
            Err(_) => (0, false, 0),
        };

        shared.deep_fetched_count.store(deep_fetched, std::sync::atomic::Ordering::SeqCst);
        let summary = shared.build_crawl_done_summary(pages_archived, cancelled_flag, &crawl_id);

        let _ = emitter.emit("crawl-done", &serde_json::to_value(&summary).unwrap_or_default()).await;

        let _ = result;
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
    if let Err(e) = crate::ssrf::validate_seed_url(&config.seed_url).await {
        let te = TauriEmitter::new(app);
        let emitter = CraspEmitter::Tauri(te);
        let _ = emit_log(&emitter, "error", "system", &format!("SSRF validation rejected seed URL {}: {}", config.seed_url, e)).await;
        return Err(e);
    }

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

    let crawl_id = format!("crawl_{}", chrono::Utc::now().timestamp_millis());
    state.set_crawl_id(crawl_id.clone());

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let emitter = CraspEmitter::Tauri(TauriEmitter::new(app));
    let ctx_arc = ctx.inner().clone();

    ctx.deep_fetch_queue.reset_counter();

    let _ = emit_log(&emitter, "info", "cloud", &format!("Cloud crawl started: id={}, seed={}", crawl_id, config.seed_url));

    let shared = state.outcomes_arc();

    let handle = tokio::spawn(async move {
        let _control = new_control;
        let client = ZyteClient::new(effective_api_key);

        let _ = emitter.emit("page-stage", &serde_json::json!({
            "url": config.seed_url,
            "crawl_id": crawl_id,
            "stage": PageStage::Discovered,
        })).await;

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

        let _ = emitter.emit("page-stage", &serde_json::json!({
            "url": config.seed_url,
            "crawl_id": crawl_id,
            "stage": PageStage::Fetching,
        })).await;

        let job_key = match client.run_job(&job_req).await {
            Ok(k) => k,
            Err(e) => {
                let _ = emit_log(&emitter, "error", "cloud", &format!("Zyte job submit failed: {}", e));
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": config.seed_url,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed { failed_stage: "zyte_submit".to_string(), reason: e.clone() },
                })).await;
                let summary = shared.build_crawl_done_summary(0, false, &crawl_id);
                let _ = emitter.emit("crawl-done", &serde_json::to_value(&summary).unwrap_or_default()).await;
                return;
            }
        };

        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<ZyteProgress>(100);

        let wait_client = client.clone();
        let wait_job_key = job_key.clone();
        let wait_task = tokio::spawn(async move {
            wait_client.wait_for_job(&wait_job_key, progress_tx).await
        });

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

        if let Err(e) = wait_task.await.unwrap_or(Err("join error".to_string())) {
            let _ = progress_emitter.await;
            let _ = emit_log(&emitter, "error", "cloud", &format!("Zyte job wait failed: {}", e));
            let _ = emitter.emit("page-stage", &serde_json::json!({
                "url": config.seed_url,
                "crawl_id": crawl_id,
                "stage": PageStage::Failed { failed_stage: "zyte_wait".to_string(), reason: e.clone() },
            })).await;
            let summary = shared.build_crawl_done_summary(0, false, &crawl_id);
            let _ = emitter.emit("crawl-done", &serde_json::to_value(&summary).unwrap_or_default()).await;
            return;
        }
        let _ = progress_emitter.await;

        let _ = emitter.emit("page-stage", &serde_json::json!({
            "url": config.seed_url,
            "crawl_id": crawl_id,
            "stage": PageStage::Fetched { status_code: 200 },
        })).await;

        let (items_tx, mut items_rx) = tokio::sync::mpsc::channel::<serde_json::Value>(1000);

        let fetch_client = client.clone();
        let fetch_job_key = job_key.clone();
        let fetch_task = tokio::spawn(async move {
            fetch_client.fetch_items(&fetch_job_key, 100, items_tx).await
        });

        let emitter_items = emitter.clone();
        let ctx_items = ctx_arc.clone();
        let crawl_id_items = crawl_id.clone();
        let app_data_dir_items = app_data_dir.clone();
        let shared_items = shared.clone();
        let items_emitter = tokio::spawn(async move {
            let mut count = 0u64;
            let mut buffer: Vec<serde_json::Value> = Vec::with_capacity(50);
            let mut flush_interval = tokio::time::interval(
                std::time::Duration::from_millis(500)
            );
            flush_interval.tick().await;
            let fallback_active = std::sync::atomic::AtomicBool::new(false);

            loop {
                tokio::select! {
                    item = items_rx.recv() => {
                        match item {
                            Some(item) => {
                                count += 1;
                                let _ = emitter_items.emit("archive-success", &item).await;
                                buffer.push(item);

                                if buffer.len() >= 50 {
                                    let outcomes = persist_items_with_outcome(
                                        &ctx_items,
                                        std::mem::take(&mut buffer),
                                        &crawl_id_items,
                                        &app_data_dir_items,
                                        &fallback_active,
                                    ).await;
                                    emit_persist_stages(&emitter_items, &crawl_id_items, outcomes, Some(&shared_items)).await;
                                }
                            }
                            None => {
                                if !buffer.is_empty() {
                                    let outcomes = persist_items_with_outcome(
                                        &ctx_items,
                                        std::mem::take(&mut buffer),
                                        &crawl_id_items,
                                        &app_data_dir_items,
                                        &fallback_active,
                                    ).await;
                                    emit_persist_stages(&emitter_items, &crawl_id_items, outcomes, Some(&shared_items)).await;
                                }
                                break;
                            }
                        }
                    }
                    _ = flush_interval.tick() => {
                        if !buffer.is_empty() {
                            let outcomes = persist_items_with_outcome(
                                &ctx_items,
                                std::mem::take(&mut buffer),
                                &crawl_id_items,
                                &app_data_dir_items,
                                &fallback_active,
                            ).await;
                            emit_persist_stages(&emitter_items, &crawl_id_items, outcomes, Some(&shared_items)).await;
                        }
                    }
                }
            }
            count
        });

        let _ = fetch_task.await;
        let count = items_emitter.await.unwrap_or(0);

        let summary = shared.build_crawl_done_summary(count, false, &crawl_id);
        let _ = emitter.emit("crawl-done", &serde_json::to_value(&summary).unwrap_or_default()).await;
    });

    {
        let mut h = state.task_handle.lock();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

pub async fn emit_persist_stages(
    emitter: &CraspEmitter,
    crawl_id: &str,
    outcomes: Vec<(serde_json::Value, PersistOutcome)>,
    shared: Option<&Arc<SharedCrawlOutcomes>>,
) {
    for (item, outcome) in outcomes {
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target = match &outcome {
            PersistOutcome::Mongo { db, collection } => PersistTarget::Mongo {
                db: db.clone(),
                collection: collection.clone(),
            },
            PersistOutcome::LocalFile { path } => PersistTarget::LocalFile {
                path: path.clone(),
            },
            PersistOutcome::Failed { reason } => {
                let _ = emit_log(emitter, "error", "system", &format!("Persist failed for {}: {}", url, reason));
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": url,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed {
                        failed_stage: "persisting".to_string(),
                        reason: reason.clone(),
                    },
                })).await;
                if let Some(s) = shared {
                    s.pages_failed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                continue;
            }
        };

        if let Some(s) = shared {
            match &outcome {
                PersistOutcome::Mongo { .. } => {
                    s.used_mongo.store(true, std::sync::atomic::Ordering::SeqCst);
                    s.pages_completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                PersistOutcome::LocalFile { path } => {
                    let mut lp = s.local_file_path.lock();
                    if lp.is_none() {
                        *lp = Some(path.clone());
                    }
                    drop(lp);
                    s.pages_completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                PersistOutcome::Failed { .. } => {}
            }
        }

        let _ = emitter.emit("page-stage", &serde_json::json!({
            "url": url,
            "crawl_id": crawl_id,
            "stage": PageStage::Persisting { target: target.clone() },
        })).await;

        let _ = emitter.emit("page-stage", &serde_json::json!({
            "url": url,
            "crawl_id": crawl_id,
            "stage": PageStage::Persisted { target },
        })).await;
    }
}

#[tauri::command]
pub async fn local_scrapy_crawl(
    app: AppHandle,
    config: CrawlConfig,
    state: tauri::State<'_, CrawlState>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<(), String> {
    if let Err(e) = crate::ssrf::validate_seed_url(&config.seed_url).await {
        let te = TauriEmitter::new(app);
        let emitter = CraspEmitter::Tauri(te);
        let _ = emit_log(&emitter, "error", "system", &format!("SSRF validation rejected seed URL {}: {}", config.seed_url, e)).await;
        return Err(e);
    }

    {
        let mut handle = state.task_handle.lock();
        if let Some(h) = handle.take() {
            h.abort();
        }
    }

    let crawl_id = format!("crawl_{}", chrono::Utc::now().timestamp_millis());
    state.set_crawl_id(crawl_id.clone());

    let new_control = Arc::new(CrawlControl::new());
    {
        let mut ctrl = state.control.lock();
        *ctrl = new_control.clone();
    }

    let control = new_control;
    let ctx_arc = ctx.inner().clone();

    ctx.deep_fetch_queue.reset_counter();

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let emitter = CraspEmitter::Tauri(TauriEmitter::new(app));

    let _ = emit_log(&emitter, "info", "local-scrapy", &format!("Local-Scrapy crawl started: id={}, seed={}", crawl_id, config.seed_url));

    let out_path = format!("{}/crasp_{}.jl", app_data_dir, chrono::Utc::now().timestamp());
    let shared = state.outcomes_arc();

    let handle = tokio::spawn(async move {
        let result = crate::local_scrapy::run_local_spider_streaming(
            &config,
            &out_path,
            &emitter,
            &control,
            &ctx_arc,
            &crawl_id,
            &app_data_dir,
            Some(shared.clone()),
        ).await;

        let (pages_archived, cancelled) = match &result {
            Ok(count) => (*count, control.is_cancelled()),
            Err(_) => (0, false),
        };

        let summary = shared.build_crawl_done_summary(pages_archived, cancelled, &crawl_id);
        let _ = emitter.emit("crawl-done", &serde_json::to_value(&summary).unwrap_or_default()).await;

        let _ = result;
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
pub async fn validate_url(url: String) -> Result<String, String> {
    match crate::ssrf::validate_seed_url(&url).await {
        Ok(parsed) => Ok(parsed.to_string()),
        Err(e) => {
            Err(format!("SSRF validation rejected: {}", e))
        }
    }
}

#[tauri::command]
pub fn default_config() -> CrawlConfig {
    CrawlConfig::default()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MongoConnectionStatus {
    pub ok: bool,
    pub db_name: Option<String>,
    pub pages_count: Option<u64>,
    pub message: Option<String>,
}

#[tauri::command]
pub async fn test_mongo_connection(
    uri: String,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<MongoConnectionStatus, String> {
    let client = match mongodb::Client::with_uri_str(&uri).await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Connection failed: {}", e);
            ctx.set_mongo_state(ServiceState::Unreachable, Some(msg.clone()));
            return Ok(MongoConnectionStatus {
                ok: false,
                db_name: None,
                pages_count: None,
                message: Some(msg),
            });
        }
    };

    let db = client.database("crasp");
    match db.run_command(mongodb::bson::doc! { "ping": 1 }).await {
        Ok(_) => {}
        Err(e) => {
            let msg = format!("Ping failed: {}", e);
            ctx.set_mongo_state(ServiceState::Unreachable, Some(msg.clone()));
            return Ok(MongoConnectionStatus {
                ok: false,
                db_name: None,
                pages_count: None,
                message: Some(msg),
            });
        }
    }

    let pages_count: u64 = db
        .collection::<mongodb::bson::Document>("pages")
        .count_documents(mongodb::bson::doc! {})
        .await
        .unwrap_or(0);

    ctx.set_mongo_state(ServiceState::Connected, Some("crasp".to_string()));

    Ok(MongoConnectionStatus {
        ok: true,
        db_name: Some("crasp".to_string()),
        pages_count: Some(pages_count),
        message: None,
    })
}

#[tauri::command]
pub async fn test_zyte_connection(
    api_key: String,
    project_id: String,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<ZyteConnectionStatus, String> {
    let client = ZyteClient::new(api_key);
    let result = client.test_connection(&project_id).await;

    if let Ok(ref status) = result {
        if status.ok {
            let detail = status.project_name.clone().unwrap_or_else(|| project_id.clone());
            ctx.set_zyte_state(ServiceState::Connected, Some(detail));
        } else {
            let detail = status.message.clone().unwrap_or_else(|| "Connection failed".to_string());
            ctx.set_zyte_state(ServiceState::Unreachable, Some(detail));
        }
    }

    result
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PageSummary {
    pub url: String,
    pub title: String,
    pub depth: u32,
    pub stage: String,
    pub status_reason: Option<String>,
    pub content_size: usize,
    pub timestamp: String,
    pub source: StorageSource,
    pub content_preview: Option<String>,
    pub extracted_title: Option<String>,
    pub author: Option<String>,
    pub published_date: Option<String>,
    pub excerpt: Option<String>,
    pub reading_time_minutes: Option<u32>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub assets: Option<crate::schema::PageAssets>,
    pub extraction_method: Option<String>,
    pub extraction_confidence: Option<f32>,
    pub thin_content: Option<bool>,
    pub deep_fetched: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum StorageSource {
    Mongo,
    LocalFile { path: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalCrawlSummary {
    pub crawl_id: String,
    pub page_count: u64,
    pub file_size_bytes: u64,
    pub last_modified: String,
    pub file_path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum StorageUsed {
    Mongo,
    LocalFile { path: String },
    Both { local_path: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrawlDoneSummary {
    pub pages_archived: u64,
    pub pages_completed: u64,
    pub pages_failed: u64,
    pub pages_skipped: u64,
    pub cancelled: bool,
    pub crawl_id: String,
    pub storage_used: Option<StorageUsed>,
    pub deep_fetched_count: u64,
}

pub fn parse_jl_pages(
    path: &PathBuf,
    crawl_id_filter: Option<&str>,
) -> Result<Vec<PageSummary>, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open JL file {:?}: {}", path, e))?;
    let reader = std::io::BufReader::new(file);
    let mut pages = Vec::new();

    for line_result in std::io::BufRead::lines(reader) {
        let line = line_result.map_err(|e| format!("Read error: {}", e))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let item: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(filter) = crawl_id_filter {
            let item_cid = item
                .get("crawl_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if item_cid != filter {
                continue;
            }
        }

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

        let (status, status_reason) = match item.get("status") {
            Some(serde_json::Value::String(s)) => (s.clone(), None),
            Some(serde_json::Value::Object(map)) => {
                if let Some(v) = map.get("Failed").and_then(|v| v.as_str()) {
                    ("Failed".to_string(), Some(v.to_string()))
                } else if let Some(v) = map.get("Skipped").and_then(|v| v.as_str()) {
                    ("Skipped".to_string(), Some(v.to_string()))
                } else {
                    ("Unknown".to_string(), None)
                }
            }
            _ => {
                let code = item
                    .get("status_code")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as i32;
                if (200..300).contains(&code) {
                    ("Completed".to_string(), None)
                } else {
                    ("Failed".to_string(), Some(format!("HTTP {}", code)))
                }
            }
        };

        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content_size = content.len();
        let content_preview = if content_size > 0 {
            Some(content.chars().take(500).collect())
        } else {
            None
        };
        let timestamp = item
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let extracted_title = item
            .get("extracted_title")
            .and_then(|v| v.as_str())
            .map(String::from);
        let author = item
            .get("author")
            .and_then(|v| v.as_str())
            .map(String::from);
        let published_date = item
            .get("published_date")
            .and_then(|v| v.as_str())
            .map(String::from);
        let excerpt_val = item
            .get("excerpt")
            .and_then(|v| v.as_str())
            .map(String::from);
        let reading_time_minutes = item
            .get("reading_time_minutes")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let body_text = item
            .get("body_text")
            .and_then(|v| v.as_str())
            .map(String::from);
        let body_html = item
            .get("body_html")
            .and_then(|v| v.as_str())
            .map(String::from);
        let assets: Option<crate::schema::PageAssets> = item
            .get("assets")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let extraction_method = item
            .get("extraction_method")
            .and_then(|v| v.as_str())
            .map(String::from);
        let extraction_confidence = item
            .get("extraction_confidence")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);
        let thin_content = item
            .get("thin_content")
            .and_then(|v| v.as_bool());
        let deep_fetched = item
            .get("deep_fetched")
            .and_then(|v| v.as_bool());

        pages.push(PageSummary {
            url,
            title,
            depth,
            stage: status,
            status_reason,
            content_size,
            timestamp: if timestamp.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                timestamp
            },
            source: StorageSource::LocalFile {
                path: path.to_string_lossy().to_string(),
            },
            content_preview,
            extracted_title,
            author,
            published_date,
            excerpt: excerpt_val,
            reading_time_minutes,
            body_text,
            body_html,
            assets,
            extraction_method,
            extraction_confidence,
            thin_content,
            deep_fetched,
        });
    }

    Ok(pages)
}

#[tauri::command]
pub async fn list_local_crawls(
    app: AppHandle,
) -> Result<Vec<LocalCrawlSummary>, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let dir = PathBuf::from(&app_data_dir);
    let entries = tokio::task::spawn_blocking(move || {
        let mut crawls = Vec::new();
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => return crawls,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let filename = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if !filename.starts_with("crawl-") || !filename.ends_with(".jl") {
                continue;
            }

            let crawl_id = filename
                .strip_prefix("crawl-")
                .unwrap_or("")
                .strip_suffix(".jl")
                .unwrap_or("")
                .to_string();

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| {
                    let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
                    Some(chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)?
                        .to_rfc3339())
                })
                .unwrap_or_default();

            let page_count = {
                let p = path.clone();
                std::io::BufReader::new(
                    std::fs::File::open(&p).unwrap()
                )
                .lines()
                .filter(|l| l.as_ref().map_or(false, |s| !s.trim().is_empty()))
                .count() as u64
            };

            crawls.push(LocalCrawlSummary {
                crawl_id,
                page_count,
                file_size_bytes: metadata.len(),
                last_modified: modified,
                file_path: path.to_string_lossy().to_string(),
            });
        }

        crawls
    })
    .await
    .map_err(|e| format!("spawn_blocking panic: {}", e))?;

    Ok(entries)
}

#[tauri::command]
pub async fn list_archived_pages(
    crawl_id: Option<String>,
    ctx: tauri::State<'_, Arc<AppContext>>,
    app: AppHandle,
) -> Result<Vec<PageSummary>, String> {
    let mut mongo_pages = Vec::new();

    if let Some(store) = &ctx.store {
        let mut filter = mongodb::bson::Document::new();
        if let Some(cid) = &crawl_id {
            filter.insert("crawl_id", cid.clone());
        }

        let mut cursor = store
            .pages_col()
            .find(filter)
            .await
            .map_err(|e| format!("Mongo query failed: {}", e))?;

        while let Some(doc) = cursor
            .try_next()
            .await
            .map_err(|e| format!("Cursor error: {}", e))?
        {
            let content_preview = if doc.content.len() > 0 {
                Some(doc.content.chars().take(500).collect())
            } else {
                None
            };
            mongo_pages.push(PageSummary {
                url: doc.url.clone(),
                title: doc.title.clone(),
                depth: doc.depth,
                stage: doc.status.clone(),
                status_reason: doc.status_reason.clone(),
                content_size: doc.content.len(),
                timestamp: doc.timestamp.clone(),
                source: StorageSource::Mongo,
                content_preview,
                extracted_title: doc.extracted_title.clone(),
                author: doc.author.clone(),
                published_date: doc.published_date.clone(),
                excerpt: doc.excerpt.clone(),
                reading_time_minutes: doc.reading_time_minutes,
                body_text: doc.body_text.clone(),
                body_html: doc.body_html.clone(),
                assets: doc.assets.clone(),
            extraction_method: doc.extraction_method.clone(),
            extraction_confidence: doc.extraction_confidence,
            thin_content: doc.thin_content,
            deep_fetched: doc.deep_fetched,
        });
        }
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let crawl_id_clone = crawl_id.clone();
    let local_pages = tokio::task::spawn_blocking(move || {
        let dir = PathBuf::from(&app_data_dir);
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => return Vec::<PageSummary>::new(),
        };

        let mut all_local = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            let filename = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if !filename.starts_with("crawl-") || !filename.ends_with(".jl") {
                continue;
            }

            if let Some(ref cid) = crawl_id_clone {
                let file_cid = filename
                    .strip_prefix("crawl-")
                    .unwrap_or("")
                    .strip_suffix(".jl")
                    .unwrap_or("");
                if file_cid != cid {
                    continue;
                }
            }

            if let Ok(pages) = parse_jl_pages(&path, None) {
                all_local.extend(pages);
            }
        }
        all_local
    })
    .await
    .map_err(|e| format!("spawn_blocking panic: {}", e))?;

    let mut seen_urls = std::collections::HashSet::new();
    for p in &mongo_pages {
        seen_urls.insert(p.url.clone());
    }
    let local_only: Vec<PageSummary> = local_pages
        .into_iter()
        .filter(|p| !seen_urls.contains(&p.url))
        .collect();

    let mut all_pages = mongo_pages;
    all_pages.extend(local_only);
    Ok(all_pages)
}

#[tauri::command]
pub async fn get_page_content(
    url: String,
    source: StorageSource,
    crawl_id: Option<String>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<Option<String>, String> {
    let doc = get_page_doc(url, source, crawl_id, ctx).await?;
    Ok(doc.map(|d| d.content.clone()))
}

async fn get_page_doc(
    url: String,
    source: StorageSource,
    crawl_id: Option<String>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<Option<crate::schema::PageDoc>, String> {
    match source {
        StorageSource::Mongo => {
            if let Some(store) = &ctx.store {
                let mut filter = mongodb::bson::doc! { "url": &url };
                if let Some(cid) = &crawl_id {
                    filter.insert("crawl_id", cid);
                }
                let doc = store
                    .pages_col()
                    .find_one(filter)
                    .await
                    .map_err(|e| format!("Mongo query failed: {}", e))?;
                Ok(doc)
            } else {
                Ok(None)
            }
        }
        StorageSource::LocalFile { path } => {
            let url_clone = url.clone();
            let crawl_id_clone = crawl_id.clone();
            let doc = tokio::task::spawn_blocking(move || {
                let file = std::fs::File::open(&path)
                    .map_err(|e| format!("Failed to open JL file: {}", e))?;
                let reader = std::io::BufReader::new(file);
                for line_result in std::io::BufRead::lines(reader) {
                    let line = line_result.map_err(|e| format!("Read error: {}", e))?;
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let item: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let item_url = item
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if item_url == url_clone {
                        if let Some(cid) = &crawl_id_clone {
                            let item_cid = item
                                .get("crawl_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if item_cid != cid {
                                continue;
                            }
                        }
                        let page_doc = crate::schema::PageDoc {
                            crawl_id: item
                                .get("crawl_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            url: item_url.to_string(),
                            url_normalized: item_url.to_lowercase(),
                            depth: item
                                .get("depth")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32,
                            title: item
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            status: item
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Completed")
                                .to_string(),
                            status_code: item
                                .get("status_code")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0) as i32,
                            status_reason: item
                                .get("status_reason")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            content: item
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            content_format: "text".to_string(),
                            content_bytes: None,
                            discovered_links: item
                                .get("discovered_links")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32,
                            timestamp: item
                                .get("timestamp")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            duplicate_group_id: 0,
                            search_blob: String::new(),
                            extracted_title: item
                                .get("extracted_title")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            author: item
                                .get("author")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            published_date: item
                                .get("published_date")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            excerpt: item
                                .get("excerpt")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            reading_time_minutes: item
                                .get("reading_time_minutes")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32),
                            body_text: item
                                .get("body_text")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            body_html: item
                                .get("body_html")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            assets: item
                                .get("assets")
                                .and_then(|v| serde_json::from_value(v.clone()).ok()),
                            extraction_method: item
                                .get("extraction_method")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            extraction_confidence: item
                                .get("extraction_confidence")
                                .and_then(|v| v.as_f64())
                                .map(|v| v as f32),
                            thin_content: item
                                .get("thin_content")
                                .and_then(|v| v.as_bool()),
                            deep_fetched: item
                                .get("deep_fetched")
                                .and_then(|v| v.as_bool()),
                        };
                        return Ok(Some(page_doc));
                    }
                }
                Ok(None)
            })
            .await
            .map_err(|e| format!("spawn_blocking panic: {}", e))?
            .map_err(|e: String| e)?;
            Ok(doc)
        }
    }
}



#[tauri::command]
pub async fn export_content(
    request: crate::export::ExportRequest,
    app: AppHandle,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<crate::export::ExportResult, String> {
    use std::io::Write;

    request.is_valid()?;

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let exports_dir = std::path::PathBuf::from(&app_data_dir).join("exports");
    std::fs::create_dir_all(&exports_dir)
        .map_err(|e| format!("Failed to create exports dir: {}", e))?;

    match request.scope {
        crate::export::ExportScope::SinglePage => {
            let page_url = request.page_url.as_ref().unwrap();
            let source = request.source.as_ref().unwrap_or(&StorageSource::Mongo).clone();
            let page_doc = get_page_doc(page_url.clone(), source, request.crawl_id.clone(), ctx).await?;
            let page_doc = page_doc.ok_or("Page not found".to_string())?;

            let (ext, content) = match request.format {
                crate::export::ExportFormat::PlainText => (
                    "txt",
                    crate::export::page_to_plain_text(&page_doc, &request.content),
                ),
                crate::export::ExportFormat::Markdown => (
                    "md",
                    crate::export::page_to_markdown(&page_doc, &request.content),
                ),
                crate::export::ExportFormat::Html => (
                    "html",
                    crate::export::page_to_html(&page_doc, &request.content),
                ),
                crate::export::ExportFormat::Epub => {
                    return Err("EPUB is not supported for single page export".to_string());
                }
            };

            let slug: String = page_url
                .split('/')
                .filter(|s| !s.is_empty() && *s != "http:" && *s != "https:")
                .last()
                .unwrap_or("page")
                .chars()
                .take(30)
                .collect();
            let ts = chrono::Utc::now().timestamp_millis();
            let filename = format!("{}_{}.{}_{}_{}", slug, ts, ext, request.format_string(), request.scope_string());
            let path = exports_dir.join(&filename);
            let mut file = std::fs::File::create(&path)
                .map_err(|e| format!("Failed to create file: {}", e))?;
            file.write_all(content.as_bytes())
                .map_err(|e| format!("Failed to write file: {}", e))?;

            Ok(crate::export::ExportResult {
                path: path.to_string_lossy().to_string(),
                page_count: 1,
                format: request.format_string(),
                scope: request.scope_string(),
            })
        }
        crate::export::ExportScope::WholeCrawlOneFile | crate::export::ExportScope::WholeCrawlFolder => {
            let crawl_id = request.crawl_id.as_ref().unwrap().clone();
            let mut page_docs: Vec<crate::schema::PageDoc> = Vec::new();

            if let Some(store) = &ctx.store {
                let filter = mongodb::bson::doc! { "crawl_id": &crawl_id };
                let mut cursor = store
                    .pages_col()
                    .find(filter)
                    .await
                    .map_err(|e| format!("Mongo query failed: {}", e))?;

                while let Some(doc) = cursor
                    .try_next()
                    .await
                    .map_err(|e| format!("Cursor error: {}", e))?
                {
                    page_docs.push(doc);
                }
            }

            if page_docs.is_empty() {
                let crawl_id_clone = crawl_id.clone();
                let app_data_dir_clone = app_data_dir.clone();
                page_docs = tokio::task::spawn_blocking(move || {
                    let dir = std::path::PathBuf::from(&app_data_dir_clone);
                    let read_dir = match std::fs::read_dir(&dir) {
                        Ok(rd) => rd,
                        Err(_) => return Vec::new(),
                    };
                    let mut docs = Vec::new();
                    for entry in read_dir.flatten() {
                        let path = entry.path();
                        let filename = match path.file_name() {
                            Some(n) => n.to_string_lossy().to_string(),
                            None => continue,
                        };
                        if !filename.starts_with("crawl-") || !filename.ends_with(".jl") {
                            continue;
                        }
                        let file_cid = filename
                            .strip_prefix("crawl-")
                            .unwrap_or("")
                            .strip_suffix(".jl")
                            .unwrap_or("");
                        if file_cid != crawl_id_clone {
                            continue;
                        }
                        let file = match std::fs::File::open(&path) {
                            Ok(f) => f,
                            Err(_) => continue,
                        };
                        let reader = std::io::BufReader::new(file);
                        for line_result in std::io::BufRead::lines(reader) {
                            let line = match line_result {
                                Ok(l) => l,
                                Err(_) => continue,
                            };
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let item: serde_json::Value = match serde_json::from_str(trimmed) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            if item.get("crawl_id").and_then(|v| v.as_str()).unwrap_or("") != crawl_id_clone {
                                continue;
                            }
                            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let timestamp = item.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let pd = crate::schema::PageDoc {
                                crawl_id: crawl_id_clone.clone(),
                                url: url.clone(),
                                url_normalized: url.to_lowercase(),
                                depth: item.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                status: "Completed".to_string(),
                                status_code: 200,
                                status_reason: None,
                                content: item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                content_format: "text".to_string(),
                                content_bytes: None,
                                discovered_links: 0,
                                timestamp,
                                duplicate_group_id: 0,
                                search_blob: String::new(),
                                extracted_title: item.get("extracted_title").and_then(|v| v.as_str()).map(String::from),
                                author: item.get("author").and_then(|v| v.as_str()).map(String::from),
                                published_date: item.get("published_date").and_then(|v| v.as_str()).map(String::from),
                                excerpt: item.get("excerpt").and_then(|v| v.as_str()).map(String::from),
                                reading_time_minutes: item.get("reading_time_minutes").and_then(|v| v.as_u64()).map(|v| v as u32),
                                body_text: item.get("body_text").and_then(|v| v.as_str()).map(String::from),
                                body_html: item.get("body_html").and_then(|v| v.as_str()).map(String::from),
                                assets: item.get("assets").and_then(|v| serde_json::from_value(v.clone()).ok()),
                                extraction_method: item.get("extraction_method").and_then(|v| v.as_str()).map(String::from),
                                extraction_confidence: item.get("extraction_confidence").and_then(|v| v.as_f64()).map(|v| v as f32),
                                thin_content: item.get("thin_content").and_then(|v| v.as_bool()),
                                deep_fetched: item.get("deep_fetched").and_then(|v| v.as_bool()),
                            };
                            docs.push(pd);
                        }
                    }
                    docs
                })
                .await
                .map_err(|e| format!("spawn_blocking panic: {}", e))?;
            }

            if page_docs.is_empty() {
                return Err("No pages found for this crawl".to_string());
            }

            page_docs.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.url.cmp(&b.url)));
            let page_count = page_docs.len();

            match request.format {
                crate::export::ExportFormat::Epub => {
                    let chapters: Vec<crate::export::EpubChapter> = page_docs
                        .iter()
                        .map(crate::export::page_to_epub_chapter)
                        .collect();

                    let book_title = chapters
                        .first()
                        .map(|c| c.title.clone())
                        .unwrap_or_else(|| "Crasp Archive".to_string());

                    let cover_url = page_docs
                        .first()
                        .and_then(|d| d.assets.as_ref())
                        .and_then(|a| a.og_image.as_deref());

                    let ts = chrono::Utc::now().timestamp_millis();
                    let output_path = exports_dir.join(format!("{}_{}.epub", crawl_id, ts));

                    crate::export::generate_epub(
                        &chapters,
                        &book_title,
                        cover_url,
                        &output_path,
                    )?;

                    Ok(crate::export::ExportResult {
                        path: output_path.to_string_lossy().to_string(),
                        page_count,
                        format: request.format_string(),
                        scope: request.scope_string(),
                    })
                }
                _ => {
                    match request.scope {
                        crate::export::ExportScope::WholeCrawlOneFile => {
                            let (ext, content) = match request.format {
                                crate::export::ExportFormat::PlainText => (
                                    "txt",
                                    crate::export::pages_to_plain_text_combined(&page_docs, &request.content),
                                ),
                                crate::export::ExportFormat::Markdown => (
                                    "md",
                                    crate::export::pages_to_markdown_combined(&page_docs, &request.content),
                                ),
                                crate::export::ExportFormat::Html => (
                                    "html",
                                    crate::export::pages_to_html_combined(&page_docs, &request.content),
                                ),
                                _ => unreachable!(),
                            };

                            let ts = chrono::Utc::now().timestamp_millis();
                            let filename = format!("{}_{}_{}_{}", crawl_id, ext, ts, request.scope_string());
                            let path = exports_dir.join(&filename);
                            let mut file = std::fs::File::create(&path)
                                .map_err(|e| format!("Failed to create file: {}", e))?;
                            file.write_all(content.as_bytes())
                                .map_err(|e| format!("Failed to write file: {}", e))?;

                            Ok(crate::export::ExportResult {
                                path: path.to_string_lossy().to_string(),
                                page_count,
                                format: request.format_string(),
                                scope: request.scope_string(),
                            })
                        }
                        crate::export::ExportScope::WholeCrawlFolder => {
                            let files = match request.format {
                                crate::export::ExportFormat::PlainText => {
                                    crate::export::pages_to_plain_text_folder(&page_docs, &request.content)
                                }
                                crate::export::ExportFormat::Markdown => {
                                    crate::export::pages_to_markdown_folder(&page_docs, &request.content)
                                }
                                crate::export::ExportFormat::Html => {
                                    crate::export::pages_to_html_folder(&page_docs, &request.content)
                                }
                                _ => unreachable!(),
                            };

                            let ts = chrono::Utc::now().timestamp_millis();
                            let folder_name = format!("{}_{}_{}", crawl_id, request.format_string(), ts);
                            let folder_path = exports_dir.join(&folder_name);
                            std::fs::create_dir_all(&folder_path)
                                .map_err(|e| format!("Failed to create folder: {}", e))?;

                            for (filename, content) in files {
                                let file_path = folder_path.join(&filename);
                                let mut file = std::fs::File::create(&file_path)
                                    .map_err(|e| format!("Failed to create file: {}", e))?;
                                file.write_all(content.as_bytes())
                                    .map_err(|e| format!("Failed to write file: {}", e))?;
                            }

                            Ok(crate::export::ExportResult {
                                path: folder_path.to_string_lossy().to_string(),
                                page_count,
                                format: request.format_string(),
                                scope: request.scope_string(),
                            })
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub async fn deep_fetch_page(
    url: String,
    crawl_id: String,
    ctx: tauri::State<'_, Arc<AppContext>>,
    app: AppHandle,
) -> Result<PageSummary, String> {
    crate::ssrf::validate_seed_url(&url).await?;

    let zyte = ctx
        .zyte
        .as_ref()
        .ok_or("Zyte not configured — set ZYTE_API_KEY to enable deep fetch")?;

    let _permit = ctx.deep_fetch_queue.acquire().await?;

    let result = zyte.deep_fetch(&url, &ctx.http).await?;

    let extraction = if let Some(ref article) = result.article {
        crate::zyte::zyte_article_to_extraction(article, &result.browser_html, &url)
    } else {
        tokio::task::spawn_blocking({
            let html = result.browser_html.clone();
            let url = url.clone();
            move || {
                let er = crate::extraction::extract_main_content(&html, &url);
                crate::extraction::ZyteExtractionResult {
                    title: er.title,
                    author: er.author,
                    published_date: er.published_date,
                    excerpt: er.excerpt,
                    body_html: er.body_html,
                    body_text: er.body_text,
                    reading_time_minutes: er.reading_time_minutes,
                    confidence: er.confidence,
                    method: er.method.clone(),
                    thin_content: er.thin_content,
                }
            }
        })
        .await
        .map_err(|e| e.to_string())?
    };

    if let Some(store) = &ctx.store {
        let filter = mongodb::bson::doc! {
            "url": &url,
            "crawl_id": &crawl_id,
        };
        let update = mongodb::bson::doc! {
            "$set": {
                "body_html": &extraction.body_html,
                "body_text": &extraction.body_text,
                "extraction_method": &extraction.method,
                "extraction_confidence": extraction.confidence,
                "thin_content": extraction.thin_content,
                "deep_fetched": true,
            }
        };
        let options = mongodb::options::UpdateOptions::builder().build();
        let _ = store
            .pages_col()
            .update_one(filter, update)
            .with_options(options)
            .await;
    }

    let emitter = CraspEmitter::Tauri(TauriEmitter::new(app));
    let _ = emit_log(
        &emitter,
        "info",
        "local",
        &format!(
            "Manual deep fetch complete for {} — method={}, confidence={:.2}, thin={}",
            url, extraction.method, extraction.confidence, extraction.thin_content
        ),
    )
    .await;

    Ok(PageSummary {
        url: url.clone(),
        title: String::new(),
        depth: 0,
        stage: "Completed".to_string(),
        status_reason: None,
        content_size: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        source: StorageSource::Mongo,
        content_preview: None,
        extracted_title: extraction.title,
        author: extraction.author,
        published_date: extraction.published_date,
        excerpt: extraction.excerpt,
        reading_time_minutes: Some(extraction.reading_time_minutes),
        body_text: if !extraction.body_text.is_empty() {
            Some(extraction.body_text)
        } else {
            None
        },
        body_html: if !extraction.body_html.is_empty() {
            Some(extraction.body_html)
        } else {
            None
        },
        assets: None,
        extraction_method: Some(extraction.method),
        extraction_confidence: Some(extraction.confidence),
        thin_content: Some(extraction.thin_content),
        deep_fetched: Some(true),
    })
}

#[tauri::command]
pub fn reveal_in_explorer(
    path: String,
) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn open_data_folder(
    app: AppHandle,
) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    std::fs::create_dir_all(&app_data_dir)
        .map_err(|e| format!("Failed to create app data dir: {}", e))?;

    tauri_plugin_opener::open_path(&app_data_dir, None::<&str>)
        .map_err(|e| format!("Failed to open data folder: {}", e))
}

#[tauri::command]
pub fn get_last_crawl_summary(
    state: tauri::State<'_, CrawlState>,
) -> Result<Option<CrawlDoneSummary>, String> {
    let crawl_id = state.crawl_id.lock().clone();
    let Some(cid) = crawl_id else { return Ok(None) };
    let completed = state.outcomes.pages_completed.load(std::sync::atomic::Ordering::SeqCst);
    let failed = state.outcomes.pages_failed.load(std::sync::atomic::Ordering::SeqCst);
    let skipped = state.outcomes.pages_skipped.load(std::sync::atomic::Ordering::SeqCst);
    if completed == 0 && failed == 0 && skipped == 0 {
        return Ok(None);
    }
    Ok(Some(state.outcomes.build_crawl_done_summary(
        completed + failed + skipped,
        false,
        &cid,
    )))
}
