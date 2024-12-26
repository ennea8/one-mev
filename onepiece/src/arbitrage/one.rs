use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::sync::Arc;

use alloy::primitives::{Bytes, TxHash, I256, U256};
use alloy::rpc::types::eth::Transaction;

use crate::abi::IOne;
use crate::arbitrage::types::{Arbitrage, NewBlock, One, Piece};
use crate::simulation::simulator::{SimulatorFactory, Tx, VictimTx};

#[derive(Debug)]
pub struct OneSimulateResult {
    pub revenue: I256,
    pub profit: I256,
    pub gas_used: u64,
    pub gas_cost: u64,
    pub calldata: Bytes,
}

impl One {
    pub fn one_id(&self) -> String {
        let mut tx_hashes = Vec::new();
        for piece in &self.pieces {
            let tx_hash = piece.victim_tx.tx_hash;
            let tx_hash_4_bytes = &format!("{:?}", tx_hash)[0..10];
            tx_hashes.push(String::from(tx_hash_4_bytes));
        }
        tx_hashes.sort();
        tx_hashes.dedup();
        tx_hashes.join("-")
    }

    pub fn get_victim_txs(&self) -> Vec<VictimTx> {
        let mut unique_txs = HashSet::new();
        self.pieces.iter().filter_map(|p| if unique_txs.insert(p.victim_tx.tx_hash) { Some(p.victim_tx.clone()) } else { None }).collect()
    }

    pub fn get_victim_tx_hashes(&self) -> Vec<TxHash> {
        self.get_victim_txs().iter().map(|tx| tx.tx_hash).collect()
    }

    // pub fn encode_frontrun_tx(&self) -> Result<()> {
    //     Ok(())
    // }

    pub fn encode_backrun_tx(&self) -> Result<()> {
        Ok(())
    }

    // simulation on chain version
    // support one or multi
    // should can be concurrency called from outside
    pub async fn simulate(&self, evm_factory: Arc<SimulatorFactory>, new_block: NewBlock) -> Result<OneSimulateResult> {
        let mut sim = evm_factory.new_fork_simulator(false);
        sim.set_base_fee(new_block.next_base_fee);

        // frontrun [empty]

        // Victim Txs
        let victim_txs = self.get_victim_txs();

        for victim_tx in victim_txs {
            // simulate victim_tx
            match sim.call(Tx::from(victim_tx.clone())) {
                Ok(result) => {
                    info!("🟢📌 victim_tx success : {:?}", victim_tx.tx_hash);
                }
                Err(e) => {
                    warn!("❗❌ one.simulate victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e);
                    return Err(anyhow!("❗❌ one.simulate victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e));
                }
            }
        }

        // sim.set_base_fee(U256::ZERO);
        // [placeholder] some contract call but no need to take gas into account
        // sim.set_base_fee(new_block.next_base_fee);

        let arbitrages: Vec<Arbitrage> = self.pieces.iter().filter_map(|p| p.arbitrage.clone()).collect();

        let (revenue, profit, gas_used, gas_cost, calldata) = match sim.simulateArbitrageMulti(arbitrages, new_block.next_base_fee) {
            Ok(result) => result,
            Err(err) => {
                info!("❌ [simulateArbitrageMulti]  error {:?}", err);
                return Err(anyhow!("❌ [simulateArbitrageMulti]  error {:?}", err));
            }
        };
        // TODO get access_list?

        let simulated_arbitrage_result = OneSimulateResult { revenue, profit, gas_used, gas_cost, calldata };
        Ok(simulated_arbitrage_result)
    }

    // Base on OneSimulator contract
    // Base on call_static, gas code is not taken into account
    // Used to find optimal amount_in/amount_out
    // Result is separate for each arbitrage, not merged
    pub async fn simulate_a_piece(&self, evm_factory: Arc<SimulatorFactory>, new_block: NewBlock) -> Result<(U256, I256)> {
        let mut sim = evm_factory.new_fork_simulator(false);
        sim.set_base_fee(new_block.next_base_fee);

        // require one piece
        if self.pieces.len() != 1 {
            return Err(anyhow!("❌ [simulate_localy_version]  require one piece"));
        }
        let victim_tx = self.pieces[0].victim_tx.clone();

        // Victim Txs
        match sim.call(Tx::from(victim_tx.clone())) {
            Ok(result) => {
                info!("🟢📌 victim_tx success : {:?}", victim_tx.tx_hash);
            }
            Err(e) => {
                warn!("❗❌ one.simulate victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e);
                return Err(anyhow!("❗❌ one.simulate victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e));
            }
        }

        let arbitrage = self.pieces[0].arbitrage.as_ref().unwrap();

        let (amount_in, profit) = match sim.simulateSwapIn(arbitrage.path.clone()) {
            Ok(result) => result,
            Err(err) => {
                info!("❌ [simulateArbitrage]  error {:?}", err);
                return Err(anyhow!("❌ [simulateArbitrage]  error {:?}", err));
            }
        };

        Ok((amount_in, profit))
    }
}
