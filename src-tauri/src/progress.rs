use tokio::sync::mpsc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrawlPageDone {
    pub url: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub depth: u32,
    pub chars: usize,
    pub thin_content: bool,
    pub elapsed_ms: u64,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CrawlProgressEvent {
    Log { level: String, engine: String, message: String, crawl_id: Option<String> },
    PageDone(CrawlPageDone),
    Discover { url: String, depth: u32, parent: String },
}

#[derive(Clone)]
pub struct CliEmitter {
    tx: mpsc::UnboundedSender<CrawlProgressEvent>,
}

impl CliEmitter {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<CrawlProgressEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    pub async fn emit(&self, event: &str, payload: &serde_json::Value) -> Result<(), String> {
        let progress_event = match event {
            "app-log" => {
                let level = payload.get("level").and_then(|v| v.as_str()).unwrap_or("info").to_string();
                let engine = payload.get("engine").and_then(|v| v.as_str()).unwrap_or("local").to_string();
                let message = payload.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let crawl_id = payload.get("crawl_id").and_then(|v| v.as_str()).map(String::from);
                Some(CrawlProgressEvent::Log { level, engine, message, crawl_id })
            }
            "archive-success" | "archive-failed" => {
                let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let status = match payload.get("status") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Object(map)) => {
                        if map.contains_key("Failed") { "Failed".to_string() }
                        else if map.contains_key("Skipped") { "Skipped".to_string() }
                        else { "Unknown".to_string() }
                    }
                    _ => "Completed".to_string(),
                };
                let status_reason = match payload.get("status") {
                    Some(serde_json::Value::Object(map)) => {
                        map.get("Failed").and_then(|v| v.as_str()).map(String::from)
                            .or_else(|| map.get("Skipped").and_then(|v| v.as_str()).map(String::from))
                    }
                    _ => None,
                };
                let depth = payload.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let content = payload.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let chars = content.len();
                let thin_content = payload.get("thin_content").and_then(|v| v.as_bool()).unwrap_or(false);
                let hash = payload.get("hash").and_then(|v| v.as_str()).map(String::from);
                Some(CrawlProgressEvent::PageDone(CrawlPageDone {
                    url, status, status_reason, depth, chars, thin_content, elapsed_ms: 0, hash,
                }))
            }
            "crawl-discover" => {
                let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let depth = payload.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let parent = payload.get("parent").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(CrawlProgressEvent::Discover { url, depth, parent })
            }
            _ => None,
        };

        if let Some(evt) = progress_event {
            let _ = self.tx.send(evt);
        }

        Ok(())
    }
}

#[derive(Clone)]
pub enum CraspEmitter {
    Tauri(crate::commands::TauriEmitter),
    Cli(CliEmitter),
}

impl CraspEmitter {
    pub async fn emit(&self, event: &str, payload: &serde_json::Value) -> Result<(), String> {
        match self {
            CraspEmitter::Tauri(e) => e.emit(event, payload).await,
            CraspEmitter::Cli(e) => e.emit(event, payload).await,
        }
    }
}
