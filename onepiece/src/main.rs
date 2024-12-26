use anyhow::Result;
use std::sync::Arc;

use tokio::sync::broadcast::{self, Sender};
use tokio::task::JoinSet;

use tracing::{error, info};

use alloy::rpc::client::WsConnect;
use alloy_provider::{Provider, ProviderBuilder};

use one_common::{init_logs_v2, print_banner};

use onepiece::arbitrage::strategy::event_handler;
use onepiece::arbitrage::execution::action_handler;
use onepiece::arbitrage::streams::{stream_new_blocks, stream_pending_transactions};
use onepiece::arbitrage::types::{Event, ActionEvent};
use onepiece::common::config::{get_global_config, init_global_config};

#[tokio::main]
async fn main() -> Result<()> {
    let chain = std::env::var("chain").unwrap_or_else(|_| "eth".to_string());
    let chain_env = format!(".env.{}.arbitrage", chain);
    dotenv::from_filename(&chain_env).ok();

    let _log = init_logs_v2();
    print_banner();

    info!("-----Load Config-----!");

    init_global_config(); // TODO remove?
    let config = get_global_config();
    info!("Config: {:?}", config);

    // provider
    let ws = WsConnect::new(config.wss_rpc.clone());
    let provider = Arc::new(ProviderBuilder::new().on_ws(ws).await?);

    info!("-----add stream-----!");
    let (event_sender, _): (Sender<Event>, _) = broadcast::channel(512); 
    let (action_sender, _): (Sender<ActionEvent>, _) = broadcast::channel(512); 

    let mut set = JoinSet::new();

    // for geth like service
    set.spawn(stream_new_blocks(provider.clone(), event_sender.clone()));
    
    // set.spawn(stream_uniswap_v2(provider.clone(), event_sender.clone()));
    
    // for geth like service
    set.spawn(stream_pending_transactions(provider.clone(), event_sender.clone()));
    
    info!("-----add strategy-----!");
    set.spawn(event_handler(provider.clone(), event_sender.clone(),action_sender.clone()));

    info!("-----add action_handler-----!");
    set.spawn(action_handler(provider.clone(), action_sender.clone()));


    info!("-----start-----!");
    while let Some(result) = set.join_next().await {
        match result {
            Ok(Err(e)) => {
                error!("Error set.join_next  {:?}", e);
                return Err(e.into()); // Propagate the error to stop the program
            }
            Err(e) => {
                error!("Task failed: {:?}", e);
                return Err(e.into()); // Propagate the error to stop the program
            }
            Ok(_) => {
                // Process the file content here
                info!("set.join_next {:?}", result);
            }
        }
    }
    Ok(())
}
