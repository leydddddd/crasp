pub mod commands;
pub mod crawler;
pub mod extraction;
pub mod headless;
pub mod local_scrapy;
pub mod logging;
pub mod progress;
pub mod runtime;
pub mod schema;
pub mod ssrf;
pub mod store;
pub mod zyte;
pub mod export;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use commands::CrawlState;
use runtime::AppContext;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(CrawlState::new())
        .manage(Arc::new(AppContext::degraded()))
        .invoke_handler(tauri::generate_handler![
            commands::start_crawl,
            commands::start_cloud_crawl,
            commands::local_scrapy_crawl,
            commands::cancel_crawl,
            commands::pause_crawl,
            commands::resume_crawl,
            commands::validate_url,
            commands::default_config,
            commands::get_app_status,
            commands::test_mongo_connection,
            commands::test_zyte_connection,
            commands::list_archived_pages,
            commands::list_local_crawls,
            commands::list_crawls,
            commands::get_crawl_doc,
            commands::rename_crawl,
            commands::get_page_content,
            commands::export_content,
            commands::deep_fetch_page,
            commands::reveal_in_explorer,
            commands::open_data_folder,
            commands::get_last_crawl_summary,
            commands::preview_frontier,
            commands::list_assets,
            commands::export_logs,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                let (ctx, is_degraded) = match AppContext::from_env().await {
                    Ok(ctx) => (ctx, false),
                    Err(_) => (AppContext::degraded(), true),
                };

                let app_status = ctx.to_app_status();

                handle.manage(Arc::new(ctx));

                if is_degraded || matches!(app_status.mongo_state, runtime::ServiceState::NotConfigured | runtime::ServiceState::Unreachable) {
                    let _ = handle.emit("app-error", serde_json::json!({
                        "mongo_state": app_status.mongo_state,
                        "mongo_detail": app_status.mongo_detail,
                        "zyte_state": app_status.zyte_state,
                        "zyte_detail": app_status.zyte_detail,
                        "zyte_project": app_status.zyte_project,
                    }));
                }

                let _ = handle.emit("app-ready", serde_json::to_value(&app_status).unwrap_or_default());
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Crasp");
}
