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

use crate::commands::TauriEmitter;

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

pub struct Crawler {
    client: reqwest::Client,
}

impl Crawler {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Crasp/0.1 (archiver; +https://crasp.app)")
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");

        Self { client }
    }

    pub async fn run(
        self,
        config: CrawlConfig,
        emitter: TauriEmitter,
        control: &CrawlControl,
    ) -> Result<Vec<ArchivedPage>, String> {
        let seed = Url::parse(&config.seed_url).map_err(|e| e.to_string())?;
        let domain = seed.domain().ok_or("seed URL has no domain")?.to_string();
        let scheme = seed.scheme().to_string();

        let (work_tx, work_rx) = mpsc::channel::<FrontierItem>(config.max_pages as usize * 2);
        let (result_tx, mut result_rx) = mpsc::channel::<ArchivedPage>(config.max_pages as usize * 2);

        let visited = Arc::new(DashSet::<String>::new());
        let pages_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        {
            let key = normalize_url(&config.seed_url);
            visited.insert(key);
        }
        pages_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let _ = work_tx.send(FrontierItem {
            url: config.seed_url.clone(),
            depth: 0,
            parent: None,
        }).await;

        let client = self.client;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency));
        let cancelled_flag = cancelled.clone();

        let (done_tx, mut done_rx) = mpsc::channel::<WorkerOutput>(1000);

        let cfg_worker = config.clone();
        let emitter_worker = emitter.clone();

        let mut worker_cancel_rx = control.cancel_rx();
        let worker_pause_rx = control.pause_rx();

        let worker_loop = {
            let done_tx = done_tx.clone();
            let semaphore = semaphore.clone();
            let cancelled_flag = cancelled_flag.clone();

            tokio::spawn(async move {
                let mut rx = work_rx;
                loop {
                    let item = tokio::select! {
                        item = rx.recv() => {
                            match item {
                                Some(i) => i,
                                None => break,
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

                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };

                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }

                    let mut pause_rx = worker_pause_rx.clone();
                    if *pause_rx.borrow() {
                        loop {
                            tokio::select! {
                                _ = pause_rx.changed() => {
                                    if !*pause_rx.borrow() {
                                        break;
                                    }
                                }
                                _ = tokio::task::yield_now() => {
                                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                        break;
                                    }
                                    if !*pause_rx.borrow() {
                                        break;
                                    }
                                }
                            }
                        }
                        if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                            break;
                        }
                    }

                    if cancelled_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }

                    let c = client.clone();
                    let cf = cfg_worker.clone();
                    let em = emitter_worker.clone();
                    let tx = done_tx.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        let output = process_page(c, item, &cf, &em).await;
                        let _ = tx.send(output).await;
                    });
                }
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

                        let should_inject = {
                            let current = i_pages_count.load(std::sync::atomic::Ordering::SeqCst);
                            if current >= i_max_pages {
                                continue;
                            }
                            i_visited.insert(key)
                        };

                        if should_inject {
                            i_pages_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

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

        drop(work_tx);
        drop(done_tx);

        let mut results: Vec<ArchivedPage> = Vec::new();
        let emitter_result = emitter.clone();
        let mut cancel_rx = control.cancel_rx();
        let mut pause_rx = control.pause_rx();

        loop {
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
                            results.push(p);
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

        cancelled_flag.store(true, std::sync::atomic::Ordering::SeqCst);

        worker_loop.abort();
        collector.abort();

        let _ = worker_loop.await;
        let _ = collector.await;

        let _ = emitter.emit("crawl-done", &serde_json::json!({
            "pages_archived": results.len(),
            "cancelled": cancelled.load(std::sync::atomic::Ordering::SeqCst),
        })).await;

        Ok(results)
    }
}

