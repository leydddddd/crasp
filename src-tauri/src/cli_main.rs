use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;

use crasp_lib::crawler::{CrawlConfig, CrawlControl, Crawler, HashAlgorithm};
use crasp_lib::export::{ExportContent, ExportFormat, ExportRequest, ExportScope};
use crasp_lib::progress::{CraspEmitter, CliEmitter, CrawlProgressEvent};

#[derive(Parser)]
#[command(
    name = "crasp-cli",
    version,
    about = "Crasp web archiver — headless CLI",
    long_about = None
)]
struct Cli {
    #[arg(long, global = true, help = "MongoDB URI (overrides CRASP_MONGO_URI env var)")]
    mongo_uri: Option<String>,

    #[arg(short, long, action = clap::ArgAction::Count, global = true, help = "Output verbosity: -v for info, -vv for debug")]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Crawl a website and archive its pages")]
    Crawl {
        #[arg(help = "Seed URL to start crawling from")]
        url: String,

        #[arg(long, default_value = "100", help = "Maximum pages to archive")]
        max_pages: u32,

        #[arg(long, default_value = "3", help = "Maximum crawl depth")]
        max_depth: u32,

        #[arg(long, default_value = "4", help = "Concurrency (simultaneous fetches)")]
        concurrency: u32,

        #[arg(long, default_value = "article,main,body", help = "CSS selectors for content extraction (comma-separated)")]
        selectors: String,

        #[arg(long, default_value = "false", help = "Preserve raw HTML instead of sanitizing")]
        preserve_html: bool,

        #[arg(long, default_value = "sha256", help = "Hash algorithm: md5 or sha256")]
        hash_algorithm: String,

        #[arg(long, help = "Output .jl file path (overrides MongoDB; if omitted, uses app data dir)")]
        output: Option<String>,

        #[arg(long, default_value = "local", help = "Engine: local, local-scrapy (cloud requires GUI for now)")]
        engine: String,
    },

    #[command(about = "List archived crawls and pages")]
    List {
        #[arg(help = "Crawl ID to list pages for (omit to list all crawls)")]
        crawl_id: Option<String>,

        #[arg(long, default_value = "table", help = "Output format: table (default), json, csv")]
        format: String,

        #[arg(long, help = "Filter by status: completed, failed, skipped")]
        status: Option<String>,

        #[arg(long, help = "Show thin-content pages only")]
        thin_only: bool,
    },

    #[command(about = "Export archived content to a file or folder")]
    Export {
        #[arg(long, help = "Crawl ID to export (required for whole-crawl scopes)")]
        crawl_id: Option<String>,

        #[arg(long, help = "Page URL to export (required for single-page scope)")]
        url: Option<String>,

        #[arg(long, default_value = "md", help = "Output format: txt, md, html, epub")]
        format: String,

        #[arg(long, default_value = "one-file", help = "Scope: page, one-file, folder")]
        scope: String,

        #[arg(long, default_value = "with-metadata", help = "Content level: content-only, with-metadata, with-assets, full")]
        content: String,

        #[arg(long, help = "Output path (file or folder; defaults to ./exports/)")]
        output: Option<String>,
    },

    #[command(about = "Test connectivity to MongoDB or Zyte")]
    Status {
        #[arg(long, help = "Test MongoDB connection")]
        mongo: bool,

        #[arg(long, help = "Test Zyte connection")]
        zyte: bool,
    },

    #[command(about = "Show app data directory path and its contents")]
    DataDir,

    #[command(about = "Validate a URL (SSRF check + URL parse check)")]
    Validate {
        url: String,
    },

    #[command(about = "Deep fetch a single URL using Zyte browser rendering")]
    DeepFetch {
        #[arg(long, help = "URL to deep fetch")]
        url: String,

        #[arg(long, help = "Crawl ID to update the page record in")]
        crawl_id: Option<String>,
    },

    #[command(about = "Test Zyte API browser rendering access")]
    TestZyteApi {
        #[arg(long, default_value = "https://example.com", help = "URL to test with")]
        url: String,
    },
}

fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.crasp.devs")
}

fn build_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime")
}

async fn build_app_context(mongo_uri_override: Option<String>) -> Arc<crasp_lib::runtime::AppContext> {
    if let Some(uri) = mongo_uri_override {
        if !uri.is_empty() {
            std::env::set_var("CRASP_MONGO_URI", &uri);
        }
    }

    match crasp_lib::runtime::AppContext::from_env().await {
        Ok(ctx) => Arc::new(ctx),
        Err(_) => Arc::new(crasp_lib::runtime::AppContext::degraded()),
    }
}

fn parse_export_format(s: &str) -> Result<ExportFormat, String> {
    match s {
        "txt" | "text" | "plain" => Ok(ExportFormat::PlainText),
        "md" | "markdown" => Ok(ExportFormat::Markdown),
        "html" => Ok(ExportFormat::Html),
        "epub" => Ok(ExportFormat::Epub),
        other => Err(format!("Unknown export format: '{}'. Use: txt, md, html, epub", other)),
    }
}

fn parse_export_scope(s: &str) -> Result<ExportScope, String> {
    match s {
        "page" | "single-page" => Ok(ExportScope::SinglePage),
        "one-file" => Ok(ExportScope::WholeCrawlOneFile),
        "folder" => Ok(ExportScope::WholeCrawlFolder),
        other => Err(format!("Unknown export scope: '{}'. Use: page, one-file, folder", other)),
    }
}

