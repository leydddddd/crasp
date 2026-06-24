use serde::{Deserialize, Serialize};

use crate::progress::CraspEmitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub engine: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl_id: Option<String>,
}

pub fn emit_log(
    emitter: &CraspEmitter,
    level: &str,
    engine: &str,
    message: &str,
) -> tokio::task::JoinHandle<()> {
    emit_log_with_crawl_id(emitter, level, engine, message, None)
}

pub fn emit_log_with_crawl_id(
    emitter: &CraspEmitter,
    level: &str,
    engine: &str,
    message: &str,
    crawl_id: Option<String>,
) -> tokio::task::JoinHandle<()> {
    let entry = LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        engine: engine.to_string(),
        message: message.to_string(),
        crawl_id,
    };
    let emitter = emitter.clone();
    tokio::spawn(async move {
        let _ = emitter
            .emit("app-log", &serde_json::to_value(&entry).unwrap_or_default())
            .await;
    })
}