async fn process_page(
    client: reqwest::Client,
    item: FrontierItem,
    config: &CrawlConfig,
    emitter: &TauriEmitter,
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
    };

    let _ = emitter.emit("scrape-progress", &serde_json::json!({
        "url": url_str,
        "status": "fetching",
        "depth": depth,
    })).await;

    let html_text = {
        let response = match client.get(&url_str).send().await {
            Ok(r) => r,
            Err(e) => {
                page.status = PageStatus::Failed(e.to_string());
                return WorkerOutput { page, found_links: vec![] };
            }
        };

        let status_code = response.status();
        if !status_code.is_success() {
            page.status = PageStatus::Failed(format!("HTTP {}", status_code));
            return WorkerOutput { page, found_links: vec![] };
        }

        match response.text().await {
            Ok(t) => t,
            Err(e) => {
                page.status = PageStatus::Failed(e.to_string());
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

    let (title, links, extracted) = {
        let html_text = html_text.clone();
        let url_for_links = url_str.clone();
        let css_selectors = config.css_selectors.clone();
        let preserve_html = config.preserve_html;

        tokio::task::spawn_blocking(move || {
            let document = Html::parse_document(&html_text);
            let title = extract_title(&document);
            let links = extract_links(&document, &url_for_links);
            let extracted = extract_and_sanitize(&document, &css_selectors, preserve_html);
            (title, links, extracted)
        })
        .await
        .unwrap_or_else(|_| (String::new(), vec![], String::new()))
    };

    if title.is_empty() {
        page.title = url_str.clone();
    } else {
        page.title = title;
    }
    page.discovered_links = links.len() as u32;

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

fn extract_and_sanitize(
    document: &Html,
    selectors: &[String],
    preserve_html: bool,
) -> String {
    let matched = select_content(document, selectors);

    if matched.is_empty() {
        return String::new();
    }

    if preserve_html {
        sanitize_to_html(&matched)
    } else {
        sanitize_to_text(&matched)
    }
}

fn select_content<'a>(document: &'a Html, selectors: &[String]) -> Vec<scraper::ElementRef<'a>> {
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

fn sanitize_to_html(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::global();
    let mut output = String::new();

    for el in elements {
        for child_node_ref in el.children() {
            output.push_str(&render_node_html(&child_node_ref, &sets));
        }
    }

    output
}

fn render_node_html(node_ref: &NodeRef<Node>, sets: &TagSets) -> String {
    match node_ref.value() {
        Node::Text(t) => html_escape(&t.text),
        Node::Element(el) => {
            let tag = el.name.local.as_ref();

            if sets.is_strip(tag) {
                return String::new();
            }

            let attrs: String = el
                .attrs
                .iter()
                .filter(|(k, _)| !sets.is_tracking_attr(k.local.as_ref()) && k.local.as_ref() != "style")
                .map(|(k, v)| format!(" {}=\"{}\"", k.local, html_escape(v)))
                .collect();

            if sets.is_void(tag) {
                return format!("<{}{} />", tag, attrs);
            }

            let inner: String = node_ref.children().map(|c| render_node_html(&c, sets)).collect();

            format!("<{}{}>{}</{}>", tag, attrs, inner, tag)
        }
        Node::Comment(_) => String::new(),
        _ => String::new(),
    }
}

fn sanitize_to_text(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::global();
    let mut output = Vec::new();

    for el in elements {
        let text = collect_text_from_element(el, &sets);
        if !text.trim().is_empty() {
            output.push(text.trim().to_string());
        }
    }

    output.join("\n\n")
}

fn collect_text_from_element(el: &scraper::ElementRef, sets: &TagSets) -> String {
    let mut result = String::new();
    for child_node_ref in el.children() {
        result.push_str(&collect_text_recursive(&child_node_ref, sets));
    }
    result
}

fn collect_text_recursive(node_ref: &NodeRef<Node>, sets: &TagSets) -> String {
    match node_ref.value() {
        Node::Text(t) => t.text.to_string(),
        Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if sets.is_strip(tag) {
                return String::new();
            }

            let inner: String = node_ref
                .children()
                .map(|c| collect_text_recursive(&c, sets))
                .collect();

            let is_block = matches!(
                tag,
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "div" | "li" | "blockquote" | "pre" | "section"
                    | "article" | "main" | "header" | "footer" | "tr"
            );

            if is_block {
                let trimmed = inner.trim();
                if trimmed.is_empty() {
                    String::new()
                } else {
                    format!("\n{}\n", trimmed)
                }
            } else {
                inner
            }
        }
        Node::Comment(_) => String::new(),
        _ => String::new(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&")
        .replace('<', "<")
        .replace('>', ">")
        .replace('"', "&#34;")
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