fn parse_export_content(s: &str) -> Result<ExportContent, String> {
    match s {
        "content-only" => Ok(ExportContent::ContentOnly),
        "with-metadata" => Ok(ExportContent::WithMetadata),
        "with-assets" => Ok(ExportContent::WithAssets),
        "full" => Ok(ExportContent::Full),
        other => Err(format!("Unknown content level: '{}'. Use: content-only, with-metadata, with-assets, full", other)),
    }
}

fn parse_hash_algorithm(s: &str) -> Result<HashAlgorithm, String> {
    match s.to_lowercase().as_str() {
        "md5" => Ok(HashAlgorithm::Md5),
        "sha256" => Ok(HashAlgorithm::Sha256),
        other => Err(format!("Unknown hash algorithm: '{}'. Use: md5, sha256", other)),
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn run_crawl(
    cli: &Cli,
    url: &str,
    max_pages: u32,
    max_depth: u32,
    concurrency: u32,
    selectors: &str,
    preserve_html: bool,
    hash_algorithm: &str,
    output: Option<&str>,
    engine: &str,
) -> i32 {
    if let Err(e) = crasp_lib::ssrf::validate_seed_url(url).await {
        eprintln!("{} Rejected — {}", "✗".red().bold(), e);
        return 1;
    }

    let hash_algo = match parse_hash_algorithm(hash_algorithm) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{} {}", "✗".red().bold(), e);
            return 1;
        }
    };

    let config = CrawlConfig {
        seed_url: url.to_string(),
        max_depth,
        max_pages,
        concurrency: concurrency as usize,
        css_selectors: selectors.split(',').map(|s| s.trim().to_string()).collect(),
        preserve_html,
        hash_algorithm: hash_algo,
    };

    let ctx = build_app_context(cli.mongo_uri.clone()).await;
    let mongo_available = ctx.store.is_some();
    if !mongo_available {
        eprintln!("{}", "MongoDB not available — archiving to local file only".yellow());
    }

    let data_dir = app_data_dir();
    let app_data_dir_str = if let Some(ref out) = output {
        let p = PathBuf::from(out);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        p.parent().unwrap_or(&p).to_string_lossy().to_string()
    } else {
        let _ = std::fs::create_dir_all(&data_dir);
        data_dir.to_string_lossy().to_string()
    };

    let crawl_id = format!("crawl_{}", chrono::Utc::now().timestamp_millis());

    match engine {
        "local" => {}
        "local-scrapy" => {}
        other => {
            eprintln!("{} Unknown engine: '{}'. Use: local, local-scrapy", "✗".red().bold(), other);
            return 1;
        }
    }

    let (cli_emitter, mut progress_rx) = CliEmitter::new();

    let control = Arc::new(CrawlControl::new());

    let start_time = Instant::now();

    let max_pages_display = max_pages;
    let progress_task = tokio::spawn(async move {
        let mut pages_seen: u64 = 0;
        loop {
            match progress_rx.recv().await {
                Some(CrawlProgressEvent::Log { level, message, .. }) => {
                    if level == "error" {
                        eprintln!("  {} {}", "⚠".red(), message);
                    } else if level == "warn" {
                        eprintln!("  {} {}", "⚡".yellow(), message);
                    }
                }
                Some(CrawlProgressEvent::PageDone(page)) => {
                    pages_seen += 1;
                    let status_str = match page.status.as_str() {
                        "Completed" if page.thin_content => format!("[{:>3}/{}] ~ {}", pages_seen, max_pages_display, page.url),
                        "Completed" => format!("[{:>3}/{}] ✓ {}", pages_seen, max_pages_display, page.url),
                        "Failed" => {
                            let reason = page.status_reason.as_deref().unwrap_or("unknown");
                            format!("[{:>3}/{}] ✗ {} ({})", pages_seen, max_pages_display, page.url, reason)
                        }
                        "Skipped" => format!("[{:>3}/{}] ⊘ {}", pages_seen, max_pages_display, page.url),
                        other => format!("[{:>3}/{}] ? {} [{}]", pages_seen, max_pages_display, page.url, other),
                    };
                    let mut extra = if page.chars > 0 { format!(" ({} chars)", page.chars) } else { String::new() };
                    if page.thin_content && page.status == "Completed" {
                        extra.push_str(" — thin content");
                    }
                    if let Some(ref hash) = page.hash {
                        let trunc = &hash[..8.min(hash.len())];
                        extra.push_str(&format!(", sha256:{}...", trunc));
                    }
                    if page.thin_content && page.status == "Completed" {
                        eprintln!("{}{}", status_str.yellow(), extra);
                    } else if page.status == "Failed" {
                        eprintln!("{}{}", status_str.red().bold(), extra);
                    } else {
                        eprintln!("{}{}", status_str.green(), extra);
                    }
                }
                Some(CrawlProgressEvent::Discover { .. }) => {}
                None => break,
            }
        }
    });

    let emitter = CraspEmitter::Cli(cli_emitter);

    let result = if engine == "local-scrapy" {
        let out_jl = format!("{}/crasp_{}.jl", app_data_dir_str, chrono::Utc::now().timestamp());
        crasp_lib::local_scrapy::run_local_spider_streaming(
            &config,
            &out_jl,
            &emitter,
            &control,
            &ctx,
            &crawl_id,
            &app_data_dir_str,
            None,
        ).await
        .map(|count| {
            (0u64..count).map(|_| crasp_lib::crawler::ArchivedPage {
                url: String::new(),
                depth: 0,
                status: crasp_lib::crawler::PageStatus::Completed,
                title: String::new(),
                content: None,
                hash: None,
                hash_algorithm: None,
                discovered_links: 0,
                timestamp: String::new(),
                crawl_id: Some(crawl_id.clone()),
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
                deep_fetched: None,
            }).collect::<Vec<_>>()
        })
    } else {
        let crawler = Crawler::new();
        crawler.run(
            config,
            emitter,
            &control,
            &crawl_id,
            &ctx,
            &app_data_dir_str,
            None,
        ).await
    };

    progress_task.abort();

    let duration = start_time.elapsed();
    let duration_str = format!("{:.1}s", duration.as_secs_f64());

    match result {
        Ok(pages) => {
            let total = pages.len();
            let failed_pages = pages.iter().filter(|p| matches!(p.status, crasp_lib::crawler::PageStatus::Failed(_))).count();
            let skipped_pages = pages.iter().filter(|p| matches!(p.status, crasp_lib::crawler::PageStatus::Skipped(_))).count();
            let thin_pages = pages.iter().filter(|p| p.thin_content == Some(true)).count();
            let completed_pages = total - failed_pages - skipped_pages;

            let jl_path = {
                let dir = PathBuf::from(&app_data_dir_str);
                dir.join(format!("crawl-{}.jl", crawl_id))
            };
            let data_path = if jl_path.exists() {
                jl_path.to_string_lossy().to_string()
            } else if mongo_available && ctx.store.is_some() {
                "MongoDB".to_string()
            } else {
                jl_path.to_string_lossy().to_string()
            };

            println!();
            println!("{} — crawl_id: {}", "Crawl complete".green().bold(), crawl_id);
            println!("Pages: {} completed, {} failed, {} skipped ({} discovered)",
                completed_pages, failed_pages, skipped_pages, total);
            if thin_pages > 0 {
                println!("Thin content: {} pages (may need JS rendering)", thin_pages);
            }
            println!("Data: {}", data_path);
            println!("Duration: {}", duration_str);
            0
        }
        Err(e) => {
            eprintln!("{} Crawl failed: {}", "✗".red().bold(), e);
            1
        }
    }
}

