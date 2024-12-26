use alloy_provider::ext::DebugApi;
use anyhow::{anyhow, ensure, Error, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tokio::sync::broadcast::{self, Sender};

use alloy::pubsub::PubSubFrontend;
use alloy::{
    primitives::{utils::parse_ether, Address, Bytes, TxHash, I256, U128, U256, U64},
    providers::Provider,
    rpc::types::eth::{Block, Log, Transaction},
    rpc::types::trace::parity::TraceType,
};
use alloy_eips::eip2930::AccessList;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::BlockNumber;
use alloy_provider::RootProvider;
use alloy_transport_ws::WsConnect;
use bounded_vec_deque::BoundedVecDeque;

//revm
use revm::db::{CacheDB, EmptyDB};
use revm::primitives::Bytecode;

use crate::abi::IOne;
use crate::arbitrage::appetizer::appetizer;
use crate::arbitrage::config::constants::ethereum::weth_addr;
use crate::arbitrage::config::constants::{OWNER_ADDRESS, REVM_ONE_ADDRESS, REVM_ONE_SIMULATOR_ADDRESS};
use crate::arbitrage::execution::Executor;
use crate::arbitrage::main_dish::{main_dish, main_dish_on_new_block};
use crate::arbitrage::pools::{load_pools, PoolManager};
use crate::arbitrage::simulation::create_evm_factory;
use crate::arbitrage::types::{ActionEvent, BackrunAction};
use crate::arbitrage::types::{Arbitrage, One, Piece};
use crate::arbitrage::types::{Event, NewBlock, NewPendingTx, PendingTxInfo};
use crate::common::bytecode::{ONE_BYTECODE, ONE_SIMULATOR_BYTECODE};
use crate::common::config::{get_app_config, get_global_config};
use crate::simulation::simulator::{Simulator, SimulatorFactory, Tx, TxResult, VictimTx};

pub async fn event_handler(
    provider: Arc<RootProvider<PubSubFrontend>>,
    event_sender: Sender<Event>,
    action_sender: Sender<ActionEvent>,
) -> Result<()> {
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest, true).await.unwrap().unwrap();
    // let config = get_app_config();

    info!("event_handler subscribe");
    let mut event_receiver = event_sender.subscribe();

    let mut new_block = NewBlock {
        block_number: block.header.number.unwrap_or_default(),
        base_fee: U256::try_from(block.header.base_fee_per_gas.unwrap()).unwrap(),
        next_base_fee: U256::try_from(block.header.base_fee_per_gas.unwrap()).unwrap(), // same as base_fee at begin
    };

    let poolManager = load_pools().unwrap();

    info!("pools loaded len: {:?}", poolManager.pools.len());

    let mut pending_txs: DashMap<TxHash, PendingTxInfo> = DashMap::new();
    let mut promising_pieces: DashMap<TxHash, Vec<Piece>> = DashMap::new(); // support  muti vic tx  // TODO 多 middle tx支持
    let mut simulated_one_ids: BoundedVecDeque<String> = BoundedVecDeque::new(30);

    loop {
        match event_receiver.recv().await {
            Ok(event) => match event {
                Event::Block(block) => {
                    new_block = block;
                    info!("⭕🔱─── ⋆⋅☆⋅⋆ ── New Block: {:?}", new_block);

                    let block_with_txs =
                        provider.get_block_by_number(BlockNumberOrTag::Number(new_block.block_number), false).await.unwrap().unwrap();

                    let txs: Vec<TxHash> = block_with_txs
                        .transactions
                        .hashes()
                        .map(|tx_hash| *tx_hash) // Assuming tx has a field `hash`
                        .collect();

                    // remove txs already on chain
                    for tx_hash in &txs {
                        if pending_txs.contains_key(tx_hash) {
                            let removed = pending_txs.remove(tx_hash).unwrap();
                            promising_pieces.remove(tx_hash);
                            info!("[TX REMOVED]: {:?} / Pending txs len: {:?}", tx_hash, pending_txs.len());
                        }
                    }
                    // remove pending txs older than 5 blocks
                    pending_txs.retain(|_, v| (new_block.block_number - v.pending_tx.added_block.unwrap()) < 5u64);
                    let pending_tx_keys: Vec<_> = pending_txs.iter().map(|entry| entry.key().clone()).collect();
                    promising_pieces.retain(|h, _| pending_tx_keys.contains(h));

                    // update revenue info for pending_txs not removed
                    {

                        let new_block = new_block.clone();
                        let evm_factory = create_evm_factory(provider.clone(), new_block.block_number).unwrap();
                        let mut promising_pieces = promising_pieces.clone();
                        let pending_txs = pending_txs.clone();

                        tokio::spawn(async move {
                            // let evm_factory = create_evm_factory(provider.clone(), new_block.block_number).unwrap();
                            main_dish_on_new_block(evm_factory.clone(), new_block.clone(), &mut promising_pieces, &pending_txs).await;
                        });
                    }
                }
                Event::PendingTx(mut pending_tx) => {
                    let tx_hash = pending_tx.tx.hash;

                    //info!("tx received: {:?}", tx_hash);

                    let mut should_add = false;

                    //check if already received and confirmed
                    {
                        let already_received = pending_txs.contains_key(&tx_hash);
                        if already_received {
                            // check if it's still in pending_txs
                            let tx_receipt = provider.get_transaction_receipt(tx_hash).await;
                            match tx_receipt {
                                Ok(receipt) => match receipt {
                                    Some(_) => {
                                        // returning a receipt means that the tx is confirmed
                                        // should not be in pending_txs
                                        pending_txs.remove(&tx_hash);
                                        debug!("tx received again, and already confirmed: {:?}", tx_hash);
                                    }
                                    None => {}
                                },
                                _ => {}
                            }

                            debug!("tx received again, and already confirmed: {:?}", tx_hash);
                            continue;
                        }
                    }

                    // check gas price
                    let mut victim_gas_price = U256::ZERO;
                    match pending_tx.tx.transaction_type {
                        Some(tx_type) => {
                            if tx_type == 0 {
                                victim_gas_price = U256::from(pending_tx.tx.gas_price.unwrap_or_default());
                                should_add = victim_gas_price >= new_block.next_base_fee;
                            // TODO check next_base_fee or base_fee?
                            } else if tx_type == 2 {
                                victim_gas_price = U256::from(pending_tx.tx.max_fee_per_gas.unwrap_or_default());
                                should_add = victim_gas_price >= new_block.next_base_fee;
                            }
                        }
                        _ => {}
                    }
                    if !should_add {
                        debug!("tx gas price < base fee, ignore: {:?}", tx_hash);
                        continue;
                    }

                    // get touched pools
                    let swap_info =
                        match poolManager.get_touched_pools_by_debug_trace_call(provider.clone(), &pending_tx.tx, &new_block).await {
                            Ok(result) => {
                                result
                                //info!("get_touched_pools_by_debug_trace_call success: {:?}", result);
                            }
                            Err(e) => {
                                warn!("get_touched_pools_by_debug_trace_call error: {:?}", e);
                                vec![]
                            }
                        };

                    if swap_info.is_empty() {
                        continue;
                    }
                    info!("swap_info: {:?}", swap_info);

                    let evm_factory = create_evm_factory(provider.clone(), new_block.block_number).unwrap();

                    // update pending_txs/pending_tx
                    pending_tx.added_block = Some(new_block.block_number);
                    let pending_tx_info = PendingTxInfo { pending_tx: pending_tx.clone(), touched_pairs: swap_info.clone() };
                    pending_txs.insert(pending_tx.tx.hash, pending_tx_info.clone());

                    let victim_tx = VictimTx::from_transaction(pending_tx.tx.clone());

                    let piece_added = match appetizer(
                        evm_factory.clone(),
                        new_block.clone(),
                        victim_tx.clone(),
                        swap_info.clone(),
                        &poolManager,
                        &mut promising_pieces,
                    )
                    .await
                    {
                        Ok(piece_added) => piece_added,
                        Err(e) => {
                            error!("❗❌ [appetizer] error: {:?}", e);
                            continue;
                        }
                    };

                    if piece_added == 0 || promising_pieces.is_empty() {
                        continue;
                    }

                    match main_dish(
                        evm_factory.clone(),
                        new_block.clone(),
                        &mut promising_pieces,
                        &pending_txs,
                        &mut simulated_one_ids,
                        action_sender.clone(),
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(e) => {
                            error!("❗❌ [main_dish] error: {:?}", e);
                            continue;
                        }
                    }
                }
            },
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("Receiver lagged behind by {} messages", n);
                // Handle lagging receiver
            }
            Err(e) => {
                error!("Receive error: {}", e);
                // Handle other errors
            }
        }
    }
    Ok(())
}