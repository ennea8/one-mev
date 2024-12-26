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

//revm
use revm::db::{CacheDB, EmptyDB};
use revm::primitives::Bytecode;

use crate::abi::IOne;
use crate::arbitrage::config::constants::ethereum::weth_addr;
use crate::arbitrage::config::constants::{OWNER_ADDRESS, REVM_ONE_ADDRESS, REVM_ONE_SIMULATOR_ADDRESS};
use crate::arbitrage::execution::Executor;
use crate::arbitrage::pools::{load_pools, PoolManager};
use crate::arbitrage::types::{ActionEvent, BackrunAction};
use crate::arbitrage::types::{Arbitrage, One, Piece};
use crate::arbitrage::types::{Event, NewBlock, NewPendingTx, PendingTxInfo, SwapInfo};
use crate::common::bytecode::{ONE_BYTECODE, ONE_SIMULATOR_BYTECODE};
use crate::common::config::{get_app_config, get_global_config};
use crate::simulation::simulator::{Simulator, SimulatorFactory, Tx, TxResult, VictimTx};

pub async fn appetizer(
    evm_factory: Arc<SimulatorFactory>,
    new_block: NewBlock,
    victim_tx: VictimTx,
    swap_items: Vec<SwapInfo>,
    pool_manager: &PoolManager,
    promising_pieces: &mut DashMap<TxHash, Vec<Piece>>,
) -> Result<usize> {
    let mut sim = evm_factory.new_fork_simulator(false);
    sim.set_base_fee(new_block.next_base_fee);

    let mut piece_added = 0;

    // simulate victim_tx
    match sim.call(Tx::from(victim_tx.clone())) {
        Ok(result) => {
            info!("🟢📌 victim_tx success : {:?}", victim_tx.tx_hash);
        }
        Err(e) => {
            warn!("❗❌ victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e);
            return Err(anyhow!("❗❌ victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e));
        }
    }

    // generate swap paths for each swap_info
    for info in swap_items {
        let swap_paths = pool_manager.generate_swap_path_from_touched_pool(&info.target_pair, info.direction.clone());

        sim.set_base_fee(U256::ZERO);

        // find profitable path and find optimal amount
        let arbi = sim.find_profitable_path_and_opt_amount(swap_paths);

        match arbi {
            Ok(Some(the_arbi)) => {
                // update promising_txs
                if the_arbi.max_revenue > I256::ZERO {
                    let arbitrage = Arbitrage {
                        path: the_arbi.path.clone(),
                        optimized_in: the_arbi.optimized_in.clone(),
                        max_revenue: the_arbi.max_revenue, // check need clone?
                    };
                    let piece: Piece = Piece {
                        swap_info: info.clone(),
                        victim_tx: victim_tx.clone(),
                        arbitrage: Some(arbitrage),
                        sandwich: None,
                        updated_at: new_block.block_number,
                    };
                    promising_pieces.entry(victim_tx.tx_hash).or_insert_with(Vec::new).push(piece.clone());

                    piece_added += 1;
                }
            }
            Ok(None) => {
                debug!("[find_arbi_paths] no profitable path found, continue");
                continue;
            }
            Err(e) => {
                warn!("❗❌ [find_arbi_paths] error: {:?}", e);
                continue;
            }
        }
    }

    Ok(piece_added)
}