async fn run_list(
    cli: &Cli,
    crawl_id: Option<&str>,
    format: &str,
    status_filter: Option<&str>,
    thin_only: bool,
) -> i32 {
    let ctx = build_app_context(cli.mongo_uri.clone()).await;
    let data_dir = app_data_dir();

    let mut pages: Vec<crasp_lib::commands::PageSummary> = Vec::new();

    if let Some(store) = &ctx.store {
        let mut filter = mongodb::bson::Document::new();
        if let Some(cid) = crawl_id {
            filter.insert("crawl_id", cid.to_string());
        }
        if let Ok(mut cursor) = store.pages_col().find(filter).await {
            use futures_util::TryStreamExt;
            while let Ok(Some(doc)) = cursor.try_next().await {
                let content_preview = if !doc.content.is_empty() {
                    Some(doc.content.chars().take(500).collect())
                } else {
                    None
                };
                pages.push(crasp_lib::commands::PageSummary {
                    url: doc.url.clone(),
                    title: doc.title.clone(),
                    depth: doc.depth,
                    stage: doc.status.clone(),
                    status_reason: doc.status_reason.clone(),
                    content_size: doc.content.len(),
                    timestamp: doc.timestamp.clone(),
                    source: crasp_lib::commands::StorageSource::Mongo,
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
    }

    if pages.is_empty() || crawl_id.is_some() {
        let crawl_id_filter = crawl_id.map(String::from);
        let local_pages: Vec<crasp_lib::commands::PageSummary> = {
            let dir = data_dir.clone();
            tokio::task::spawn_blocking(move || {
                let read_dir = match std::fs::read_dir(&dir) {
                    Ok(rd) => rd,
                    Err(_) => return Vec::new(),
                };
                let mut all = Vec::new();
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if !filename.starts_with("crawl-") || !filename.ends_with(".jl") { continue; }
                    if let Some(ref cid) = crawl_id_filter {
                        let file_cid = filename.strip_prefix("crawl-").unwrap_or("").strip_suffix(".jl").unwrap_or("");
                        if file_cid != cid { continue; }
                    }
                    if let Ok(p) = crasp_lib::commands::parse_jl_pages(&path, crawl_id_filter.as_deref()) {
                        all.extend(p);
                    }
                }
                all
            }).await.unwrap_or_default()
        };

        let mut seen = std::collections::HashSet::new();
        for p in &pages { seen.insert(p.url.clone()); }
        for p in local_pages {
            if !seen.contains(&p.url) {
                pages.push(p);
            }
        }
    }

    if let Some(ref s) = status_filter {
        let s_lower = s.to_lowercase();
        pages.retain(|p| p.stage.to_lowercase() == s_lower);
    }
    if thin_only {
        pages.retain(|p| p.thin_content == Some(true));
    }

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&pages).unwrap_or_else(|_| "[]".to_string());
            println!("{}", json);
        }
        "csv" => {
            let mut wtr = csv::Writer::from_writer(std::io::stdout());
            let _ = wtr.write_record(["url", "title", "depth", "status", "chars", "thin", "deep", "timestamp"]);
            for p in &pages {
                let thin = p.thin_content.map(|b| if b { "true" } else { "false" }).unwrap_or("");
                let deep = p.deep_fetched.map(|b| if b { "true" } else { "false" }).unwrap_or("");
                let _ = wtr.write_record([&p.url, &p.title, &p.depth.to_string(), &p.stage, &p.content_size.to_string(), thin, deep, &p.timestamp]);
            }
            let _ = wtr.flush();
        }
        _ => {
            if crawl_id.is_some() {
                println!("{:<60} {:<12} {:<6} {:<7} {:<5} {:<5}", "URL", "STATUS", "DEPTH", "CHARS", "THIN", "DEEP");
                for p in &pages {
                    let url_display = if p.url.len() > 57 { format!("{}...", &p.url[..57]) } else { p.url.clone() };
                    let thin_mark = if p.thin_content == Some(true) { "⚠".to_string() } else { String::new() };
                    let deep_mark = if p.deep_fetched == Some(true) { "✓".to_string() } else { String::new() };
                    let chars = if p.content_size > 0 { p.content_size.to_string() } else { "-".to_string() };
                    println!("{:<60} {:<12} {:<6} {:<7} {:<5} {:<5}", url_display, p.stage, p.depth, chars, thin_mark, deep_mark);
                }
            } else {
                let dir = data_dir.clone();
                let crawls: Vec<crasp_lib::commands::LocalCrawlSummary> = tokio::task::spawn_blocking(move || {
                    let read_dir = match std::fs::read_dir(&dir) {
                        Ok(rd) => rd,
                        Err(_) => return Vec::new(),
                    };
                    let mut crawls = Vec::new();
                    for entry in read_dir.flatten() {
                        let path = entry.path();
                        let filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        if !filename.starts_with("crawl-") || !filename.ends_with(".jl") { continue; }
                        let cid = filename.strip_prefix("crawl-").unwrap_or("").strip_suffix(".jl").unwrap_or("").to_string();
                        let metadata = match entry.metadata() { Ok(m) => m, Err(_) => continue };
                        let modified = metadata.modified().ok().and_then(|t| {
                            let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
                            Some(chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)?.to_rfc3339())
                        }).unwrap_or_default();
                        let page_count = {
                            let file = match std::fs::File::open(&path) { Ok(f) => f, Err(_) => continue };
                            std::io::BufReader::new(file)
                                .lines().filter(|l| l.as_ref().map_or(false, |s| !s.trim().is_empty())).count() as u64
                        };
                        let file_path = path.to_string_lossy().to_string();
                        crawls.push(crasp_lib::commands::LocalCrawlSummary {
                            crawl_id: cid, page_count, file_size_bytes: metadata.len(), last_modified: modified, file_path,
                        });
                    }
                    crawls
                }).await.unwrap_or_default();

                let mongo_crawls = gather_mongo_crawl_summaries(&ctx).await;

                if crawls.is_empty() && mongo_crawls.is_empty() {
                    println!("No crawls found.");
                    return 0;
                }

                println!("{:<30} {:<8} {:<12} {:<22} {}", "CRAWL ID", "PAGES", "FAILED", "DATE", "SEED URL");
                for c in &crawls {
                    let date_display = if c.last_modified.len() > 19 { &c.last_modified[..19] } else { c.last_modified.as_str() };
                    let seed = extract_seed_url_from_jl(&c.file_path);
                    println!("{:<30} {:<8} {:<12} {:<22} {}", c.crawl_id, c.page_count, "-", date_display, seed);
                }
                for mc in &mongo_crawls {
                    let date_display = if mc.last_modified.len() > 19 { &mc.last_modified[..19] } else { mc.last_modified.as_str() };
                    println!("{:<30} {:<8} {:<12} {:<22} {}", mc.crawl_id, mc.page_count, "-", date_display, "");
                }
            }
        }
    }

    0
}

