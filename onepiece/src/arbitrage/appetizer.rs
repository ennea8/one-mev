use anyhow::{anyhow, Result};
use dashmap::DashMap;
use std::sync::Arc;

use alloy::{
    primitives::{TxHash, I256, U256},
    providers::Provider,
};

//revm

use crate::arbitrage::pools::PoolManager;
use crate::arbitrage::types::{Arbitrage, Piece};
use crate::arbitrage::types::{NewBlock, SwapInfo};
use crate::simulation::simulator::{SimulatorFactory, Tx, VictimTx};

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
