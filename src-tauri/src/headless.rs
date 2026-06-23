use std::path::PathBuf;

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures_util::StreamExt;

pub struct RenderedPage {
    pub html: String,
    pub final_url: String,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct HeadlessConfig {
    pub wait_for_idle_ms: u64,
    pub timeout_ms: u64,
    pub load_images: bool,
    pub user_agent: Option<String>,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            wait_for_idle_ms: 2000,
            timeout_ms: 30000,
            load_images: false,
            user_agent: None,
        }
    }
}

pub fn find_chrome_binary() -> Option<PathBuf> {
    let auto_found = chromiumoxide::detection::default_executable(
        chromiumoxide::detection::DetectionOptions::default(),
    ).ok();

    let mut windows_paths: Vec<std::path::PathBuf> = vec![
        r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into(),
        r"C:\Program Files\Chromium\Application\chrome.exe".into(),
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe".into(),
        r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe".into(),
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into(),
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
    ];

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let lad = std::path::PathBuf::from(&local_app_data);
        windows_paths.push(lad.join("Google\\Chrome\\Application\\chrome.exe"));
        windows_paths.push(lad.join("Microsoft\\Edge\\Application\\msedge.exe"));
    }

    let candidate = auto_found.or_else(|| {
        windows_paths.iter().find(|p| p.exists()).cloned()
    });

    candidate.and_then(|path| {
        let metadata = std::fs::metadata(&path).ok()?;
        if metadata.len() < 100_000 {
            eprintln!(
                "[headless] Warning: Chrome binary at {} is suspiciously small ({} bytes) — may be a launcher stub",
                path.display(), metadata.len()
            );
        }
        Some(path)
    })
}

pub struct HeadlessBrowser {
    browser: Browser,
    handler_handle: tokio::task::JoinHandle<()>,
    config: HeadlessConfig,
}

impl HeadlessBrowser {
    pub async fn launch(config: HeadlessConfig) -> Result<Self, String> {
        let chrome_path = find_chrome_binary()
            .ok_or("Chrome/Chromium binary not found. Install Chrome to enable deep fetch.".to_string())?;

        eprintln!("[headless] Chrome binary: {}", chrome_path.display());
        if let Ok(metadata) = std::fs::metadata(&chrome_path) {
            eprintln!("[headless] Chrome binary size: {} bytes", metadata.len());
        }

        // Do NOT call disable_default_args() — chromiumoxide's DEFAULT_ARGS
        // include --disable-dev-shm-usage, --disable-extensions, --no-first-run,
        // and other flags needed for a stable headless session.
        //
        // chromiumoxide automatically:
        //   - adds --remote-debugging-port=0  (port 0 = OS picks free port)
        //   - adds --headless --hide-scrollbars --mute-audio  (since headless: true by default)
        //   - adds --no-sandbox --disable-setuid-sandbox  (via no_sandbox())
        //   - creates a temp --user-data-dir  (avoiding profile lock issues)
        //
        // We only add args NOT already present in DEFAULT_ARGS.

        let builder = BrowserConfig::builder()
            .no_sandbox()
            .chrome_executable(&chrome_path)
            .arg("--disable-gpu")
            .arg("--no-default-browser-check");

        let builder = if !config.load_images {
            builder.arg("--blink-settings=imagesEnabled=false")
        } else {
            builder
        };

        let browser_config = builder
            .build()
            .map_err(|e| format!("Failed to configure Chromium: {}", e))?;

        eprintln!("[headless] Launching Chrome...");

        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|e| format!("Failed to launch Chromium: {}", e))?;

        eprintln!("[headless] Chrome launched successfully");

        let handler_handle = tokio::spawn(async move {
            loop {
                if handler.next().await.is_none() {
                    break;
                }
            }
        });

        Ok(Self {
            browser,
            handler_handle,
            config,
        })
    }

    pub async fn render_page(&self, url: &str) -> Result<RenderedPage, String> {
        let page = self
            .browser
            .new_page("about:blank")
            .await
            .map_err(|e| format!("Failed to open tab: {}", e))?;

        if let Some(ref ua) = self.config.user_agent {
            page.set_user_agent(ua)
                .await
                .map_err(|e| format!("Failed to set user agent: {}", e))?;
        }

        eprintln!("[headless] Navigating to {} (timeout: {}ms)...", url, self.config.timeout_ms);

        let nav_result = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.timeout_ms),
            page.goto(url),
        )
        .await;

        match &nav_result {
            Ok(Ok(_)) => eprintln!("[headless] Navigation succeeded"),
            Ok(Err(e)) => eprintln!("[headless] Navigation error: {}", e),
            Err(_) => eprintln!("[headless] Navigation timed out after {}ms", self.config.timeout_ms),
        }

        let _nav_result = nav_result
            .map_err(|_| {
                format!(
                    "Navigation timeout after {}ms for {}",
                    self.config.timeout_ms, url
                )
            })?
            .map_err(|e| format!("Navigation failed for {}: {}", url, e))?;

        eprintln!("[headless] Waiting for idle ({}ms)...", self.config.wait_for_idle_ms);

        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.wait_for_idle_ms),
            page.wait_for_navigation(),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(
            self.config.wait_for_idle_ms / 2,
        ))
        .await;

        eprintln!("[headless] Getting page content...");

        let html = page
            .content()
            .await
            .map_err(|e| format!("Failed to get page content for {}: {}", url, e))?;

        eprintln!("[headless] Page content retrieved: {} chars", html.len());

        let final_url = match page.url().await {
            Ok(Some(u)) => {
                eprintln!("[headless] Final URL: {}", u);
                u
            }
            Ok(None) => {
                eprintln!("[headless] No final URL returned");
                url.to_string()
            }
            Err(e) => {
                eprintln!("[headless] URL retrieval error: {}", e);
                url.to_string()
            }
        };

        page.close().await.ok();

        Ok(RenderedPage {
            html,
            final_url,
            status_code: None,
        })
    }

    pub fn close(self) {
        self.handler_handle.abort();
    }
}

impl Drop for HeadlessBrowser {
    fn drop(&mut self) {
        self.handler_handle.abort();
    }
}
