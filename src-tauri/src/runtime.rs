use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::store::ArchiveStore;
use crate::zyte::{ZyteClient, DeepFetchConfig, DeepFetchQueue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    NotConfigured,
    ConfiguredUnverified,
    Connected,
    Unreachable,
}

#[derive(Clone, serde::Serialize)]
pub struct AppStatus {
    pub mongo_state: ServiceState,
    pub mongo_detail: Option<String>,
    pub zyte_state: ServiceState,
    pub zyte_detail: Option<String>,
    pub zyte_project: Option<String>,
    pub zyte_available: bool,
    pub deep_fetch_enabled: bool,
    pub chrome_available: bool,
}

pub struct AppContext {
    pub store: Option<Arc<ArchiveStore>>,
    pub zyte: Option<Arc<ZyteClient>>,
    pub zyte_project: Option<String>,
    #[allow(dead_code)]
    pub http: Arc<reqwest::Client>,
    pub deep_fetch_config: DeepFetchConfig,
    pub deep_fetch_queue: DeepFetchQueue,
    mongo_state: AtomicU8,
    mongo_detail: parking_lot::Mutex<Option<String>>,
    zyte_state: AtomicU8,
    zyte_detail: parking_lot::Mutex<Option<String>>,
}

impl ServiceState {
    fn to_u8(self) -> u8 {
        match self {
            ServiceState::NotConfigured => 0,
            ServiceState::ConfiguredUnverified => 1,
            ServiceState::Connected => 2,
            ServiceState::Unreachable => 3,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            2 => ServiceState::Connected,
            3 => ServiceState::Unreachable,
            1 => ServiceState::ConfiguredUnverified,
            _ => ServiceState::NotConfigured,
        }
    }
}

impl AppContext {
    pub async fn from_env() -> Result<Self, String> {
        let mongo_uri_opt = env::var("CRASP_MONGO_URI").ok().filter(|s| !s.is_empty());

        let (store, mongo_initial) = if let Some(mongo_uri) = mongo_uri_opt {
            match ArchiveStore::from_uri(&mongo_uri).await {
                Ok(s) => {
                    if let Err(e) = s.ensure_indexes().await {
                        eprintln!("Warning: index creation failed: {}", e);
                    }
                    (Some(Arc::new(s)), ServiceState::ConfiguredUnverified)
                }
                Err(_) => {
                    (None, ServiceState::NotConfigured)
                }
            }
        } else {
            (None, ServiceState::NotConfigured)
        };

        let zyte = env::var("ZYTE_API_KEY").ok().and_then(|api_key| {
            if api_key.is_empty() {
                None
            } else {
                Some(Arc::new(ZyteClient::new(api_key)))
            }
        });

        let zyte_initial = if zyte.is_some() {
            ServiceState::ConfiguredUnverified
        } else {
            ServiceState::NotConfigured
        };

        let zyte_project = env::var("CRASP_ZYTE_PROJECT").ok().filter(|s| !s.is_empty());

        let http = Arc::new(
            reqwest::Client::builder()
                .pool_max_idle_per_host(8)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build shared HTTP client"),
        );

        let deep_fetch_config = DeepFetchConfig::from_env();
        let deep_fetch_queue = DeepFetchQueue::new(
            deep_fetch_config.request_delay_ms,
            deep_fetch_config.max_per_crawl,
        );

        Ok(Self {
            store,
            zyte,
            zyte_project,
            http,
            deep_fetch_config,
            deep_fetch_queue,
            mongo_state: AtomicU8::new(mongo_initial.to_u8()),
            mongo_detail: parking_lot::Mutex::new(None),
            zyte_state: AtomicU8::new(zyte_initial.to_u8()),
            zyte_detail: parking_lot::Mutex::new(None),
        })
    }

    pub fn degraded() -> Self {
        let http = Arc::new(
            reqwest::Client::builder()
                .pool_max_idle_per_host(8)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build shared HTTP client"),
        );

        let deep_fetch_config = DeepFetchConfig::default();
        let deep_fetch_queue = DeepFetchQueue::new(
            deep_fetch_config.request_delay_ms,
            deep_fetch_config.max_per_crawl,
        );

        Self {
            store: None,
            zyte: None,
            zyte_project: None,
            http,
            deep_fetch_config,
            deep_fetch_queue,
            mongo_state: AtomicU8::new(ServiceState::NotConfigured.to_u8()),
            mongo_detail: parking_lot::Mutex::new(None),
            zyte_state: AtomicU8::new(ServiceState::NotConfigured.to_u8()),
            zyte_detail: parking_lot::Mutex::new(None),
        }
    }

    pub fn get_mongo_state(&self) -> ServiceState {
        ServiceState::from_u8(self.mongo_state.load(Ordering::SeqCst))
    }

    pub fn get_zyte_state(&self) -> ServiceState {
        ServiceState::from_u8(self.zyte_state.load(Ordering::SeqCst))
    }

    pub fn get_mongo_detail(&self) -> Option<String> {
        self.mongo_detail.lock().clone()
    }

    pub fn get_zyte_detail(&self) -> Option<String> {
        self.zyte_detail.lock().clone()
    }

    pub fn set_mongo_state(&self, state: ServiceState, detail: Option<String>) {
        self.mongo_state.store(state.to_u8(), Ordering::SeqCst);
        *self.mongo_detail.lock() = detail;
    }

    pub fn set_zyte_state(&self, state: ServiceState, detail: Option<String>) {
        self.zyte_state.store(state.to_u8(), Ordering::SeqCst);
        *self.zyte_detail.lock() = detail;
    }

    pub fn to_app_status(&self) -> AppStatus {
        AppStatus {
            mongo_state: self.get_mongo_state(),
            mongo_detail: self.get_mongo_detail(),
            zyte_state: self.get_zyte_state(),
            zyte_detail: self.get_zyte_detail(),
            zyte_project: self.zyte_project.clone(),
            zyte_available: self.zyte.is_some(),
            deep_fetch_enabled: self.deep_fetch_config.enabled,
            chrome_available: self.deep_fetch_config.chrome_available,
        }
    }
}
