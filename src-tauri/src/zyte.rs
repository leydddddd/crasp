use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ZyteClient {
    http: reqwest::Client,
    api_key: String,
    base: String,
}

#[derive(Debug, Clone)]
pub struct DeepFetchConfig {
    pub enabled: bool,
    pub auto_trigger: bool,
    pub max_per_crawl: u32,
    pub request_delay_ms: u64,
    pub chrome_available: bool,
}

impl Default for DeepFetchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_trigger: false,
            max_per_crawl: 10,
            request_delay_ms: 2000,
            chrome_available: false,
        }
    }
}

impl DeepFetchConfig {
    pub fn from_env() -> Self {
        let chrome_available = crate::headless::find_chrome_binary().is_some();
        let env_enabled = std::env::var("CRASP_DEEP_FETCH_ENABLED")
            .ok()
            .and_then(|v| v.to_lowercase().parse::<bool>().ok())
            .unwrap_or(false);
        let enabled = chrome_available && env_enabled;
        Self {
            enabled,
            auto_trigger: enabled
                && std::env::var("CRASP_DEEP_FETCH_AUTO")
                    .ok()
                    .and_then(|v| v.to_lowercase().parse::<bool>().ok())
                    .unwrap_or(false),
            max_per_crawl: std::env::var("CRASP_DEEP_FETCH_MAX")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(10),
            request_delay_ms: 2000,
            chrome_available,
        }
    }
}

pub struct DeepFetchQueue {
    semaphore: Arc<tokio::sync::Semaphore>,
    last_request: Arc<tokio::sync::Mutex<std::time::Instant>>,
    delay_ms: u64,
    counter: Arc<AtomicU32>,
    max: u32,
}

