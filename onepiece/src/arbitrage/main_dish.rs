use alloy::{
    primitives::{utils::parse_ether, Address, Bytes, TxHash, I256, U128, U256, U64},
    providers::Provider,
    rpc::types::eth::{Block, Log, Transaction},
    rpc::types::trace::parity::TraceType,
};
use anyhow::Result;
use bounded_vec_deque::BoundedVecDeque;
use dashmap::DashMap;
use std::sync::Arc;

use crate::arbitrage::one::OneSimulateResult;
use crate::arbitrage::types::ActionEvent;
use crate::arbitrage::types::BackrunAction;
use crate::arbitrage::types::PendingTxInfo;
use crate::arbitrage::types::{NewBlock, One, Piece};
use crate::simulation::simulator::{Simulator, SimulatorFactory, Tx, TxResult, VictimTx};
use tokio::sync::broadcast::{self, Sender};

pub struct Ingredients {
    pub tx_hash: TxHash,
    // pub pair: Address,
    // pub main_currency: Address,
    pub amount_in: U256,
    pub max_revenue: I256,
    pub score: I256, // pub score: f64,
    pub piece: Piece,
}

pub async fn main_dish(
    evm_factory: Arc<SimulatorFactory>,
    new_block: NewBlock,
    promising_pieces: &mut DashMap<TxHash, Vec<Piece>>,
    pending_txs: &DashMap<TxHash, PendingTxInfo>,
    simulated_one_ids: &mut BoundedVecDeque<String>,
    action_sender: Sender<ActionEvent>,
) -> Result<()> {
    let mut plate = Vec::new();

    for entry in promising_pieces.iter() {
        let (promising_tx_hash, pieces) = entry.pair();

        // skip if revenue is simulated in old block
        if pieces.len() == 0 || pieces[0].updated_at < new_block.block_number {
            continue;
        }

        for piece in pieces {
            // let optimized_piece = piece.optimized_piece.as_ref().unwrap();
            let amount_in = piece.arbitrage.as_ref().unwrap().optimized_in;
            let max_revenue = piece.arbitrage.as_ref().unwrap().max_revenue;

            // TODO optimize in necessary.
            // score is related to financial utilization rate especially for sandwich
            // should ignore pieces with no profit.
            // max_revenue is ok currently
            let score = max_revenue;

            if score > I256::ZERO {
                let ingredients = Ingredients {
                    tx_hash: *promising_tx_hash,
                    // pair: piece.pair,
                    amount_in,
                    max_revenue,
                    score,
                    piece: piece.clone(),
                };
                plate.push(ingredients);
            }
        }
    }

    plate.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap());

    for i in 0..plate.len() {
        let mut pieces = Vec::new();

        for j in 0..(i + 1) {
            let ingredient = &plate[j];
            let optimized = ingredient.amount_in;

            let mut piece = ingredient.piece.clone();
            piece.arbitrage.as_mut().unwrap().optimized_in = optimized;
            pieces.push(piece);
        }

        let one = One { pieces };

        let one_id = one.one_id();
        if simulated_one_ids.contains(&one_id) {
            continue;
        }

        simulated_one_ids.push_back(one_id.clone());

        //simulate on line version // TODO get access_list?

        let result = match one.simulate(evm_factory.clone(), new_block.clone()).await {
            Ok(result) => result,
            Err(e) => {
                error!("❗❌ [main_dish] one.simulate error: {:?}", e);
                continue;
            }
        };

        let OneSimulateResult { revenue, profit, gas_used, gas_cost, calldata } = result;

        if revenue <= I256::ZERO {
            info!("[main_dish_low_revenue]  revenue {:?} / profit {:?} / gas_cost {:?} / gas_used {:?}", revenue, profit, gas_cost, gas_used);
            continue;
        }

        let base_fee = new_block.next_base_fee;
        let revenue_u256: U256 = revenue.into_raw().try_into().unwrap();
        let bribe_pct = U256::from(10000);
        let bribe_amount = (revenue_u256 * bribe_pct) / U256::from(10000);
        let realistic_back_gas_limit = (gas_used * 105) / 100;
        let max_priority_fee_per_gas = bribe_amount / U256::from(realistic_back_gas_limit);
        let max_fee_per_gas = base_fee + max_priority_fee_per_gas;

        info!("⭕⭕⭕ BaseFee: {:?} / PriorityFee: {:?} / MaxFee: {:?} ", base_fee, max_priority_fee_per_gas, max_fee_per_gas);
        info!("⭕⭕⭕ Revenue: {:?} / Profit: {:?} / GasUsed: {:?} / GasCost: {:?} / Bribe: {:?}", revenue, profit, gas_used, gas_cost, bribe_amount);

        // submit tx

        let victim_tx_hashes = one.get_victim_tx_hashes();
        let mut victim_txs = Vec::new();
        for tx_hash in victim_tx_hashes {
            if let Some(tx_info) = pending_txs.get(&tx_hash) {
                let tx = tx_info.pending_tx.tx.clone();
                victim_txs.push(tx);
            }
        }

        // send action to action_handler
        let action = BackrunAction {
            new_block: new_block.clone(),
            pending_txs: victim_txs,
            back_calldata: calldata,
            realistic_back_gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        };

        match action_sender.send(ActionEvent::Backrun(action)) {
            Ok(_) => {}
            Err(e) => error!("error sending action: {}", e),
        }
    }

    Ok(())
}

pub async fn main_dish_on_new_block(
    evm_factory: Arc<SimulatorFactory>,
    new_block: NewBlock,
    promising_pieces: &mut DashMap<TxHash, Vec<Piece>>,
    pending_txs: &DashMap<TxHash, PendingTxInfo>,
) -> Result<()> {
    info!("main_dish_on_new_block, promising_pieces: {:?}", promising_pieces.len());
    // - get tx less than new_block, and filter pieces with lower gas fee than new_block
    // - update revenue info for pending_txs base on new block
    // - submit all items as a bundle // seems not necessary because in the beginning of a new block, too early

    // TODO need to check pending_txs?

    if promising_pieces.len() == 0 {
        return Ok(());
    }

    for mut entry in promising_pieces.iter_mut() {
        let (promising_tx_hash, pieces) = entry.pair_mut();
        if pieces.len() == 0 {
            continue;
        }
        // get old pieces and gas price is higher than new_block
        for piece in pieces.iter_mut() {
            if piece.updated_at < new_block.block_number && piece.victim_tx.gas_price > new_block.next_base_fee {
                // check if still profitable
                // let mut piece = piece.clone(); // a new piece, it's ok to just for getting profit status
                piece.verify_arbitrage_profit(evm_factory.clone(), new_block.clone())?;
            }
        }
    }

    Ok(())
}
