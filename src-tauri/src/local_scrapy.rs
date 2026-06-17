use std::process::Stdio;
use tokio::process::Command;

use crate::crawler::CrawlConfig;

pub async fn run_local_spider(
    config: &CrawlConfig,
    out_path: &str,
) -> Result<std::process::Output, String> {
    let css_selectors = config.css_selectors.join(",");

    let output = Command::new("scrapy")
        .arg("crawl")
        .arg("crasp_archive")
        .arg("-a")
        .arg(format!("seed_url={}", config.seed_url))
        .arg("-a")
        .arg(format!("max_depth={}", config.max_depth))
        .arg("-a")
        .arg(format!("max_pages={}", config.max_pages))
        .arg("-a")
        .arg(format!("css_selectors={}", css_selectors))
        .arg("-a")
        .arg(format!("preserve_html={}", config.preserve_html))
        .arg("-a")
        .arg(format!("hash_algorithm={}", match config.hash_algorithm {
            crate::crawler::HashAlgorithm::Md5 => "md5",
            crate::crawler::HashAlgorithm::Sha256 => "sha256",
        }))
        .arg("-o")
        .arg(out_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn scrapy: {}", e))?
        .wait_with_output()
        .await
        .map_err(|e| format!("Scrapy process error: {}", e))?;

    Ok(output)
}
