use std::env;
use std::sync::Arc;

use crate::store::ArchiveStore;
use crate::zyte::ZyteClient;

pub struct AppContext {
    pub store: Option<Arc<ArchiveStore>>,
    pub zyte: Option<Arc<ZyteClient>>,
    pub zyte_project: Option<String>,
    #[allow(dead_code)]
    pub http: Arc<reqwest::Client>,
}

impl AppContext {
    pub async fn from_env() -> Result<Self, String> {
        let mongo_uri = env::var("CRASP_MONGO_URI")
            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string());

        let store = match ArchiveStore::from_uri(&mongo_uri).await {
            Ok(s) => {
                if let Err(e) = s.ensure_indexes().await {
                    eprintln!("Warning: index creation failed: {}", e);
                }
                Some(Arc::new(s))
            }
            Err(e) => {
                eprintln!("Warning: MongoDB connection failed: {}", e);
                None
            }
        };

        let zyte = env::var("ZYTE_API_KEY").ok().and_then(|api_key| {
            if api_key.is_empty() {
                None
            } else {
                Some(Arc::new(ZyteClient::new(api_key)))
            }
        });

        let zyte_project = env::var("CRASP_ZYTE_PROJECT").ok().filter(|s| !s.is_empty());

        let http = Arc::new(
            reqwest::Client::builder()
                .pool_max_idle_per_host(8)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build shared HTTP client"),
        );

        Ok(Self {
            store,
            zyte,
            zyte_project,
            http,
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

        Self {
            store: None,
            zyte: None,
            zyte_project: None,
            http,
        }
    }

    pub fn mongo_ok(&self) -> bool {
        self.store.is_some()
    }

    pub fn zyte_available(&self) -> bool {
        self.zyte.is_some()
    }
}