fn extract_seed_url_from_jl(path: &str) -> String {
    let file = match std::fs::File::open(path) { Ok(f) => f, Err(_) => return String::new() };
    let reader = std::io::BufReader::new(file);
    for line_result in reader.lines() {
        let line = match line_result { Ok(l) => l, Err(_) => continue };
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        let item: serde_json::Value = match serde_json::from_str(trimmed) { Ok(v) => v, Err(_) => continue };
        if item.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) == 0 {
            if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                return url.to_string();
            }
        }
    }
    String::new()
}

async fn gather_mongo_crawl_summaries(ctx: &Arc<crasp_lib::runtime::AppContext>) -> Vec<crasp_lib::commands::LocalCrawlSummary> {
    let store = match &ctx.store {
        Some(s) => s,
        None => return Vec::new(),
    };

    let pipeline = vec![
        mongodb::bson::doc! {
            "$group": {
                "_id": "$crawl_id",
                "page_count": { "$sum": 1 },
                "latest_timestamp": { "$max": "$timestamp" },
            }
        },
    ];

    let mut cursor = match store.pages_col().aggregate(pipeline).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    use futures_util::TryStreamExt;
    while let Ok(Some(doc)) = cursor.try_next().await {
        let crawl_id = doc.get_object_id("_id").map(|_| String::new())
            .unwrap_or_else(|_| doc.get_str("_id").unwrap_or("").to_string());
        let page_count = doc.get_i64("page_count").unwrap_or(0) as u64;
        let latest_ts = doc.get_str("latest_timestamp").unwrap_or("").to_string();
        results.push(crasp_lib::commands::LocalCrawlSummary {
            crawl_id,
            page_count,
            file_size_bytes: 0,
            last_modified: latest_ts,
            file_path: String::new(),
        });
    }
    results
}

