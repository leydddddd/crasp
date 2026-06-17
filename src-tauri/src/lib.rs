mod commands;
mod crawler;

use commands::CrawlerHandle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(CrawlerHandle::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_crawl,
            commands::cancel_crawl,
            commands::pause_crawl,
            commands::resume_crawl,
            commands::validate_url,
            commands::default_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SiteVault");
}
