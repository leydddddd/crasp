use serde::{Deserialize, Serialize};

use crate::commands::TauriEmitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub engine: String,
    pub message: String,
}

pub fn emit_log(
    emitter: &TauriEmitter,
    level: &str,
    engine: &str,
    message: &str,
) -> tokio::task::JoinHandle<()> {
    let entry = LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        engine: engine.to_string(),
        message: message.to_string(),
    };
    let emitter = emitter.clone();
    tokio::spawn(async move {
        let _ = emitter
            .emit("app-log", &serde_json::to_value(&entry).unwrap_or_default())
            .await;
    })
}