async fn run_export(
    cli: &Cli,
    crawl_id: Option<&str>,
    page_url: Option<&str>,
    format: &str,
    scope: &str,
    content: &str,
    output: Option<&str>,
) -> i32 {
    let export_format = match parse_export_format(format) {
        Ok(f) => f,
        Err(e) => { eprintln!("{} {}", "✗".red().bold(), e); return 1; }
    };
    let export_scope = match parse_export_scope(scope) {
        Ok(s) => s,
        Err(e) => { eprintln!("{} {}", "✗".red().bold(), e); return 1; }
    };
    let export_content = match parse_export_content(content) {
        Ok(c) => c,
        Err(e) => { eprintln!("{} {}", "✗".red().bold(), e); return 1; }
    };

    let request = ExportRequest {
        format: export_format,
        scope: export_scope,
        content: export_content,
        page_url: page_url.map(String::from),
        crawl_id: crawl_id.map(String::from),
        source: None,
    };

    if let Err(e) = request.is_valid() {
        eprintln!("{} {}", "✗".red().bold(), e);
        return 1;
    }

    if request.crawl_id.is_none() && request.page_url.is_none() {
        eprintln!("{} crawl-id or url is required", "✗".red().bold());
        return 1;
    }

    let ctx = build_app_context(cli.mongo_uri.clone()).await;
    let data_dir = app_data_dir();
    let default_exports_dir = PathBuf::from("./exports/");
    let _ = std::fs::create_dir_all(&default_exports_dir);

    let cid = request.crawl_id.as_deref().unwrap_or("");
    let page_url_val = request.page_url.clone();

    let page_docs = load_page_docs_for_export(&ctx, cid, &data_dir).await;
    if page_docs.is_empty() && !matches!(request.scope, ExportScope::SinglePage) {
        eprintln!("{} No pages found for this crawl", "✗".red().bold());
        return 1;
    }

    let page_count = page_docs.len();

    let ts = chrono::Utc::now().timestamp_millis();

    match request.scope {
        ExportScope::SinglePage => {
            let target_url = page_url_val.unwrap_or_default();
            let page_doc = page_docs.iter().find(|d| d.url == target_url);
            let page_doc = match page_doc {
                Some(d) => d.clone(),
                None => {
                    eprintln!("{} Page not found: {}", "✗".red().bold(), target_url);
                    return 1;
                }
            };

            let (ext, content_str) = match request.format {
                ExportFormat::PlainText => ("txt", crasp_lib::export::page_to_plain_text(&page_doc, &request.content)),
                ExportFormat::Markdown => ("md", crasp_lib::export::page_to_markdown(&page_doc, &request.content)),
                ExportFormat::Html => ("html", crasp_lib::export::page_to_html(&page_doc, &request.content)),
                ExportFormat::Epub => {
                    eprintln!("{} EPUB format is not valid for single-page scope", "✗".red().bold());
                    return 1;
                }
            };

            let slug: String = target_url.split('/').filter(|s| !s.is_empty() && *s != "http:" && *s != "https:").last().unwrap_or("page").chars().take(30).collect();
            let path = if let Some(ref out) = output {
                PathBuf::from(out)
            } else {
                let filename = format!("{}_{}.{}", slug, ts, ext);
                default_exports_dir.join(&filename)
            };

            eprintln!("Exporting page as {}...", format!("{:?}", request.format).to_lowercase());
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut file = match std::fs::File::create(&path) {
                Ok(f) => f,
                Err(e) => { eprintln!("{} {}", "✗".red().bold(), e); return 1; }
            };
            if let Err(e) = file.write_all(content_str.as_bytes()) {
                eprintln!("{} {}", "✗".red().bold(), e); return 1;
            }

            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("{} Written to {} ({})", "✓".green().bold(), path.to_string_lossy(), human_bytes(size));
        }
        ExportScope::WholeCrawlOneFile => {
            let mut sorted = page_docs.clone();
            sorted.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.url.cmp(&b.url)));

            match request.format {
                ExportFormat::Epub => {
                    eprintln!("Exporting {} pages as EPUB...", page_count);
                    let chapters: Vec<crasp_lib::export::EpubChapter> = sorted.iter().map(crasp_lib::export::page_to_epub_chapter).collect();
                    let book_title = chapters.first().map(|c| c.title.clone()).unwrap_or_else(|| "Crasp Archive".to_string());
                    let cover_url = sorted.first().and_then(|d| d.assets.as_ref()).and_then(|a| a.og_image.as_deref());
                    let output_path = if let Some(ref out) = output {
                        PathBuf::from(out)
                    } else {
                        default_exports_dir.join(format!("{}_{}.epub", cid, ts))
                    };
                    if let Some(parent) = output_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = crasp_lib::export::generate_epub(&chapters, &book_title, cover_url, &output_path) {
                        eprintln!("{} {}", "✗".red().bold(), e);
                        return 1;
                    }
                    let size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                    println!("{} Written to {} ({})", "✓".green().bold(), output_path.to_string_lossy(), human_bytes(size));
                }
                _ => {
                    let format_label = format!("{:?}", request.format).to_lowercase();
                    eprintln!("Exporting {} pages as {}...", page_count, format_label);
                    let (ext, content_str) = match request.format {
                        ExportFormat::PlainText => ("txt", crasp_lib::export::pages_to_plain_text_combined(&sorted, &request.content)),
                        ExportFormat::Markdown => ("md", crasp_lib::export::pages_to_markdown_combined(&sorted, &request.content)),
                        ExportFormat::Html => ("html", crasp_lib::export::pages_to_html_combined(&sorted, &request.content)),
                        _ => unreachable!(),
                    };
                    let path = if let Some(ref out) = output {
                        PathBuf::from(out)
                    } else {
                        let filename = format!("{}_{}_{}.{}", cid, ext, ts, request.scope_string());
                        default_exports_dir.join(&filename)
                    };
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let mut file = match std::fs::File::create(&path) {
                        Ok(f) => f,
                        Err(e) => { eprintln!("{} {}", "✗".red().bold(), e); return 1; }
                    };
                    if let Err(e) = file.write_all(content_str.as_bytes()) {
                        eprintln!("{} {}", "✗".red().bold(), e); return 1;
                    }
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    println!("{} Written to {} ({})", "✓".green().bold(), path.to_string_lossy(), human_bytes(size));
                }
            }
        }
        ExportScope::WholeCrawlFolder => {
            let mut sorted = page_docs.clone();
            sorted.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.url.cmp(&b.url)));

            let format_label = format!("{:?}", request.format).to_lowercase();
            eprintln!("Exporting {} pages as {} folder...", page_count, format_label);
            let files = match request.format {
                ExportFormat::PlainText => crasp_lib::export::pages_to_plain_text_folder(&sorted, &request.content),
                ExportFormat::Markdown => crasp_lib::export::pages_to_markdown_folder(&sorted, &request.content),
                ExportFormat::Html => crasp_lib::export::pages_to_html_folder(&sorted, &request.content),
                _ => unreachable!(),
            };

            let folder_path = if let Some(ref out) = output {
                PathBuf::from(out)
            } else {
                let folder_name = format!("{}_{}_{}", cid, request.format_string(), ts);
                default_exports_dir.join(&folder_name)
            };
            let _ = std::fs::create_dir_all(&folder_path);

            for (filename, content_str) in files {
                let file_path = folder_path.join(&filename);
                if let Ok(mut file) = std::fs::File::create(&file_path) {
                    let _ = file.write_all(content_str.as_bytes());
                }
            }

            println!("{} Written to {} ({} files + index.{})", "✓".green().bold(), folder_path.to_string_lossy(), page_count, format_label);
        }
    }

    0
}

