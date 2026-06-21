use std::sync::Arc;
use std::sync::OnceLock;

use chrono::Utc;
use dashmap::DashSet;
use digest::Digest;
use ego_tree::NodeRef;
use md5::Md5;
use scraper::{Html, Node, Selector};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::{mpsc, watch};
use url::Url;

use phf::phf_set;

use crate::commands::{persist_items_with_outcome, emit_persist_stages, SharedCrawlOutcomes};
use crate::progress::CraspEmitter;
use crate::logging::emit_log;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlConfig {
    pub seed_url: String,
    pub max_depth: u32,
    pub max_pages: u32,
    pub concurrency: usize,
    pub css_selectors: Vec<String>,
    pub preserve_html: bool,
    pub hash_algorithm: HashAlgorithm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgorithm {
    Md5,
    Sha256,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            seed_url: String::new(),
            max_depth: 3,
            max_pages: 100,
            concurrency: 4,
            css_selectors: vec!["article".into(), "main".into(), "body".into()],
            preserve_html: true,
            hash_algorithm: HashAlgorithm::Sha256,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PersistTarget {
    Mongo { db: String, collection: String },
    LocalFile { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum PageStage {
    Discovered,
    Fetching,
    Fetched { status_code: u16 },
    Parsing,
    Sanitizing,
    Preserving,
    Hashing,
    Persisting { target: PersistTarget },
    Persisted { target: PersistTarget },
    Failed { failed_stage: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PageStatus {
    Pending,
    Fetching,
    Scraping,
    Archiving,
    Completed,
    Failed(String),
    Skipped(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedPage {
    pub url: String,
    pub depth: u32,
    pub status: PageStatus,
    pub title: String,
    pub content: Option<String>,
    pub hash: Option<String>,
    pub hash_algorithm: Option<String>,
    pub discovered_links: u32,
    pub timestamp: String,
    pub crawl_id: Option<String>,
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
}

#[allow(dead_code)]
struct FrontierItem {
    url: String,
    depth: u32,
    parent: Option<String>,
}

struct WorkerOutput {
    page: ArchivedPage,
    found_links: Vec<String>,
}

pub struct CrawlControl {
    cancel_tx: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    pause_tx: watch::Sender<bool>,
    pause_rx: watch::Receiver<bool>,
    paused: Arc<std::sync::atomic::AtomicBool>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

#[allow(dead_code)]
impl CrawlControl {
    pub fn new() -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (pause_tx, pause_rx) = watch::channel(false);

        Self {
            cancel_tx,
            cancel_rx,
            pause_tx,
            pause_rx,
            paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.cancel_tx.send(true);
    }

    pub fn pause(&self) {
        self.paused.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.pause_tx.send(true);
    }

    pub fn resume(&self) {
        self.paused.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = self.pause_tx.send(false);
    }

    pub fn reset(&self) {
        self.cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
        self.paused.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = self.cancel_tx.send(false);
        let _ = self.pause_tx.send(false);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn cancel_rx(&self) -> watch::Receiver<bool> {
        self.cancel_rx.clone()
    }

    pub fn pause_rx(&self) -> watch::Receiver<bool> {
        self.pause_rx.clone()
    }
}

/// Atomically reserve a budget slot for a URL to be crawled.
///
/// Uses fetch_add + rollback to close the TOCTOU window that existed when
/// load() and fetch_add() were separate operations. Returns true if a slot
/// was reserved (the caller should proceed), false if the budget is exhausted.
///
/// If the counter was already >= max_pages, the increment is rolled back
/// and the URL is effectively rejected — but it may already be in the
/// visited set, which is harmless and prevents redundant re-injection.
fn try_reserve_slot(
    counter: &std::sync::atomic::AtomicU32,
    max_pages: u32,
) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    // fetch_add returns the PREVIOUS value.
    let prev = counter.fetch_add(1, SeqCst);
    if prev >= max_pages {
        counter.fetch_sub(1, SeqCst);
        false
    } else {
        true
    }
}

pub struct Crawler {
    client: reqwest::Client,
}

impl Crawler {
    pub fn new() -> Self {
        let client = crate::ssrf::build_safe_http_client();

        Self { client }
    }

    pub async fn run(
        self,
        config: CrawlConfig,
        emitter: CraspEmitter,
        control: &CrawlControl,
        crawl_id: &str,
        ctx: &Arc<crate::runtime::AppContext>,
        app_data_dir: &str,
        shared_outcomes: Option<Arc<SharedCrawlOutcomes>>,
    ) -> Result<Vec<ArchivedPage>, String> {
        let seed = Url::parse(&config.seed_url).map_err(|e| e.to_string())?;
        let domain = seed.domain().ok_or("seed URL has no domain")?.to_string();
        let scheme = seed.scheme().to_string();

        let (work_tx, work_rx) = mpsc::channel::<FrontierItem>(config.max_pages as usize * 2);
        let (result_tx, mut result_rx) = mpsc::channel::<ArchivedPage>(config.max_pages as usize * 2);

        let visited = Arc::new(DashSet::<String>::new());
        let pages_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Mark seed URL as visited (prevents duplicate injection by the
        // collector), then atomically reserve a budget slot and send.
        {
            let key = normalize_url(&config.seed_url);
            visited.insert(key);
        }

        if try_reserve_slot(&pages_count, config.max_pages) {
            let _ = work_tx.send(FrontierItem {
                url: config.seed_url.clone(),
                depth: 0,
                parent: None,
            }).await;
        }

        let client = self.client;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency));
        let cancelled_flag = cancelled.clone();

        let (done_tx, mut done_rx) = mpsc::channel::<WorkerOutput>(1000);

        let cfg_worker = config.clone();
        let emitter_worker = emitter.clone();
        let crawl_id_worker = crawl_id.to_string();

        let mut worker_cancel_rx = control.cancel_rx();
        let worker_pause_rx = control.pause_rx();

        let done_tx_shared: Arc<mpsc::Sender<WorkerOutput>> = Arc::new(done_tx);
        let outstanding_tasks: Arc<std::sync::atomic::AtomicU32> = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let worker_loop = {
            let done_tx_shared = done_tx_shared.clone();
            let semaphore = semaphore.clone();
            let cancelled_flag = cancelled_flag.clone();
            let outstanding = outstanding_tasks.clone();

            tokio::spawn(async move {
                let mut rx = work_rx;
                loop {
                    let item = tokio::select! {
                        item = rx.recv() => {
                            match item {
                                Some(i) => i,
                                None => {
                                    break;
                                }
                            }
                        }
                        _ = worker_cancel_rx.changed() => {
                            if *worker_cancel_rx.borrow() {
                                break;
                            }
                            continue;
                        }
                    };

                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }

                    let mut permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };

                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }

                    let mut pause_rx = worker_pause_rx.clone();
                    if *pause_rx.borrow() {
                        drop(permit);

                        loop {
                            tokio::select! {
                                result = pause_rx.changed() => {
                                    result.ok();
                                    if !*pause_rx.borrow() {
                                        break;
                                    }
                                }
                                _ = worker_cancel_rx.changed() => {
                                    if *worker_cancel_rx.borrow() {
                                        break;
                                    }
                                }
                            }
                        }

                        if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                            break;
                        }

                        permit = match semaphore.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => break,
                        };
                    }

                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }

                    let c = client.clone();
                    let cf = cfg_worker.clone();
                    let em = emitter_worker.clone();
                    let tx = (*done_tx_shared).clone();
                    let cid = crawl_id_worker.clone();
                    let out_count = outstanding.clone();

                    out_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                    tokio::spawn(async move {
                        let _permit = permit;
                        let output = process_page(c, item, &cf, &em, &cid).await;
                        let _ = tx.send(output).await;
                        out_count.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                }
                drop(done_tx_shared);
            })
        };

        let inject_tx = work_tx.clone();
        let i_visited = visited.clone();
        let i_pages_count = pages_count.clone();
        let i_max_pages = config.max_pages;
        let i_max_depth = config.max_depth;
        let i_domain = domain.clone();
        let i_scheme = scheme.clone();
        let i_emitter = emitter.clone();
        let i_cancelled = cancelled_flag.clone();

        let collector = tokio::spawn(async move {
            while let Some(output) = done_rx.recv().await {

                let archived = output.page.clone();

                let _ = result_tx.send(archived.clone()).await;

                if i_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    continue;
                }

                for link in &output.found_links {
                    if let Ok(parsed) = Url::parse(link) {
                        if parsed.domain().map(|d| d == i_domain) != Some(true) {
                            continue;
                        }
                        if parsed.scheme() != i_scheme {
                            continue;
                        }

                        let mut fragless_url = parsed.clone();
                        fragless_url.set_fragment(None);
                        let fragless = fragless_url.to_string();
                        let key = normalize_url(&fragless);

                        let depth = archived.depth + 1;
                        if depth > i_max_depth {
                            continue;
                        }

                        let should_inject = i_visited.insert(key);

                        if should_inject {
                            // Atomically reserve a budget slot. If the budget
                            // is exhausted, the URL stays in visited (harmless —
                            // prevents re-injection if we see it again later),
                            // but we don't schedule or count it.
                            if !try_reserve_slot(&i_pages_count, i_max_pages) {
                                continue;
                            }

                            let _ = i_emitter.emit("crawl-discover", &serde_json::json!({
                                "url": link,
                                "depth": depth,
                                "parent": archived.url,
                            })).await;

                            let _ = inject_tx.send(FrontierItem {
                                url: fragless,
                                depth,
                                parent: Some(archived.url.clone()),
                            }).await;
                        }
                    }
                }
            }

        });

        // ---- Channel lifetime invariants ----
        // work_tx is dropped below. The collector holds inject_tx (a clone).
        // When the main loop has collected all expected pages, it breaks out
        // and aborts the worker_loop and collector tasks.
        drop(work_tx);
        drop(done_tx_shared);



        let mut results: Vec<ArchivedPage> = Vec::new();
        let emitter_result = emitter.clone();
        let mut cancel_rx = control.cancel_rx();
        let mut pause_rx = control.pause_rx();

        let mut persist_buffer: Vec<serde_json::Value> = Vec::with_capacity(50);
        let fallback_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let crawl_id_owned = crawl_id.to_string();
        let app_data_dir_owned = app_data_dir.to_string();
        let ctx_result = ctx.clone();
        let expected_results = pages_count.clone();

        loop {
            if results.len() as u32 >= expected_results.load(std::sync::atomic::Ordering::SeqCst)
                && outstanding_tasks.load(std::sync::atomic::Ordering::SeqCst) == 0
            {
                break;
            }

            if *pause_rx.borrow() {
                tokio::select! {
                    _ = pause_rx.changed() => {
                        if !*pause_rx.borrow() {
                            continue;
                        }
                    }
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            cancelled_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                            break;
                        }
                    }
                }
                continue;
            }

            tokio::select! {
                page = result_rx.recv() => {
                    match page {
                        Some(p) => {
                            let _ = emitter_result.emit("archive-success", &serde_json::to_value(&p).unwrap_or_default()).await;
                            results.push(p.clone());
                            persist_buffer.push(serde_json::to_value(&p).unwrap_or_default());
                            if persist_buffer.len() >= 50 {
                                let outcomes = persist_items_with_outcome(
                                    &ctx_result,
                                    std::mem::take(&mut persist_buffer),
                                    &crawl_id_owned,
                                    &app_data_dir_owned,
                                    &fallback_active,
                                ).await;
                                emit_persist_stages(&emitter_result, &crawl_id_owned, outcomes, shared_outcomes.as_ref()).await;
                            }
                        }
                        None => break,
                    }
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        cancelled_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        break;
                    }
                }
                _ = pause_rx.changed() => {
                    if *pause_rx.borrow() {
                        continue;
                    }
                }
            }
        }



        if !persist_buffer.is_empty() {
            let outcomes = persist_items_with_outcome(
                &ctx_result,
                std::mem::take(&mut persist_buffer),
                &crawl_id_owned,
                &app_data_dir_owned,
                &fallback_active,
            ).await;
            emit_persist_stages(&emitter_result, &crawl_id_owned, outcomes, shared_outcomes.as_ref()).await;
        }

        let was_cancelled = cancelled.load(std::sync::atomic::Ordering::SeqCst);
        cancelled_flag.store(true, std::sync::atomic::Ordering::SeqCst);

        worker_loop.abort();
        collector.abort();

        let _ = worker_loop.await;
        let _ = collector.await;

        if was_cancelled {
            let _ = emit_log(&emitter, "warn", "local", &format!("Crawl cancelled: id={}", crawl_id));
        } else {
            let _ = emit_log(&emitter, "info", "local", &format!("Crawl completed: id={}, pages={}", crawl_id, results.len()));
        }

        Ok(results)
    }
}

