mod commands;
mod crawler;
#[allow(dead_code)]
mod local_scrapy;
mod runtime;
#[allow(dead_code)]
mod schema;
mod store;
mod zyte;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use commands::CrawlState;
use runtime::AppContext;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(CrawlState::new())
        // Pre-register a degraded AppContext so that any Tauri command
        // referencing State<'_, Arc<AppContext>> won't panic with
        // "state not found" if it fires before the async setup completes.
        // The real context replaces this once AppContext::from_env() finishes.
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
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Use tauri::async_runtime::spawn() instead of tokio::spawn().
            // Tauri v2's .setup() callback runs BEFORE the Tokio reactor is
            // live in release builds — tokio::spawn() panics with "there is
            // no reactor running". tauri::async_runtime::spawn() delegates
            // to whatever async runtime Tauri has configured (Tokio with the
            // correct runtime handle), so it works in both dev and release.
            tauri::async_runtime::spawn(async move {
                let (ctx, is_degraded) = match AppContext::from_env().await {
                    Ok(ctx) => (ctx, false),
                    Err(_) => (AppContext::degraded(), true),
                };

                let mongo_ok = ctx.mongo_ok();
                let zyte_available = ctx.zyte_available();
                let zyte_project = ctx.zyte_project.clone();

                // Replace the pre-registered degraded context with the real one.
                // manage() on an already-managed type overwrites the previous value.
                handle.manage(Arc::new(ctx));

                if is_degraded || !mongo_ok {
                    let _ = handle.emit("app-error", serde_json::json!({
                        "mongo_ok": mongo_ok,
                        "zyte_available": zyte_available,
                        "zyte_project": zyte_project,
                    }));
                }

                let _ = handle.emit("app-ready", serde_json::json!({
                    "mongo_ok": mongo_ok,
                    "zyte_available": zyte_available,
                    "zyte_project": zyte_project,
                }));
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Crasp");
}