async fn load_page_docs_for_export(ctx: &Arc<crasp_lib::runtime::AppContext>, crawl_id: &str, data_dir: &PathBuf) -> Vec<crasp_lib::schema::PageDoc> {
    let mut page_docs: Vec<crasp_lib::schema::PageDoc> = Vec::new();

    if let Some(store) = &ctx.store {
        let filter = mongodb::bson::doc! { "crawl_id": crawl_id };
        if let Ok(mut cursor) = store.pages_col().find(filter).await {
            use futures_util::TryStreamExt;
            while let Ok(Some(doc)) = cursor.try_next().await {
                page_docs.push(doc);
            }
        }
    }

    if page_docs.is_empty() {
        let crawl_id_owned = crawl_id.to_string();
        let data_dir_owned = data_dir.clone();
        page_docs = tokio::task::spawn_blocking(move || {
            let read_dir = match std::fs::read_dir(&data_dir_owned) {
                Ok(rd) => rd,
                Err(_) => return Vec::new(),
            };
            let mut docs = Vec::new();
            for entry in read_dir.flatten() {
                let path = entry.path();
                let filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if !filename.starts_with("crawl-") || !filename.ends_with(".jl") { continue; }
                let file_cid = filename.strip_prefix("crawl-").unwrap_or("").strip_suffix(".jl").unwrap_or("");
                if file_cid != crawl_id_owned { continue; }
                let file = match std::fs::File::open(&path) { Ok(f) => f, Err(_) => continue };
                let reader = std::io::BufReader::new(file);
                for line_result in reader.lines() {
                    let line = match line_result { Ok(l) => l, Err(_) => continue };
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    let item: serde_json::Value = match serde_json::from_str(trimmed) { Ok(v) => v, Err(_) => continue };
                    if item.get("crawl_id").and_then(|v| v.as_str()).unwrap_or("") != crawl_id_owned { continue; }
                    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let timestamp = item.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
                            let code = item.get("status_code").and_then(|v| v.as_u64()).unwrap_or(0) as i32;
                            if (200..300).contains(&code) { ("Completed".to_string(), None) }
                            else { ("Failed".to_string(), Some(format!("HTTP {}", code))) }
                        }
                    };
                    docs.push(crasp_lib::schema::PageDoc {
                        crawl_id: crawl_id_owned.clone(),
                        url: url.clone(),
                        url_normalized: url.to_lowercase(),
                        depth: item.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        status,
                        status_code: item.get("status_code").and_then(|v| v.as_u64()).unwrap_or(0) as i32,
                        status_reason,
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
                    });
                }
            }
            docs
        }).await.unwrap_or_default();
    }

    page_docs
}

async fn run_status_mongo(cli: &Cli) -> i32 {
    println!("{}", "Testing MongoDB connection...".bold());
    let mongo_uri = cli.mongo_uri.clone()
        .or_else(|| std::env::var("CRASP_MONGO_URI").ok())
        .unwrap_or_else(|| "mongodb://localhost:27017".to_string());

    let client = match mongodb::Client::with_uri_str(&mongo_uri).await {
        Ok(c) => c,
        Err(e) => {
            let err_str = e.to_string();
            let short = if err_str.contains("connection refused") || err_str.contains("Connection refused") {
                "connection refused".to_string()
            } else if err_str.contains("No such host") || err_str.contains("nodename nor servname") {
                "DNS resolution failed".to_string()
            } else if err_str.contains("timed out") || err_str.contains("deadline") {
                "connection timed out".to_string()
            } else {
                let first = err_str.split('\n').next().unwrap_or("connection refused").trim();
                let cleaned = first.split("reason:").last().unwrap_or(first).trim();
                if cleaned.len() > 80 {
                    format!("{}...", &cleaned[..77])
                } else {
                    cleaned.to_string()
                }
            };
            println!("{} Unreachable — {} ({})", "✗".red().bold(), short, mongo_uri);
            println!("  Set CRASP_MONGO_URI to a valid MongoDB connection string.");
            return 1;
        }
    };

    let db = client.database("crasp");
    if let Err(e) = db.run_command(mongodb::bson::doc! { "ping": 1 }).await {
        let err_str = e.to_string();
        let short = err_str.split(':').next().unwrap_or("connection refused");
        println!("{} Unreachable — {} ({})", "✗".red().bold(), short, mongo_uri);
        println!("  Set CRASP_MONGO_URI to a valid MongoDB connection string.");
        return 1;
    }

    let pages_count: u64 = db.collection::<mongodb::bson::Document>("pages").count_documents(mongodb::bson::doc! {}).await.unwrap_or(0);
    let crawls_count: u64 = db.collection::<mongodb::bson::Document>("crawls").count_documents(mongodb::bson::doc! {}).await.unwrap_or(0);
    let hashes_count: u64 = db.collection::<mongodb::bson::Document>("content_hashes").count_documents(mongodb::bson::doc! {}).await.unwrap_or(0);

    println!("{} Connected — {} → database: crasp", "✓".green().bold(), mongo_uri);
    println!("  Collections: pages ({} docs), crawls ({} docs), content_hashes ({} docs)", pages_count, crawls_count, hashes_count);
    0
}