async fn process_page(
    client: reqwest::Client,
    item: FrontierItem,
    config: &CrawlConfig,
    emitter: &CraspEmitter,
    crawl_id: &str,
) -> WorkerOutput {
    let url_str = item.url.clone();
    let depth = item.depth;

    let mut page = ArchivedPage {
        url: url_str.clone(),
        depth,
        status: PageStatus::Fetching,
        title: String::new(),
        content: None,
        hash: None,
        hash_algorithm: None,
        discovered_links: 0,
        timestamp: Utc::now().to_rfc3339(),
        crawl_id: Some(crawl_id.to_string()),
        extracted_title: None,
        author: None,
        published_date: None,
        excerpt: None,
        reading_time_minutes: None,
        body_text: None,
        body_html: None,
        assets: None,
        extraction_method: None,
        extraction_confidence: None,
        thin_content: None,
    };

    let _ = emitter.emit("scrape-progress", &serde_json::json!({
        "url": url_str,
        "status": "fetching",
        "depth": depth,
    })).await;

    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": url_str,
        "crawl_id": crawl_id,
        "stage": PageStage::Discovered,
    })).await;

    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": url_str,
        "crawl_id": crawl_id,
        "stage": PageStage::Fetching,
    })).await;

    let html_text = {
        let response = match client.get(&url_str).send().await {
            Ok(r) => r,
            Err(e) => {
                page.status = PageStatus::Failed(e.to_string());
                let _ = emit_log(emitter, "error", "local", &format!("Fetch failed for {}: {}", url_str, e));
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": url_str,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed { failed_stage: "fetching".to_string(), reason: e.to_string() },
                })).await;
                let _ = emitter.emit("archive-failed", &serde_json::to_value(&page).unwrap_or_default()).await;
                return WorkerOutput { page, found_links: vec![] };
            }
        };

        let initial_url = Url::parse(&url_str).unwrap_or_else(|_| Url::parse("http://invalid/").unwrap());
        let response = match crate::ssrf::follow_redirects(&client, initial_url, response).await {
            Ok(r) => r,
            Err(e) => {
                page.status = PageStatus::Failed(e.clone());
                let _ = emit_log(emitter, "error", "local", &format!("SSRF redirect rejected for {}: {}", url_str, e));
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": url_str,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed { failed_stage: "redirect".to_string(), reason: e.clone() },
                })).await;
                let _ = emitter.emit("archive-failed", &serde_json::to_value(&page).unwrap_or_default()).await;
                return WorkerOutput { page, found_links: vec![] };
            }
        };

        let status_code = response.status();
        if !status_code.is_success() {
            let reason = format!("HTTP {}", status_code);
            page.status = PageStatus::Failed(reason.clone());
            let _ = emit_log(emitter, "warn", "local", &format!("Non-success HTTP {} for {}", status_code, url_str));
            let _ = emitter.emit("page-stage", &serde_json::json!({
                "url": url_str,
                "crawl_id": crawl_id,
                "stage": PageStage::Failed { failed_stage: "fetching".to_string(), reason },
            })).await;
            let _ = emitter.emit("archive-failed", &serde_json::to_value(&page).unwrap_or_default()).await;
            return WorkerOutput { page, found_links: vec![] };
        }

        let code = status_code.as_u16();

        match response.text().await {
            Ok(t) => {
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": url_str,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Fetched { status_code: code },
                })).await;
                t
            }
            Err(e) => {
                page.status = PageStatus::Failed(e.to_string());
                let _ = emit_log(emitter, "error", "local", &format!("Body read failed for {}: {}", url_str, e));
                let _ = emitter.emit("page-stage", &serde_json::json!({
                    "url": url_str,
                    "crawl_id": crawl_id,
                    "stage": PageStage::Failed { failed_stage: "fetching".to_string(), reason: e.to_string() },
                })).await;
                let _ = emitter.emit("archive-failed", &serde_json::to_value(&page).unwrap_or_default()).await;
                return WorkerOutput { page, found_links: vec![] };
            }
        }
    };

    page.status = PageStatus::Scraping;

    let _ = emitter.emit("scrape-progress", &serde_json::json!({
        "url": url_str,
        "status": "scraping",
        "depth": depth,
    })).await;

    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": url_str,
        "crawl_id": crawl_id,
        "stage": PageStage::Parsing,
    })).await;

    let (title, links, extracted, extraction_result) = {
        let html_text = html_text.clone();
        let url_for_links = url_str.clone();
        let preserve_html = config.preserve_html;
        let base_url = url_str.clone();

        tokio::task::spawn_blocking(move || {
            let document = Html::parse_document(&html_text);
            let title = extract_title(&document);
            let links = extract_links(&document, &url_for_links);

            let extraction = crate::extraction::extract_main_content(&html_text, &base_url);
            let assets = crate::extraction::extract_assets(&html_text, &base_url, &extraction.body_html);

            let extracted = if preserve_html {
                extraction.body_html.clone()
            } else {
                extraction.body_text.clone()
            };

            (title, links, extracted, (extraction, assets))
        })
        .await
        .unwrap_or_else(|_| (String::new(), vec![], String::new(), {
            let er = crate::extraction::ExtractionResult::raw_fallback();
            (er, crate::schema::PageAssets::default())
        }))
    };

    let stage = if config.preserve_html { PageStage::Sanitizing } else { PageStage::Preserving };
    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": url_str,
        "crawl_id": crawl_id,
        "stage": stage,
    })).await;

    if title.is_empty() {
        page.title = url_str.clone();
    } else {
        page.title = title;
    }
    page.discovered_links = links.len() as u32;

    let (mut extraction, assets) = extraction_result;
    page.extracted_title = extraction.title.take();
    page.author = extraction.author.take();
    page.published_date = extraction.published_date.take();
    page.excerpt = extraction.excerpt.take();
    page.reading_time_minutes = if extraction.reading_time_minutes > 0 {
        Some(extraction.reading_time_minutes)
    } else {
        None
    };
    page.body_text = if !extraction.body_text.is_empty() {
        Some(std::mem::take(&mut extraction.body_text))
    } else {
        None
    };
    page.body_html = if !extraction.body_html.is_empty() {
        Some(std::mem::take(&mut extraction.body_html))
    } else {
        None
    };
    page.assets = Some(assets);
    page.extraction_method = Some(extraction.extraction_method().to_string());
    page.extraction_confidence = Some(extraction.confidence);
    let extraction_method_str = extraction.extraction_method().to_string();
    let extraction_confidence_val = extraction.confidence;
    let extraction_failed_flag = extraction.extraction_failed;
    let is_thin = extraction.thin_content || extraction_failed_flag;
    page.thin_content = Some(is_thin);
    let non_ws_count = if is_thin {
        extraction.body_text.chars().filter(|c| !c.is_whitespace()).count()
    } else {
        0
    };

    let _ = emit_log(emitter, "info", "local", &format!(
        "Extraction: {} → method={}, confidence={:.2}, thin={}",
        url_str, extraction_method_str, extraction_confidence_val, is_thin
    ));

    if is_thin || extraction_failed_flag {
        let reason = if extraction_failed_flag { "extraction failed" } else { "may require JS rendering" };
        let _ = emit_log(emitter, "warn", "local", &format!(
            "Thin content detected on {} ({} chars) — {}",
            url_str,
            non_ws_count,
            reason
        ));
    }

    let _ = emitter.emit("crawl-discover", &serde_json::json!({
        "url": url_str,
        "depth": depth,
        "link_count": links.len(),
    })).await;

    page.status = PageStatus::Archiving;

    let _ = emitter.emit("scrape-progress", &serde_json::json!({
        "url": url_str,
        "status": "archiving",
        "depth": depth,
    })).await;

    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": url_str,
        "crawl_id": crawl_id,
        "stage": PageStage::Hashing,
    })).await;

    let hash = compute_hash(&extracted, &config.hash_algorithm);
    page.hash = Some(hash);
    page.hash_algorithm = Some(match config.hash_algorithm {
        HashAlgorithm::Md5 => "md5".to_string(),
        HashAlgorithm::Sha256 => "sha256".to_string(),
    });
    page.content = Some(extracted);
    page.status = PageStatus::Completed;

    WorkerOutput {
        page,
        found_links: links,
    }
}

