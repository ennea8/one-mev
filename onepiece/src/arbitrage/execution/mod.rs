pub mod execution;
pub use execution::*;

use alloy::pubsub::PubSubFrontend;
use alloy::rpc::types::eth::AccessList;
use alloy_provider::RootProvider;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::sync::broadcast::{self, Sender};

use crate::arbitrage::types::ActionEvent;

pub async fn action_handler(provider: Arc<RootProvider<PubSubFrontend>>, action_sender: Sender<ActionEvent>) -> Result<()> {
    let executor = Executor::new(provider.clone());
    // info!("action_handler subscribe");
    let mut action_receiver = action_sender.subscribe();

    loop {
        match action_receiver.recv().await {
            Ok(event) => {
                // info!("action_handler event: {:?}", event);
                match event {
                    ActionEvent::Backrun(action) => {
                        info!("in action_handler");
                        let sando_bundle = match executor
                            .create_sando_bundle_backrun(
                                action.pending_txs,
                                  action.back_calldata,
                                AccessList::default(),
                                action.realistic_back_gas_limit,
                                action.max_priority_fee_per_gas,
                                action.max_fee_per_gas,
                                None,
                            )
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => {
                                error!("❗❌ [create_sando_bundle_backrun] error: {:?}", e);
                                return Err(anyhow!("❗❌ [create_sando_bundle_backrun] error: {:?}", e));
                            }
                        };

                        // simulate bundle
                        // match executor.simulate_bundle(sando_bundle.clone(), new_block.block_number).await {
                        //     Ok(result) => {
                        //         info!("🟢🟢🟢 [simulate_bundle] success: {:?}", result);
                        //     }
                        //     Err(e) => {
                        //         warn!("❗❌ [simulate_bundle] error: {:?}", e);
                        //     }
                        // }

                        // broadcast bundle
                        match executor.broadcast_bundle(sando_bundle.clone(), action.new_block.clone()).await {
                            Ok(result) => {
                                // TODO move result logic from execution.rs to here
                                info!("[broadcast_bundle] success: {:?}", result);
                            }
                            Err(e) => {
                                error!("❗❌ [broadcast_bundle] error: {:?}", e);
                            }
                        }
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("action_handler Receiver lagged behind by {} messages", n);
                // Handle lagging receiver
            }
            Err(e) => {
                error!("action_handler Receive error: {}", e);
                // Handle other errors
            }
        }
    }
    Ok(())
}
