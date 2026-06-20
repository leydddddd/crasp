use std::sync::atomic::Ordering;

#[tokio::test]
async fn ssrf_rejects_localhost() {
    let result = crasp_lib::ssrf::validate_seed_url("http://localhost/").await;
    assert!(result.is_err(), "localhost should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_loopback_ip() {
    let result = crasp_lib::ssrf::validate_seed_url("http://127.0.0.1/").await;
    assert!(result.is_err(), "127.0.0.1 should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_private_10() {
    let result = crasp_lib::ssrf::validate_seed_url("http://10.0.0.1/").await;
    assert!(result.is_err(), "10.0.0.1 should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_private_172_16() {
    let result = crasp_lib::ssrf::validate_seed_url("http://172.16.0.1/").await;
    assert!(result.is_err(), "172.16.0.1 should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_private_192_168() {
    let result = crasp_lib::ssrf::validate_seed_url("http://192.168.1.1/").await;
    assert!(result.is_err(), "192.168.1.1 should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_cloud_metadata() {
    let result = crasp_lib::ssrf::validate_seed_url("http://169.254.169.254/").await;
    assert!(result.is_err(), "169.254.169.254 should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_file_scheme() {
    let result = crasp_lib::ssrf::validate_seed_url("file:///etc/passwd").await;
    assert!(result.is_err(), "file:// scheme should be rejected");
}

#[tokio::test]
async fn ssrf_rejects_windows_file_scheme() {
    let result = crasp_lib::ssrf::validate_seed_url("file:///C:/Windows/System32/drivers/etc/hosts").await;
    assert!(result.is_err(), "Windows file:// scheme should be rejected");
}

#[tokio::test]
async fn ssrf_accepts_public_url() {
    let result = crasp_lib::ssrf::validate_seed_url("https://example.com/").await;
    assert!(result.is_ok(), "example.com should be accepted: {:?}", result);
}

#[test]
fn crawl_control_cancel_sets_flag() {
    let control = crasp_lib::crawler::CrawlControl::new();
    assert!(!control.is_cancelled());
    control.cancel();
    assert!(control.is_cancelled());
}

#[test]
fn crawl_control_pause_resume() {
    let control = crasp_lib::crawler::CrawlControl::new();
    assert!(!control.is_paused());
    control.pause();
    assert!(control.is_paused());
    control.resume();
    assert!(!control.is_paused());
}

#[test]
fn crawl_control_reset() {
    let control = crasp_lib::crawler::CrawlControl::new();
    control.cancel();
    control.pause();
    assert!(control.is_cancelled());
    assert!(control.is_paused());
    control.reset();
    assert!(!control.is_cancelled());
    assert!(!control.is_paused());
}

#[test]
fn engine_select_state_isolation() {
    let control1 = crasp_lib::crawler::CrawlControl::new();
    let control2 = crasp_lib::crawler::CrawlControl::new();

    control1.pause();
    assert!(control1.is_paused());
    assert!(!control2.is_paused(), "second control should not be paused");

    control2.cancel();
    assert!(control2.is_cancelled());
    assert!(!control1.is_cancelled(), "first control should not be cancelled");
}

#[test]
fn app_context_degraded_is_not_configured() {
    use crasp_lib::runtime::{AppContext, ServiceState};
    let ctx = AppContext::degraded();
    let status = ctx.to_app_status();
    assert_eq!(status.mongo_state, ServiceState::NotConfigured);
    assert_eq!(status.zyte_state, ServiceState::NotConfigured);
}

#[test]
fn archived_page_includes_crawl_id() {
    use crasp_lib::crawler::{ArchivedPage, PageStatus};
    let page = ArchivedPage {
        url: "https://example.com/".to_string(),
        depth: 0,
        status: PageStatus::Completed,
        title: "Example".to_string(),
        content: Some("content".to_string()),
        hash: Some("abc".to_string()),
        hash_algorithm: Some("sha256".to_string()),
        discovered_links: 0,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        crawl_id: Some("crawl_12345".to_string()),
    };
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json["crawl_id"], "crawl_12345");
}

#[test]
fn jl_parse_with_crawl_id_filter() {
    use crasp_lib::commands::{append_to_jl, parse_jl_pages};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("crawl-test.jl");

    let page1 = serde_json::json!({
        "url": "https://example.com/page1",
        "depth": 0, "status": "Completed", "title": "Page 1",
        "content": "content 1", "hash": "h1", "hash_algorithm": "sha256",
        "discovered_links": 5, "timestamp": "2026-01-01T00:00:00Z",
        "crawl_id": "crawl_111"
    });
    let page2 = serde_json::json!({
        "url": "https://example.com/page2",
        "depth": 0, "status": "Completed", "title": "Page 2",
        "content": "content 2", "hash": "h2", "hash_algorithm": "sha256",
        "discovered_links": 3, "timestamp": "2026-01-01T00:00:01Z",
        "crawl_id": "crawl_222"
    });

    append_to_jl(&path, &[page1, page2]).unwrap();

    let all = parse_jl_pages(&path, None).unwrap();
    assert_eq!(all.len(), 2, "no filter => 2 pages");

    let filtered = parse_jl_pages(&path, Some("crawl_111")).unwrap();
    assert_eq!(filtered.len(), 1, "crawl_111 filter => 1 page");
    assert_eq!(filtered[0].url, "https://example.com/page1");

    let empty = parse_jl_pages(&path, Some("nonexistent")).unwrap();
    assert_eq!(empty.len(), 0);
}

#[test]
fn jl_append_and_parse_roundtrip() {
    use crasp_lib::commands::{append_to_jl, parse_jl_pages};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("crawl-roundtrip.jl");

    let items = vec![
        serde_json::json!({
            "url": "https://example.com/1", "depth": 0,
            "status": "Completed", "title": "Page 1",
            "content": "content 1", "hash": "h1",
            "hash_algorithm": "sha256", "discovered_links": 0,
            "timestamp": "2026-01-01T00:00:00Z", "crawl_id": "crawl_test"
        }),
        serde_json::json!({
            "url": "https://example.com/2", "depth": 1,
            "status": {"Failed": "HTTP 404"}, "title": "Page 2",
            "content": "", "hash": null, "hash_algorithm": null,
            "discovered_links": 0, "timestamp": "2026-01-01T00:00:01Z",
            "crawl_id": "crawl_test"
        }),
    ];

    append_to_jl(&path, &items).unwrap();

    let pages = parse_jl_pages(&path, None).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].stage, "Completed");
    assert_eq!(pages[1].stage, "Failed");
    assert_eq!(pages[1].status_reason, Some("HTTP 404".to_string()));
}

#[test]
fn crawl_outcomes_cancelled_summary() {
    use crasp_lib::commands::SharedCrawlOutcomes;

    let shared = SharedCrawlOutcomes::default();
    shared.pages_completed.store(3, Ordering::SeqCst);

    let summary = shared.build_crawl_done_summary(3, true, "crawl_cancelled");
    assert!(summary.cancelled);
    assert_eq!(summary.pages_archived, 3);
    assert_eq!(summary.pages_completed, 3);
}

#[test]
fn crawl_outcomes_storage_both() {
    use crasp_lib::commands::SharedCrawlOutcomes;

    let shared = SharedCrawlOutcomes::default();
    shared.pages_completed.store(5, Ordering::SeqCst);
    shared.used_mongo.store(true, Ordering::SeqCst);
    *shared.local_file_path.lock() = Some("/tmp/crawl-test.jl".to_string());

    let summary = shared.build_crawl_done_summary(5, false, "crawl_both");
    match summary.storage_used.unwrap() {
        crasp_lib::commands::StorageUsed::Both { local_path } => {
            assert_eq!(local_path, "/tmp/crawl-test.jl");
        }
        other => panic!("Expected Both, got {:?}", other),
    }
}

#[test]
fn crawl_outcomes_only_mongo() {
    use crasp_lib::commands::SharedCrawlOutcomes;

    let shared = SharedCrawlOutcomes::default();
    shared.pages_completed.store(5, Ordering::SeqCst);
    shared.used_mongo.store(true, Ordering::SeqCst);

    let summary = shared.build_crawl_done_summary(5, false, "crawl_mongo");
    match summary.storage_used.unwrap() {
        crasp_lib::commands::StorageUsed::Mongo => {}
        other => panic!("Expected Mongo, got {:?}", other),
    }
}

#[test]
fn crawl_outcomes_only_local() {
    use crasp_lib::commands::SharedCrawlOutcomes;

    let shared = SharedCrawlOutcomes::default();
    shared.pages_completed.store(5, Ordering::SeqCst);
    *shared.local_file_path.lock() = Some("/tmp/crawl.jl".to_string());

    let summary = shared.build_crawl_done_summary(5, false, "crawl_local");
    match summary.storage_used.unwrap() {
        crasp_lib::commands::StorageUsed::LocalFile { path } => {
            assert_eq!(path, "/tmp/crawl.jl");
        }
        other => panic!("Expected LocalFile, got {:?}", other),
    }
}

#[test]
fn crawl_outcomes_no_storage() {
    use crasp_lib::commands::SharedCrawlOutcomes;

    let shared = SharedCrawlOutcomes::default();
    shared.pages_completed.store(0, Ordering::SeqCst);

    let summary = shared.build_crawl_done_summary(0, false, "crawl_empty");
    assert!(summary.storage_used.is_none(), "no storage when nothing was persisted");
}

#[test]
fn local_fallback_path_format() {
    let path = crasp_lib::commands::local_fallback_path("/tmp/data", "crawl_12345");
    let expected = if cfg!(windows) { "/tmp/data\\crawl-crawl_12345.jl" } else { "/tmp/data/crawl-crawl_12345.jl" };
    assert_eq!(path.to_string_lossy(), expected);
}