fn extract_title(document: &Html) -> String {
    Selector::parse("title")
        .ok()
        .and_then(|sel| {
            document
                .select(&sel)
                .filter_map(|el| el.text().next())
                .next()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default()
}

fn extract_links(document: &Html, base_url: &str) -> Vec<String> {
    let selector = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let base = Url::parse(base_url).ok();

    document
        .select(&selector)
        .filter_map(|el| {
            el.value().attr("href").and_then(|href| {
                if href.starts_with('#')
                    || href.starts_with("javascript:")
                    || href.starts_with("mailto:")
                {
                    return None;
                }
                if let Some(b) = &base {
                    b.join(href).ok().map(|u| u.to_string())
                } else {
                    Url::parse(href).ok().map(|u| u.to_string())
                }
            })
        })
        .collect()
}

pub fn select_content<'a>(document: &'a Html, selectors: &[String]) -> Vec<scraper::ElementRef<'a>> {
    for sel_str in selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            let els: Vec<_> = document.select(&selector).collect();
            if !els.is_empty() {
                return els;
            }
        }
    }

    for tag in &["article", "main", "body"] {
        if let Ok(selector) = Selector::parse(tag) {
            let els: Vec<_> = document.select(&selector).collect();
            if !els.is_empty() {
                return els;
            }
        }
    }

    vec![]
}