pub struct DeepFetchPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl DeepFetchQueue {
    pub fn new(delay_ms: u64, max: u32) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            last_request: Arc::new(tokio::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(100),
            )),
            delay_ms,
            counter: Arc::new(AtomicU32::new(0)),
            max,
        }
    }

    pub async fn acquire(&self) -> Result<DeepFetchPermit, String> {
        let count = self.counter.fetch_add(1, Ordering::SeqCst);
        if count >= self.max {
            self.counter.fetch_sub(1, Ordering::SeqCst);
            return Err(format!(
                "Deep fetch cap reached ({} requests this session)",
                self.max
            ));
        }

        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| e.to_string())?;

        {
            let mut last = self.last_request.lock().await;
            let elapsed = last.elapsed();
            let delay = std::time::Duration::from_millis(self.delay_ms);
            if elapsed < delay {
                tokio::time::sleep(delay - elapsed).await;
            }
            *last = std::time::Instant::now();
        }

        Ok(DeepFetchPermit { _permit: permit })
    }

    pub fn reset_counter(&self) {
        self.counter.store(0, Ordering::SeqCst);
    }
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct ZyteExtractRequest {
    url: String,
    #[serde(rename = "browserHtml", skip_serializing_if = "std::ops::Not::not")]
    browser_html: bool,
    #[serde(rename = "article", skip_serializing_if = "std::ops::Not::not")]
    article: bool,
    #[serde(rename = "articleOptions", skip_serializing_if = "Option::is_none")]
    article_options: Option<ArticleOptions>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct ArticleOptions {
    #[serde(rename = "extractFrom")]
    extract_from: String,
}

#[derive(Debug, Deserialize)]
struct ZyteExtractResponse {
    #[allow(dead_code)]
    url: Option<String>,
    #[serde(rename = "browserHtml")]
    browser_html: Option<String>,
    article: Option<ZyteArticle>,
    #[serde(rename = "statusCode")]
    status_code: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct ZyteArticle {
    headline: Option<String>,
    #[serde(rename = "datePublished")]
    date_published: Option<String>,
    #[serde(rename = "datePublishedRaw")]
    date_published_raw: Option<String>,
    author: Option<Vec<ZyteAuthor>>,
    #[serde(rename = "articleBody")]
    article_body: Option<String>,
    #[serde(rename = "articleBodyHtml")]
    article_body_html: Option<String>,
    description: Option<String>,
    #[allow(dead_code)]
    images: Option<Vec<ZyteImage>>,
}

#[derive(Debug, Deserialize)]
struct ZyteAuthor {
    #[serde(rename = "nameRaw")]
    name_raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZyteImage {
    #[allow(dead_code)]
    url: Option<String>,
}

pub struct DeepFetchResult {
    pub browser_html: String,
    pub article: Option<ZyteArticle>,
    pub status_code: u16,
}

pub fn zyte_article_to_extraction(
    article: &ZyteArticle,
    browser_html: &str,
    url: &str,
) -> crate::extraction::ZyteExtractionResult {
    let body_html = article
        .article_body_html
        .clone()
        .or_else(|| {
            article
                .article_body
                .as_ref()
                .map(|t| format!("<p>{}</p>", t.replace('\n', "</p><p>")))
        })
        .unwrap_or_default();

    let body_text = article
        .article_body
        .clone()
        .unwrap_or_else(|| {
            body_html
                .replace('<', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        });

    let word_count = body_text.split_whitespace().count();
    let reading_time = (word_count / 200).max(1) as u32;
    let non_ws = body_text.chars().filter(|c| !c.is_whitespace()).count();
    let is_thin = non_ws < 200;

    let author = article
        .author
        .as_ref()
        .and_then(|authors| authors.first())
        .and_then(|a| a.name_raw.clone());

    let _ = browser_html;
    let _ = url;

    crate::extraction::ZyteExtractionResult {
        title: article.headline.clone(),
        author,
        published_date: article
            .date_published
            .clone()
            .or_else(|| article.date_published_raw.clone()),
        excerpt: article.description.clone(),
        body_html,
        body_text,
        reading_time_minutes: reading_time,
        confidence: 0.95,
        method: "zyte_autoextract".to_string(),
        thin_content: is_thin,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteJobRequest {
    pub project: String,
    pub spider: String,
    pub add_arguments: ZyteJobArguments,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteJobArguments {
    pub seed_url: String,
    pub max_depth: u32,
    pub max_pages: u32,
    pub css_selectors: String,
    pub preserve_html: String,
    pub hash_algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteJobResponse {
    pub key: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteJobStatus {
    pub key: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteProgress {
    pub job_key: String,
    pub state: String,
    pub items_scraped: Option<u64>,
    pub requests: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZyteConnectionStatus {
    pub ok: bool,
    pub project_name: Option<String>,
    pub message: Option<String>,
}

impl ZyteClient {
    pub fn new(api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build Zyte HTTP client");

        Self {
            http,
            api_key,
            base: "https://app.zyte.com".to_string(),
        }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub async fn deep_fetch(
        &self,
        url: &str,
        http: &reqwest::Client,
    ) -> Result<DeepFetchResult, String> {
        let request_body = serde_json::json!({
            "url": url,
            "browserHtml": true,
            "article": true,
            "articleOptions": { "extractFrom": "browserHtml" }
        });

        let response = http
            .post("https://api.zyte.com/v1/extract")
            .basic_auth(&self.api_key, Some(""))
            .json(&request_body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| format!("Zyte API request failed: {}", e))?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let truncated = &body[..body.len().min(200)];
            return Err(format!("Zyte API error {}: {}", status, truncated));
        }

        let data: ZyteExtractResponse = response
            .json()
            .await
            .map_err(|e| format!("Zyte API response parse error: {}", e))?;

        Ok(DeepFetchResult {
            browser_html: data.browser_html.unwrap_or_default(),
            article: data.article,
            status_code: data.status_code.unwrap_or(200),
        })
    }

    pub async fn test_connection(
        &self,
        project_id: &str,
    ) -> Result<ZyteConnectionStatus, String> {
        let url = format!("{}/api/projects/{}", self.base, project_id);
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.api_key, Some(""))
            .send()
            .await
            .map_err(|e| format!("Zyte connection test failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Ok(ZyteConnectionStatus {
                ok: false,
                project_name: None,
                message: Some(format!("HTTP {}: {}", status, body.trim())),
            });
        }

        let project: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse Zyte project response: {}", e))?;

        let project_name = project
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok(ZyteConnectionStatus {
            ok: true,
            project_name,
            message: None,
        })
    }

    pub async fn run_job(&self, req: &ZyteJobRequest) -> Result<String, String> {
        let url = format!("{}/api/jobs/run/{}", self.base, req.project);

        let form = vec![
            ("spider", req.spider.clone()),
            ("add_arguments.seed_url", req.add_arguments.seed_url.clone()),
            ("add_arguments.max_depth", req.add_arguments.max_depth.to_string()),
            ("add_arguments.max_pages", req.add_arguments.max_pages.to_string()),
            ("add_arguments.css_selectors", req.add_arguments.css_selectors.clone()),
            ("add_arguments.preserve_html", req.add_arguments.preserve_html.clone()),
            ("add_arguments.hash_algorithm", req.add_arguments.hash_algorithm.clone()),
        ];

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Zyte API request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Zyte API error {}: {}", status, body));
        }

        let job: ZyteJobResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse Zyte job response: {}", e))?;

        Ok(job.key)
    }

    pub async fn wait_for_job(
        &self,
        job_key: &str,
        tx: mpsc::Sender<ZyteProgress>,
    ) -> Result<(), String> {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            let url = format!("{}/api/jobs/{}", self.base, job_key);
            let resp = self
                .http
                .get(&url)
                .basic_auth(&self.api_key, Some(""))
                .send()
                .await
                .map_err(|e| format!("Zyte status check failed: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Zyte status check error {}: {}", status, body));
            }

            let status: ZyteJobStatus = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse Zyte job status: {}", e))?;

            let _ = tx
                .send(ZyteProgress {
                    job_key: job_key.to_string(),
                    state: status.state.clone(),
                    items_scraped: None,
                    requests: None,
                })
                .await;

            match status.state.as_str() {
                "finished" => return Ok(()),
                "cancelled" => return Err("Zyte job was cancelled".to_string()),
                "error" => return Err("Zyte job failed with error state".to_string()),
                _ => {}
            }
        }
    }

    pub async fn fetch_items(
        &self,
        job_key: &str,
        batch_size: usize,
        tx: mpsc::Sender<serde_json::Value>,
    ) -> Result<usize, String> {
        let mut start = 0;
        let mut total = 0;

        loop {
            let url = format!(
                "{}/api/items/{}?start={}&count={}",
                self.base, job_key, start, batch_size
            );

            let resp = self
                .http
                .get(&url)
                .basic_auth(&self.api_key, Some(""))
                .send()
                .await
                .map_err(|e| format!("Zyte items fetch failed: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Zyte items fetch error {}: {}", status, body));
            }

            let raw_body = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read Zyte items response: {}", e))?;

            let items: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
                let mut parsed = Vec::new();
                for line in raw_body.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                        parsed.push(val);
                    }
                }
                parsed
            })
            .await
            .map_err(|e| format!("spawn_blocking panic: {}", e))?;

            let count = items.len();
            if count == 0 {
                break;
            }

            for item in items {
                let _ = tx.send(item).await;
                total += 1;
            }

            if count < batch_size {
                break;
            }

            start += batch_size;
        }

        Ok(total)
    }
}
