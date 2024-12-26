use alloy::transports::ws::WsConnect;
use alloy::{
    primitives::{Address, U128, U256, U64},
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    pubsub::PubSubFrontend,
    rpc::types::eth::{Block, Log, Transaction},
};
use alloy_transport_http::Http;
use reqwest::Client;

use std::sync::Arc;

// TODO: remove this; wrapped in Arc, readonly
pub async fn create_default_wss_provider(
) -> Result<Arc<RootProvider<PubSubFrontend>>, anyhow::Error> {
    let wss_rpc = std::env::var("WSS_RPC")?;
    let url: &str = wss_rpc.as_str();
    let client = ProviderBuilder::new().on_ws(WsConnect::new(url)).await?;
    Ok(Arc::new(client))
}


pub async fn create_default_wss_provider2(
) -> Result<RootProvider<PubSubFrontend>, anyhow::Error> {
    let wss_rpc = std::env::var("WSS_RPC")?;
    let url: &str = wss_rpc.as_str();
    let client = ProviderBuilder::new().on_ws(WsConnect::new(url)).await?;
    Ok(client)
}

pub async fn create_default_http_provider() -> Result<Arc<ReqwestProvider>, anyhow::Error> {
    let provider = ProviderBuilder::new().on_http(std::env::var("HTTP_RPC").unwrap().parse()?);
    let provider = Arc::new(provider);
    Ok(provider)
}
