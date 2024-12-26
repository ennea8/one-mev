use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, Value};

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::{error, fs};

use futures_util::StreamExt;
use tokio::sync::broadcast::Sender;

use alloy::pubsub::PubSubFrontend;
use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{address, Address, Bytes, TxHash, U128, U256, U64},
    providers::Provider,
    rpc::types::eth::{Block, Filter, Log, Transaction},
};
use alloy_provider::RootProvider;
use alloy_transport_ws::WsConnect;

use crate::utils::calculate_next_block_base_fee;

use crate::arbitrage::config::blacklist::SENDER_BLACK_LIST;
use crate::arbitrage::types::{DexVariant, Event, NewBlock, NewPendingTx};

pub async fn stream_new_blocks(provider: Arc<RootProvider<PubSubFrontend>>, event_sender: Sender<Event>) -> Result<()> {
    let sub = provider.subscribe_blocks().await?;
    let stream = sub.into_stream(); //.take(2);

    let mut stream = Box::pin(stream.filter_map(|block| async move {
        match block.header.number {
            Some(number) => Some(NewBlock {
                block_number: number,
                base_fee: U256::try_from(block.header.base_fee_per_gas.unwrap()).unwrap(),
                next_base_fee: calculate_next_block_base_fee(
                    U256::from(block.header.gas_used),
                    U256::from(block.header.gas_limit),
                    U256::from(block.header.base_fee_per_gas.unwrap_or_default()),
                ),
            }),
            None => None,
        }
    }));

    while let Some(block) = stream.next().await {
        // info!("new_block_sending");
        match event_sender.send(Event::Block(block)) {
            Ok(_) => {}
            Err(err) => {
                info!("new_block_sending error {err}");
            }
        }
    }

    Ok(())
}

pub async fn stream_pending_transactions(provider: Arc<RootProvider<PubSubFrontend>>, event_sender: Sender<Event>) -> Result<()> {
    let stream = provider.subscribe_full_pending_transactions().await.unwrap();
    let mut stream = stream.into_stream();

    while let Some(result) = stream.next().await {
        // black list sender
        if SENDER_BLACK_LIST.contains(&result.from) {
            continue;
        }

        match event_sender.send(Event::PendingTx(NewPendingTx { added_block: None, tx: result })) {
            Ok(_) => {}
            Err(err) => {
                info!("pending_transactions_sending error {err}");
            }
        }
    }
    Ok(())
}
