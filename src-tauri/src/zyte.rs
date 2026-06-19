use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ZyteClient {
    http: reqwest::Client,
    api_key: String,
    base: String,
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
