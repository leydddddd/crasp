use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::TryStreamExt;
use tauri::{AppHandle, Emitter, Manager};

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
}

impl CrawlState {
    pub fn new() -> Self {
        Self {
            control: parking_lot::Mutex::new(Arc::new(CrawlControl::new())),
            task_handle: parking_lot::Mutex::new(None),
            crawl_id: parking_lot::Mutex::new(None),
        }
    }

    pub fn set_crawl_id(&self, id: String) {
        *self.crawl_id.lock() = Some(id);
    }

    #[allow(dead_code)]
    pub fn get_crawl_id(&self) -> Option<String> {
        self.crawl_id.lock().clone()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum PersistOutcome {
    Mongo { db: String, collection: String },
    LocalFile { path: String },
    Failed { reason: String },
}

fn local_fallback_path(app_data_dir: &str, crawl_id: &str) -> PathBuf {
    let dir = PathBuf::from(app_data_dir);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("crawl-{}.jl", crawl_id))
}

fn append_to_jl(path: &PathBuf, items: &[serde_json::Value]) -> Result<(), String> {
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

    let emitter = TauriEmitter::new(app);
    let control = new_control;
    let ctx_arc = ctx.inner().clone();

    control.reset();

    let _ = emit_log(&emitter, "info", "local", &format!("Crawl started: id={}, seed={}", crawl_id, config.seed_url));

    let handle = tokio::spawn(async move {
        let crawler = Crawler::new();
        let _ = crawler.run(config, emitter, &control, &crawl_id, &ctx_arc, &app_data_dir).await;
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

    let emitter = TauriEmitter::new(app);
    let ctx_arc = ctx.inner().clone();

    let _ = emit_log(&emitter, "info", "cloud", &format!("Cloud crawl started: id={}, seed={}", crawl_id, config.seed_url));

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
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": config.seed_url,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed { failed_stage: "zyte_submit".to_string(), reason: e.clone() },
                })).await;
                let _ = emitter.emit("crawl-done", &serde_json::json!({
                    "pages_archived": 0,
                    "cancelled": false,
                    "error": e,
                })).await;
                return;
            }
        };

        // Cloud crawl: the remote job handles Discovered -> Fetching ->
        // Fetched. After the job finishes we fetch items and persist them.
        // Intermediate pipeline stages are not observable remotely, so we
        // emit at coarser granularity: Discovered -> Fetching -> Persisting
        // -> Persisted (or Failed).

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
            let _ = emitter.emit("page-stage", &serde_json::json!({
                "url": config.seed_url,
                "crawl_id": crawl_id,
                "stage": PageStage::Failed { failed_stage: "zyte_wait".to_string(), reason: e.clone() },
            })).await;
            let _ = emitter.emit("crawl-done", &serde_json::json!({
                "pages_archived": 0,
                "cancelled": false,
                "error": e,
            })).await;
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
                                    emit_persist_stages(&emitter_items, &crawl_id_items, outcomes).await;
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
                                    emit_persist_stages(&emitter_items, &crawl_id_items, outcomes).await;
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
                            emit_persist_stages(&emitter_items, &crawl_id_items, outcomes).await;
                        }
                    }
                }
            }
            count
        });

        let _ = fetch_task.await;
        let count = items_emitter.await.unwrap_or(0);

        let _ = emitter.emit("crawl-done", &serde_json::json!({
            "pages_archived": count,
            "cancelled": false,
            "crawl_id": crawl_id,
        })).await;
    });

    {
        let mut h = state.task_handle.lock();
        *h = Some(handle.abort_handle());
    }

    Ok(())
}

pub async fn emit_persist_stages(
    emitter: &TauriEmitter,
    crawl_id: &str,
    outcomes: Vec<(serde_json::Value, PersistOutcome)>,
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
                continue;
            }
        };

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

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let emitter = TauriEmitter::new(app);

    let _ = emit_log(&emitter, "info", "local-scrapy", &format!("Local-Scrapy crawl started: id={}, seed={}", crawl_id, config.seed_url));

    let out_path = format!("{}/crasp_{}.jl", app_data_dir, chrono::Utc::now().timestamp());

    let handle = tokio::spawn(async move {
        crate::local_scrapy::run_local_spider_streaming(
            &config,
            &out_path,
            &emitter,
            &control,
            &ctx_arc,
            &crawl_id,
            &app_data_dir,
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
pub async fn validate_url(url: String) -> Result<String, String> {
    crate::ssrf::validate_seed_url(&url).await?;
    Ok(url)
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
}

#[tauri::command]
pub async fn list_archived_pages(
    crawl_id: Option<String>,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<Vec<PageSummary>, String> {
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

        let mut summaries = Vec::new();
        while let Some(doc) = cursor
            .try_next()
            .await
            .map_err(|e| format!("Cursor error: {}", e))?
        {
            summaries.push(PageSummary {
                url: doc.url.clone(),
                title: doc.title.clone(),
                depth: doc.depth,
                stage: doc.status.clone(),
                status_reason: doc.status_reason.clone(),
                content_size: doc.content.len(),
                timestamp: doc.timestamp.clone(),
            });
        }

        Ok(summaries)
    } else {
        Ok(Vec::new())
    }
}

#[tauri::command]
pub async fn export_page(
    crawl_id: String,
    url: String,
    format: String,
    ctx: tauri::State<'_, Arc<AppContext>>,
) -> Result<String, String> {
    let slug: String = url
        .split('/')
        .filter(|s| !s.is_empty() && *s != "http:" && *s != "https:")
        .last()
        .unwrap_or("page")
        .chars()
        .take(30)
        .collect();

    let content = if let Some(store) = &ctx.store {
        let filter = mongodb::bson::doc! {
            "crawl_id": crawl_id,
            "url": url,
        };
        let doc = store
            .pages_col()
            .find_one(filter)
            .await
            .map_err(|e| format!("Mongo query failed: {}", e))?;

        match doc {
            Some(page) => page.content.clone(),
            None => return Err("Page not found in store".to_string()),
        }
    } else {
        return Err("No database connected, cannot look up page".to_string());
    };

    let content = if content.is_empty() {
        return Err("Page has no content".to_string());
    } else {
        content
    };

    let ext = if format == "md" { "md" } else { "txt" };
    let ts = chrono::Utc::now().timestamp_millis();
    let filename = format!("{}_{}.{}", slug, ts, ext);

    use std::io::Write;
    let dir = std::env::temp_dir().join("crasp_exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create export dir: {}", e))?;
    let path = dir.join(&filename);
    let mut file =
        std::fs::File::create(&path).map_err(|e| format!("Failed to create file: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}