static STRIP_SET: phf::Set<&'static str> = phf_set! {
    "script", "style", "noscript", "iframe", "object", "embed",
    "applet", "form", "input", "button", "select", "textarea",
    "svg", "canvas",
};

static VOID_SET: phf::Set<&'static str> = phf_set! {
    "br", "hr", "img", "input", "meta", "link",
};

static TRACKING_SET: phf::Set<&'static str> = phf_set! {
    "onclick", "onload", "onerror", "onmouseover", "onmouseout",
    "onmousedown", "onmouseup", "onkeydown", "onkeyup", "onfocus",
    "onblur", "onsubmit", "onchange", "data-tracking", "data-ad",
    "data-analytics", "data-pixel",
};

struct TagSets {
    strip: &'static phf::Set<&'static str>,
    void_tags: &'static phf::Set<&'static str>,
    tracking: &'static phf::Set<&'static str>,
}

impl TagSets {
    fn global() -> &'static Self {
        static INSTANCE: OnceLock<TagSets> = OnceLock::new();
        INSTANCE.get_or_init(|| TagSets {
            strip: &STRIP_SET,
            void_tags: &VOID_SET,
            tracking: &TRACKING_SET,
        })
    }

    fn is_strip(&self, tag: &str) -> bool {
        self.strip.contains(tag)
    }

    fn is_void(&self, tag: &str) -> bool {
        self.void_tags.contains(tag)
    }

    fn is_tracking_attr(&self, attr: &str) -> bool {
        self.tracking.contains(attr) || attr.starts_with("on")
    }
}

