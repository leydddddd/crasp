use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use md5::{Digest as Md5Digest, Md5};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};
use tokio::sync::{Semaphore, watch};
use url::Url;

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

struct FrontierItem {
    url: String,
    depth: u32,
    parent: Option<String>,
}

struct WorkerOutput {
    page: ArchivedPage,
    found_links: Vec<String>,
}

pub struct Crawler {
    client: reqwest::Client,
    cancel_tx: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    pause_tx: watch::Sender<bool>,
    pause_rx: watch::Receiver<bool>,
    visited: Arc<tokio::sync::Mutex<HashSet<String>>>,
    pages_count: Arc<std::sync::atomic::AtomicU32>,
    paused: Arc<std::sync::atomic::AtomicBool>,
}

impl Crawler {
    pub fn new() -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (pause_tx, pause_rx) = watch::channel(false);

        let client = reqwest::Client::builder()
            .user_agent("SiteVault/0.1 (archiver; +https://sitevault.app)")
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            cancel_tx,
            cancel_rx,
            pause_tx,
            pause_rx,
            visited: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            pages_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
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

    pub async fn run(
        &self,
        config: CrawlConfig,
        emitter: TauriEmitter,
    ) -> Result<Vec<ArchivedPage>, String> {
        let seed = Url::parse(&config.seed_url).map_err(|e| e.to_string())?;
        let domain = seed.domain().ok_or("seed URL has no domain")?.to_string();
        let scheme = seed.scheme().to_string();

        self.reset_state().await;

        let (work_tx, work_rx) = tokio::sync::mpsc::channel::<FrontierItem>(config.max_pages as usize * 2);
        let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<ArchivedPage>(config.max_pages as usize * 2);

        {
            let key = normalize_url(&config.seed_url);
            let mut v = self.visited.lock().await;
            v.insert(key);
        }
        self.pages_count.store(1, std::sync::atomic::Ordering::SeqCst);

        let _ = work_tx.send(FrontierItem {
            url: config.seed_url.clone(),
            depth: 0,
            parent: None,
        }).await;

        let semaphore = Arc::new(Semaphore::new(config.concurrency));
        let client = self.client.clone();
        let cfg = config.clone();
        let visited = self.visited.clone();
        let pages_count = self.pages_count.clone();
        let paused = self.paused.clone();
        let cancel_rx = self.cancel_rx.clone();
        let pause_rx = self.pause_rx.clone();

        let worker_handle = tokio::spawn(async move {
            let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<WorkerOutput>(1000);

            let w_client = client.clone();
            let w_cfg = cfg.clone();
            let w_emitter = emitter.clone();
            let w_done_tx = done_tx.clone();
            let w_semaphore = semaphore.clone();

            let worker_loop = tokio::spawn(async move {
                let mut rx = work_rx;
                while let Some(item) = rx.recv().await {
                    let permit = match w_semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    let c = w_client.clone();
                    let cf = w_cfg.clone();
                    let em = w_emitter.clone();
                    let tx = w_done_tx.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        let output = process_page(c, item, &cf, &em).await;
                        let _ = tx.send(output).await;
                    });
                }
            });

            let inject_tx = work_tx.clone();
            let i_visited = visited.clone();
            let i_pages_count = pages_count.clone();
            let i_max_pages = cfg.max_pages;
            let i_max_depth = cfg.max_depth;
            let i_domain = domain.clone();
            let i_scheme = scheme.clone();
            let i_emitter = emitter.clone();
            let i_paused = paused.clone();

            let collector = tokio::spawn(async move {
                while let Some(output) = done_rx.recv().await {
                    let archived = output.page.clone();

                    if *i_paused.load(std::sync::atomic::Ordering::SeqCst) {
                        let _ = result_tx.send(archived).await;
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
                            let fragless = parsed.clone().fragment(None).to_string();
                            let key = normalize_url(&fragless);
                            {
                                let mut v = i_visited.lock().await;
                                if v.contains(&key) {
                                    continue;
                                }
                                let current = i_pages_count.load(std::sync::atomic::Ordering::SeqCst);
                                if current >= i_max_pages {
                                    continue;
                                }
                                v.insert(key);
                            }
                            i_pages_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                            let depth = archived.depth + 1;
                            if depth > i_max_depth {
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

                    let _ = result_tx.send(archived).await;
                }
            });

            (worker_loop, collector)
        });

        let mut results: Vec<ArchivedPage> = Vec::new();
        let emitter_e = emitter.clone();
        let mut cancel = self.cancel_rx.clone();
        let mut pause = self.pause_rx.clone();
        let p_flag = self.paused.clone();

        loop {
            tokio::select! {
                page = result_rx.recv() => {
                    match page {
                        Some(p) => {
                            let _ = emitter_e.emit("archive-success", &serde_json::to_value(&p).unwrap_or_default()).await;
                            results.push(p);
                        }
                        None => break,
                    }
                }
                _ = cancel.changed() => {
                    if *cancel.borrow() {
                        break;
                    }
                }
                _ = pause.changed() => {
                    if *pause.borrow() {
                        loop {
                            tokio::select! {
                                res = result_rx.recv() => {
                                    if res.is_none() {
                                        break;
                                    }
                                }
                                _ = cancel.changed() => {
                                    if *cancel.borrow() {
                                        break;
                                    }
                                }
                                _ = pause.changed() => {
                                    if !*p_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                        break;
                                    }
                                }
                                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                                    if !*p_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        worker_handle.abort();
        Ok(results)
    }

    async fn reset_state(&self) {
        self.visited.lock().await.clear();
        self.pages_count.store(0, std::sync::atomic::Ordering::SeqCst);
        self.paused.store(false, std::sync::atomic::Ordering::SeqCst);
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

    let html_text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            page.status = PageStatus::Failed(e.to_string());
            return WorkerOutput { page, found_links: vec![] };
        }
    };

    page.status = PageStatus::Scraping;

    let _ = emitter.emit("scrape-progress", &serde_json::json!({
        "url": url_str,
        "status": "scraping",
        "depth": depth,
    })).await;

    let document = Html::parse_document(&html_text);

    page.title = extract_title(&document);
    if page.title.is_empty() {
        page.title = url_str.clone();
    }

    let links = extract_links(&document, &url_str);
    page.discovered_links = links.len() as u32;

    let _ = emitter.emit("crawl-discover", &serde_json::json!({
        "url": url_str,
        "depth": depth,
        "link_count": links.len(),
    })).await;

    let extracted = extract_and_sanitize(&document, &config.css_selectors, config.preserve_html);

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

static STRIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "iframe", "object", "embed",
    "applet", "form", "input", "button", "select", "textarea",
    "svg", "canvas",
];

static STRUCTURAL_TAGS: &[&str] = &[
    "div", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li", "table", "thead", "tbody", "tr", "th", "td",
    "a", "img", "figure", "figcaption", "blockquote", "pre", "code",
    "span", "strong", "em", "b", "i", "u", "br", "hr",
    "section", "article", "main", "header", "footer", "nav", "aside",
    "dl", "dt", "dd", "abbr", "cite", "time", "mark", "summary", "details",
];

static TRACKING_ATTRS: &[&str] = &[
    "onclick", "onload", "onerror", "onmouseover", "onmouseout",
    "onmousedown", "onmouseup", "onkeydown", "onkeyup", "onfocus",
    "onblur", "onsubmit", "onchange", "data-tracking", "data-ad",
    "data-analytics", "data-pixel",
];

struct TagSets {
    strip: HashSet<&'static str>,
    structural: HashSet<&'static str>,
    tracking: HashSet<&'static str>,
}

impl TagSets {
    fn new() -> Self {
        Self {
            strip: STRIP_TAGS.iter().copied().collect(),
            structural: STRUCTURAL_TAGS.iter().copied().collect(),
            tracking: TRACKING_ATTRS.iter().copied().collect(),
        }
    }
}

fn is_strip_tag(tag: &str, sets: &TagSets) -> bool {
    sets.strip.contains(tag)
}

fn is_structural(tag: &str, sets: &TagSets) -> bool {
    sets.structural.contains(tag)
}

fn is_tracking_attr(attr: &str, sets: &TagSets) -> bool {
    sets.tracking.contains(attr) || attr.starts_with("on")
}

fn sanitize_to_html(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::new();
    let mut output = String::new();

    for el in elements {
        let node_id = el.id();
        for child in el.children() {
            output.push_str(&render_node_html(child, &sets));
        }
    }

    output
}

fn render_node_html(node: scraper::Node, sets: &TagSets) -> String {
    match node.value() {
        scraper::Node::Text(t) => html_escape(&t.text),
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();

            if is_strip_tag(tag, sets) {
                return String::new();
            }

            let attrs: String = el
                .attrs
                .iter()
                .filter(|(k, _)| !is_tracking_attr(k.local.as_ref(), sets) && k.local.as_ref() != "style")
                .map(|(k, v)| format!(" {}=\"{}\"", k.local, html_escape(v)))
                .collect();

            let inner: String = node.children().map(|c| render_node_html(c, sets)).collect();

            if is_structural(tag, sets) {
                format!("<{}{}>{}</{}>", tag, attrs, inner, tag)
            } else {
                format!("<{}{}>{}</{}>", tag, attrs, inner, tag)
            }
        }
        scraper::Node::Comment(_) => String::new(),
        _ => String::new(),
    }
}

fn sanitize_to_text(elements: &[scraper::ElementRef]) -> String {
    let sets = TagSets::new();
    let mut output = Vec::new();

    for el in elements {
        let text = collect_text(el, &sets);
        if !text.trim().is_empty() {
            output.push(text.trim().to_string());
        }
    }

    output.join("\n\n")
}

fn collect_text(node: scraper::ElementRef, sets: &TagSets) -> String {
    let mut result = String::new();

    for child in node.children() {
        result.push_str(&collect_text_recursive(child, sets));
    }

    result
}

fn collect_text_recursive(node: scraper::Node, sets: &TagSets) -> String {
    match node.value() {
        scraper::Node::Text(t) => t.text.clone(),
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if is_strip_tag(tag, sets) {
                return String::new();
            }

            let inner: String = node
                .children()
                .map(|c| collect_text_recursive(c, sets))
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
        scraper::Node::Comment(_) => String::new(),
        _ => String::new(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&")
        .replace('<', "<")
        .replace('>', ">")
        .replace('"', """)
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
