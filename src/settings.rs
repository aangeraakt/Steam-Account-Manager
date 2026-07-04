use crate::proxy::ProxyEntry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub proxies: Vec<ProxyEntry>,
    pub default_proxy_id: Option<String>,
    pub register_country: String,
    pub proxy_fetch_country: String,
    pub proxy_host_template: String,
    pub proxy_start_port: u16,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            proxies: Vec::new(),
            default_proxy_id: None,
            register_country: "TR".to_string(),
            proxy_fetch_country: "TR".to_string(),
            proxy_host_template: "127.0.0.1".to_string(),
            proxy_start_port: 10000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppData {
    pub accounts: crate::accounts::AccountStore,
    pub settings: AppSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_turkey_proxy_defaults() {
        let settings = AppSettings::default();
        assert_eq!(settings.register_country, "TR");
        assert_eq!(settings.proxy_fetch_country, "TR");
        assert_eq!(settings.proxy_host_template, "127.0.0.1");
        assert_eq!(settings.proxy_start_port, 10000);
    }
}
