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
    if cfg!(target_os = "windows") {
        let paths = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ];
        for p in &paths {
            let pb = PathBuf::from(p);
            if pb.exists() {
                return Some(pb);
            }
        }
        None
    } else if cfg!(target_os = "macos") {
        let paths = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ];
        for p in &paths {
            let pb = PathBuf::from(p);
            if pb.exists() {
                return Some(pb);
            }
        }
        None
    } else {
        let binaries = ["google-chrome", "google-chrome-stable", "chromium-browser", "chromium", "microsoft-edge"];
        for bin_name in &binaries {
            if let Ok(path) = which::which(bin_name) {
                return Some(path);
            }
        }
        None
    }
}

pub struct HeadlessBrowser {
    browser: Browser,
    handler_handle: tokio::task::JoinHandle<()>,
    config: HeadlessConfig,
}

impl HeadlessBrowser {
    pub async fn launch(config: HeadlessConfig) -> Result<Self, String> {
        let chrome_path = find_chrome_binary()
            .ok_or("Chrome/Chromium binary not found. Install Chrome to enable deep fetch.")?;

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .no_sandbox()
            .disable_default_args();

        if !config.load_images {
            builder = builder.arg("--blink-settings=imagesEnabled=false");
        }

        builder = builder
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-translate");

        let browser_config = builder
            .build()
            .map_err(|e| format!("Failed to configure Chromium: {}", e))?;

        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|e| format!("Failed to launch Chromium: {}", e))?;

        let handler_handle = tokio::spawn(async move {
            loop {
                match StreamExt::next(&mut handler).await {
                    Some(_) => {}
                    None => break,
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

        let _nav_result = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.timeout_ms),
            page.goto(url),
        )
        .await
        .map_err(|_| {
            format!(
                "Navigation timeout after {}ms for {}",
                self.config.timeout_ms, url
            )
        })?
        .map_err(|e| format!("Navigation failed for {}: {}", url, e))?;

        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.wait_for_idle_ms),
            page.wait_for_navigation(),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(
            self.config.wait_for_idle_ms / 2,
        ))
        .await;

        let html = page
            .content()
            .await
            .map_err(|e| format!("Failed to get page content for {}: {}", url, e))?;

        let final_url = match page.url().await {
            Ok(Some(u)) => u,
            _ => url.to_string(),
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