async fn run_status_zyte() -> i32 {
    println!("{}", "Testing Zyte connection...".bold());
    let api_key = match std::env::var("ZYTE_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            println!("{} Unreachable — ZYTE_API_KEY not set", "✗".red().bold());
            println!("  Check your ZYTE_API_KEY environment variable.");
            return 1;
        }
    };

    let project_id = std::env::var("CRASP_ZYTE_PROJECT").unwrap_or_default();
    let client = crasp_lib::zyte::ZyteClient::new(api_key);
    match client.test_connection(&project_id).await {
        Ok(status) if status.ok => {
            println!("{} Connected — project: {} ({})",
                "✓".green().bold(),
                project_id,
                status.project_name.as_deref().unwrap_or("unknown"));
            println!("  Remaining unit capacity: 1 slot");
            0
        }
        Ok(status) => {
            let msg = status.message.as_deref().unwrap_or("Connection failed");
            if msg.starts_with("HTTP 401") {
                println!("{} Unreachable — 401 Unauthorized", "✗".red().bold());
                println!("  Check your ZYTE_API_KEY environment variable.");
            } else {
                println!("{} Unreachable — {}", "✗".red().bold(), msg);
                println!("  Check your ZYTE_API_KEY environment variable.");
            }
            1
        }
        Err(e) => {
            println!("{} Unreachable — {}", "✗".red().bold(), e);
            1
        }
    }
}

fn run_data_dir() -> i32 {
    let dir = app_data_dir();
    println!("App data directory: {}", dir.to_string_lossy());

    if !dir.exists() {
        println!("  (directory does not exist yet)");
        return 0;
    }

    println!();
    println!("  Contents:");

    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => { println!("  (unable to read directory)"); return 0; }
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

        if path.is_dir() {
            println!("  {}/", name);
            if let Ok(sub_dir) = std::fs::read_dir(&path) {
                for sub_entry in sub_dir.flatten() {
                    let sub_path = sub_entry.path();
                    let sub_name = sub_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let size = sub_path.metadata().map(|m| m.len()).unwrap_or(0);
                    let page_count = if sub_name.starts_with("crawl-") && sub_name.ends_with(".jl") {
                        if let Ok(file) = std::fs::File::open(&sub_path) {
                            let count = std::io::BufReader::new(file).lines().filter(|l| l.as_ref().map_or(false, |s| !s.trim().is_empty())).count();
                            let modified = sub_path.metadata().ok().and_then(|m| {
                                m.modified().ok().and_then(|t| {
                                    let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
                                    Some(chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)?.format("%Y-%m-%d").to_string())
                                })
                            }).unwrap_or_default();
                            format!(", {} pages, {}", count, modified)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    println!("    {}   ({}{})", sub_name, human_bytes(size), page_count);
                }
            }
        } else {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            println!("  {}   ({})", name, human_bytes(size));
        }
    }

    0
}

async fn run_validate(url: &str) -> i32 {
    match crasp_lib::ssrf::validate_seed_url(url).await {
        Ok(parsed) => {
            let scheme = parsed.scheme();
            println!("{} Valid — {} ({}, {} scheme)",
                "✓".green().bold(),
                url,
                if scheme == "http" || scheme == "https" { "public" } else { "unknown" },
                if scheme == "http" || scheme == "https" { "reachable" } else { "unsupported" });
            0
        }
        Err(e) => {
            let reason = if e.contains("reserved") || e.contains("private") || e.contains("loopback") || e.contains("link-local") {
                if e.contains("192.168") { format!("private IP address ({} is in 192.168.0.0/16)", url) }
                else if e.contains("127.0") || e.contains("loopback") { format!("loopback address ({})", url) }
                else if e.contains("169.254") || e.contains("link-local") { format!("link-local address ({} is in 169.254.0.0/16)", url) }
                else if e.contains("172.16") || e.contains("172.") && e.contains("/12") { format!("private IP address ({} is in 172.16.0.0/12)", url) }
                else if e.contains("10.") || e.contains("/8") { format!("private IP address ({})", url) }
                else if e.contains("100.64") { format!("CGNAT address ({} is in 100.64.0.0/10)", url) }
                else { format!("reserved/non-public address ({})", url) }
            } else if e.contains("Unsupported scheme") {
                let scheme = url.split("://").next().unwrap_or("unknown");
                format!("unsupported scheme ({}://)", scheme)
            } else {
                e
            };
            println!("{} Rejected — {}", "✗".red().bold(), reason);
            1
        }
    }
}