pub fn sanitize_to_html(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::global();
    let mut output = String::with_capacity(8192);

    for el in elements {
        for child_node_ref in el.children() {
            render_node_html_into(&child_node_ref, sets, &mut output);
        }
    }

    output
}

fn render_node_html_into(node_ref: &NodeRef<Node>, sets: &TagSets, out: &mut String) {
    match node_ref.value() {
        Node::Text(t) => {
            html_escape_into(&t.text, out);
        }
        Node::Element(el) => {
            let tag = el.name.local.as_ref();

            if sets.is_strip(tag) {
                return;
            }

            out.push('<');
            out.push_str(tag);

            for (k, v) in el.attrs.iter() {
                if sets.is_tracking_attr(k.local.as_ref()) || k.local.as_ref() == "style" {
                    continue;
                }
                out.push(' ');
                out.push_str(k.local.as_ref());
                out.push_str("=\"");
                html_escape_into(v, out);
                out.push('"');
            }

            if sets.is_void(tag) {
                out.push_str(" />");
                return;
            }

            out.push('>');

            for c in node_ref.children() {
                render_node_html_into(&c, sets, out);
            }

            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        Node::Comment(_) => {}
        _ => {}
    }
}

pub fn sanitize_to_text(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::global();
    let mut parts = Vec::new();

    for el in elements {
        let mut buf = String::with_capacity(2048);
        for child_node_ref in el.children() {
            collect_text_recursive_into(&child_node_ref, sets, &mut buf);
        }
        let trimmed = buf.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    parts.join("\n\n")
}

fn collect_text_recursive_into(node_ref: &NodeRef<Node>, sets: &TagSets, out: &mut String) {
    match node_ref.value() {
        Node::Text(t) => {
            out.push_str(&t.text);
        }
        Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if sets.is_strip(tag) {
                return;
            }

            let is_block = matches!(
                tag,
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "div" | "li" | "blockquote" | "pre" | "section"
                    | "article" | "main" | "header" | "footer" | "tr"
            );

            let start_len = out.len();

            for c in node_ref.children() {
                collect_text_recursive_into(&c, sets, out);
            }

            if is_block {
                let inner = &out[start_len..];
                let trimmed = inner.trim();
                if trimmed.is_empty() {
                    out.truncate(start_len);
                } else {
                    // Clone trimmed to release the immutable borrow on `out`,
                    // then truncate and rewrite. This allocates once per block
                    // element — a small cost compared to the O(n²) avoided
                    // by not returning Strings from every recursive call.
                    let trimmed_owned = trimmed.to_string();
                    out.truncate(start_len);
                    out.push('\n');
                    out.push_str(&trimmed_owned);
                    out.push('\n');
                }
            }
        }
        Node::Comment(_) => {}
        _ => {}
    }
}

fn html_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&"),
            '<' => out.push_str("<"),
            '>' => out.push_str(">"),
            '"' => out.push_str("&#34;"),
            _ => out.push(c),
        }
    }
}

fn compute_hash(content: &str, algorithm: &HashAlgorithm) -> String {
    match algorithm {
        HashAlgorithm::Md5 => {
            let mut hasher = Md5::new();
            hasher.update(content.as_bytes());
            hex::encode(hasher.finalize())
        }
        HashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            hex::encode(hasher.finalize())
        }
    }
}

fn normalize_url(url: &str) -> String {
    let mut parsed = match Url::parse(url) {
        Ok(u) => u,
        Err(_) => return url.to_lowercase(),
    };
    parsed.set_fragment(None);
    let mut normalized = parsed.to_string();
    if normalized.ends_with('/') {
        normalized.pop();
    }
    normalized.to_lowercase()
}
