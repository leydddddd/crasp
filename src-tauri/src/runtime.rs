use std::env;

use crate::store::ArchiveStore;
use crate::zyte::ZyteClient;

pub struct AppContext {
    pub store: ArchiveStore,
    pub zyte: Option<ZyteClient>,
    pub http: reqwest::Client,
}

impl AppContext {
    pub async fn from_env() -> Result<Self, String> {
        let mongo_uri = env::var("CRASP_MONGO_URI")
            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string());

        let store = ArchiveStore::from_uri(&mongo_uri).await?;
        store.ensure_indexes().await?;

        let zyte = env::var("ZYTE_API_KEY").ok().and_then(|api_key| {
            if api_key.is_empty() {
                None
            } else {
                Some(ZyteClient::new(api_key))
            }
        });

        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(8)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build shared HTTP client");

        Ok(Self { store, zyte, http })
    }
}
