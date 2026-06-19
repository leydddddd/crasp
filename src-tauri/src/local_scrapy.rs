use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::commands::{persist_items_with_outcome, emit_persist_stages, TauriEmitter};
use crate::crawler::{CrawlConfig, CrawlControl, PageStage};
use crate::runtime::AppContext;

fn config_to_args(config: &CrawlConfig, out_path: &str) -> Vec<String> {
    vec![
        "crawl".to_string(),
        "crasp_archive".to_string(),
        "-a".to_string(),
        format!("seed_url={}", config.seed_url),
        "-a".to_string(),
        format!("max_depth={}", config.max_depth),
        "-a".to_string(),
        format!("max_pages={}", config.max_pages),
        "-a".to_string(),
        format!("css_selectors={}", config.css_selectors.join(",")),
        "-a".to_string(),
        format!("preserve_html={}", config.preserve_html),
        "-a".to_string(),
        format!(
            "hash_algorithm={}",
            match config.hash_algorithm {
                crate::crawler::HashAlgorithm::Md5 => "md5",
                crate::crawler::HashAlgorithm::Sha256 => "sha256",
            }
        ),
        "-o".to_string(),
        out_path.to_string(),
    ]
}

pub async fn run_local_spider_streaming(
    config: &CrawlConfig,
    out_path: &str,
    emitter: &TauriEmitter,
    control: &Arc<CrawlControl>,
    ctx: &Arc<AppContext>,
    crawl_id: &str,
    app_data_dir: &str,
) -> Result<u64, String> {
    // Local-Scrapy pipeline stages are coarser — we emit Discovered,
    // Fetching (representing the remote job), then after items are
    // fetched and persisted, Persisting -> Persisted (or Failed).
    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": config.seed_url,
        "crawl_id": crawl_id,
        "stage": PageStage::Discovered,
    })).await;

    let _ = emitter.emit("page-stage", &serde_json::json!({
        "url": config.seed_url,
        "crawl_id": crawl_id,
        "stage": PageStage::Fetching,
    })).await;

    let args = config_to_args(config, out_path);

    let mut child = Command::new("scrapy")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn scrapy: {}", e))?;

    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
    let stderr_reader = BufReader::new(stderr);

    let emitter_stderr = emitter.clone();
    let control_stderr = control.clone();

    let stderr_task = tokio::spawn(async move {
        let mut lines = stderr_reader.lines();
        let mut items_scraped: u64 = 0;

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if line.contains("item_scraped_count") {
                                if let Some(rest) = line.split("item_scraped_count").nth(1) {
                                    let trimmed = rest.trim();
                                    let num_str = trimmed
                                        .trim_start_matches('=')
                                        .trim_start_matches(':')
                                        .trim()
                                        .split(|c: char| !c.is_ascii_digit())
                                        .next()
                                        .unwrap_or("0");
                                    if let Ok(n) = num_str.parse::<u64>() {
                                        if n > items_scraped {
                                            items_scraped = n;
                                            let _ = emitter_stderr
                                                .emit(
                                                    "scrape-progress",
                                                    &serde_json::json!({
                                                        "url": format!("item #{}", items_scraped),
                                                        "status": "archiving",
                                                        "depth": 0,
                                                    }),
                                                )
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                    if control_stderr.is_cancelled() {
                        break;
                    }
                }
            }
        }
    });

    let mut cancel_rx = control.cancel_rx();
    let status = tokio::select! {
        status = child.wait() => {
            status.map_err(|e| format!("Scrapy process error: {}", e))?
        }
        _ = cancel_rx.changed() => {
            if *cancel_rx.borrow() {
                let _ = child.kill().await;
                let _ = child.wait().await;

                let _ = stderr_task.abort();
                let _ = emitter.emit("crawl-done", &serde_json::json!({
                    "pages_archived": 0,
                    "cancelled": true,
                    "crawl_id": crawl_id,
                })).await;

                return Ok(0);
            }
            child.wait().await.map_err(|e| format!("Scrapy process error: {}", e))?
        }
    };

    let _ = stderr_task.await;

    if !status.success() {
        return Err(format!("Scrapy process exited with status: {}", status));
    }

    let out_path_owned = out_path.to_string();
    let emitter_ingest = emitter.clone();
    let ctx_ingest = ctx.clone();
    let crawl_id_ingest = crawl_id.to_string();
    let app_data_dir_ingest = app_data_dir.to_string();

    let items: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
        let file = match std::fs::File::open(&out_path_owned) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = std::io::BufReader::new(file);
        let mut items: Vec<serde_json::Value> = Vec::new();

        for line_result in std::io::BufRead::lines(reader) {
            match line_result {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        items.push(val);
                    }
                }
                Err(_) => break,
            }
        }

        items
    })
    .await
    .map_err(|e| format!("spawn_blocking panic: {}", e))?;

    let count = items.len() as u64;

    for item in &items {
        let _ = emitter_ingest.emit("archive-success", item).await;
    }

    let fallback_active = std::sync::atomic::AtomicBool::new(false);
    let outcomes = persist_items_with_outcome(
        &ctx_ingest,
        items,
        &crawl_id_ingest,
        &app_data_dir_ingest,
        &fallback_active,
    )
    .await;
    emit_persist_stages(&emitter_ingest, &crawl_id_ingest, outcomes).await;

    let _ = emitter.emit("crawl-done", &serde_json::json!({
        "pages_archived": count,
        "cancelled": false,
        "crawl_id": crawl_id,
    })).await;

    Ok(count)
}
