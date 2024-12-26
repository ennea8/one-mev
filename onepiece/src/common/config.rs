use alloy::primitives::Address;
use alloy_signer_local::PrivateKeySigner;
use once_cell::sync::OnceCell;
use one_config::Config;
use reqwest::Url;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub searcher_signer: PrivateKeySigner,
    pub bundle_signer: PrivateKeySigner,
    pub bot_address: Address,

    pub wss_rpc: Url,
    pub http_rpc: Url,

    pub chain_id: u64,

    // added
    pub debug: bool,
}

static GLOBAL_CONFIG: OnceCell<Arc<AppConfig>> = OnceCell::new();

// get a clone of the app config
pub fn get_app_config() -> AppConfig {
    let config = Config::get_config().unwrap();

    let app_config = AppConfig {
        searcher_signer: config.searcher_signer,
        bundle_signer: config.bundle_signer,
        bot_address: config.bot_address,
        wss_rpc: config.wss_rpc,
        http_rpc: config.http_rpc,

        chain_id: std::env::var("CHAIN_ID").unwrap().parse::<u64>().unwrap(),

        // extra
        debug: std::env::var("DEBUG").map(|v| v.to_lowercase() == "true").unwrap_or(true),
    };

    app_config
}

pub fn init_global_config() {
    // build app config
    let config = Config::get_config().unwrap();

    let app_config = AppConfig {
        searcher_signer: config.searcher_signer,
        bundle_signer: config.bundle_signer,
        bot_address: config.bot_address,
        wss_rpc: config.wss_rpc,
        http_rpc: config.http_rpc,

        chain_id: std::env::var("CHAIN_ID").unwrap().parse::<u64>().unwrap(),

        // extra
        debug: std::env::var("DEBUG").map(|v| v.to_lowercase() == "true").unwrap_or(true),
    };

    GLOBAL_CONFIG.set(Arc::new(app_config)).expect("Global config already initialized");
}

pub fn get_global_config() -> Arc<AppConfig> {
    GLOBAL_CONFIG.get().expect("Global config not initialized").clone()
}

