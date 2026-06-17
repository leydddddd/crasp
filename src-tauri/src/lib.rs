mod commands;
mod crawler;
#[allow(dead_code)]
mod local_scrapy;
#[allow(dead_code)]
mod runtime;
#[allow(dead_code)]
mod schema;
#[allow(dead_code)]
mod store;
#[allow(dead_code)]
mod zyte;

use tauri::Emitter;
use commands::CrawlState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(CrawlState::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_crawl,
            commands::start_cloud_crawl,
            commands::local_scrapy_crawl,
            commands::cancel_crawl,
            commands::pause_crawl,
            commands::resume_crawl,
            commands::validate_url,
            commands::default_config,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tokio::spawn(async move {
                match runtime::AppContext::from_env().await {
                    Ok(ctx) => {
                        let _ = handle.emit("app-ready", serde_json::json!({
                            "zyte_available": ctx.zyte.is_some(),
                        }));
                    }
                    Err(e) => {
                        let _ = handle.emit("app-error", serde_json::json!({
                            "error": e,
                        }));
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Crasp");
}