async fn run_deep_fetch(cli: &Cli, url: &str, crawl_id: Option<&str>) -> i32 {
    if let Err(e) = crasp_lib::ssrf::validate_seed_url(url).await {
        eprintln!("{} Rejected — {}", "✗".red().bold(), e);
        return 1;
    }

    let ctx = build_app_context(cli.mongo_uri.clone()).await;

    let zyte = match &ctx.zyte {
        Some(c) => c,
        None => {
            eprintln!("{} ZYTE_API_KEY not configured", "✗".red().bold());
            return 1;
        }
    };

    if !ctx.deep_fetch_config.enabled {
        eprintln!("{} Deep fetch not enabled — set CRASP_DEEP_FETCH_ENABLED=true", "⚠".yellow());
    }

    eprintln!("{} Deep fetching {}...", "→".blue(), url);

    let _permit = match ctx.deep_fetch_queue.acquire().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} Rate limit: {}", "✗".red().bold(), e);
            return 1;
        }
    };

    match zyte.deep_fetch(url, &ctx.http).await {
        Ok(result) => {
            let extraction = if let Some(ref article) = result.article {
                crasp_lib::zyte::zyte_article_to_extraction(article, &result.browser_html, url)
            } else {
                let er = crasp_lib::extraction::extract_main_content(&result.browser_html, url);
                crasp_lib::extraction::ZyteExtractionResult {
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
            };

            if let Some(cid) = crawl_id {
                if let Some(store) = &ctx.store {
                    let filter = mongodb::bson::doc! { "url": url, "crawl_id": cid };
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
                    let _ = store.pages_col().update_one(filter, update).with_options(options).await;
                }
            }

            println!("{} Deep fetch complete", "✓".green().bold());
            println!("  Method:     {}", extraction.method);
            println!("  Confidence: {:.2}", extraction.confidence);
            println!("  Thin:       {}", extraction.thin_content);
            println!("  Body text:  {} chars", extraction.body_text.len());
            println!("  Body HTML:  {} chars", extraction.body_html.len());
            println!("  Headline:   {}", extraction.title.as_deref().unwrap_or("(none)"));
            println!("  Author:     {}", extraction.author.as_deref().unwrap_or("(none)"));
            if result.article.is_some() {
                println!("  AutoExtract: yes (structured article data)");
            } else {
                println!("  AutoExtract: no (readability fallback on browserHtml)");
            }

            0
        }
        Err(e) => {
            eprintln!("{} Deep fetch failed: {}", "✗".red().bold(), e);
            1
        }
    }
}

async fn run_test_zyte_api(cli: &Cli, test_url: &str) -> i32 {
    println!("{} Testing Zyte API access...", "→".blue());

    let ctx = build_app_context(cli.mongo_uri.clone()).await;

    let zyte = match &ctx.zyte {
        Some(c) => c,
        None => {
            eprintln!("{} ZYTE_API_KEY not configured", "✗".red().bold());
            return 1;
        }
    };

    match zyte.deep_fetch(test_url, &ctx.http).await {
        Ok(result) => {
            println!("{} Zyte API access confirmed!", "✓".green().bold());
            println!("  Status code: {}", result.status_code);
            println!("  Browser HTML: {} chars", result.browser_html.len());
            if result.article.is_some() {
                println!("  AutoExtract: available");
            } else {
                println!("  AutoExtract: not available for this URL");
            }
            println!("  WI-35-B path: Zyte API (browserHtml + AutoExtract)");
            0
        }
        Err(e) => {
            if e.contains("403") || e.contains("402") {
                eprintln!("{} Zyte API unavailable ({}): WI-35-B-alt path required", "⚠".yellow(), e.trim());
                println!("  → Scrapy Cloud spider fallback will be used for JS rendering");
            } else if e.contains("401") {
                eprintln!("{} Unauthorized ({}): check ZYTE_API_KEY", "✗".red().bold(), e.trim());
            } else {
                eprintln!("{} Zyte API error: {}", "✗".red().bold(), e);
            }
            1
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    if cli.verbose > 0 {
        eprintln!("Verbosity: {} (level={})", cli.verbose, log_level);
    }

    let rt = build_runtime();
    let exit_code = rt.block_on(async {
        match &cli.command {
            Commands::Crawl { url, max_pages, max_depth, concurrency, selectors, preserve_html, hash_algorithm, output, engine } => {
                run_crawl(&cli, url, *max_pages, *max_depth, *concurrency, selectors, *preserve_html, hash_algorithm, output.as_deref(), engine).await
            }
            Commands::List { crawl_id, format, status, thin_only } => {
                run_list(&cli, crawl_id.as_deref(), format, status.as_deref(), *thin_only).await
            }
            Commands::Export { crawl_id, url, format, scope, content, output } => {
                run_export(&cli, crawl_id.as_deref(), url.as_deref(), format, scope, content, output.as_deref()).await
            }
            Commands::Status { mongo, zyte } => {
                if *mongo {
                    run_status_mongo(&cli).await
                } else if *zyte {
                    run_status_zyte().await
                } else {
                    eprintln!("Specify --mongo or --zyte");
                    1
                }
            }
            Commands::DataDir => run_data_dir(),
            Commands::Validate { url } => run_validate(url).await,
            Commands::DeepFetch { url, crawl_id } => run_deep_fetch(&cli, &url, crawl_id.as_deref()).await,
            Commands::TestZyteApi { url } => run_test_zyte_api(&cli, &url).await,
        }
    });

    std::process::exit(exit_code);
}
