use anyhow::{anyhow, Result};
use std::sync::Arc;

use crate::arbitrage::types::NewBlock;
use crate::arbitrage::types::Piece;
use crate::simulation::simulator::{SimulatorFactory, Tx};
use alloy::primitives::{utils::parse_ether, I256, U256};

impl Piece {
    pub fn is_sandwich(&self) -> bool {
        self.sandwich.is_some()
    }

    pub fn is_arbitrage(&self) -> bool {
        self.arbitrage.is_some()
    }

    // vefify if still profitable especailly when new block arrives
    // update updated_at for new block
    // evm_factory already bind with block number
    // case: profit can be more or less
    pub fn verify_arbitrage_profit(&mut self, evm_factory: Arc<SimulatorFactory>, new_block: NewBlock) -> Result<bool> {
        let mut evm = evm_factory.new_fork_simulator(false);
        evm.set_base_fee(new_block.next_base_fee);

        let mut path = self.arbitrage.as_ref().unwrap().path.clone();
        path[0].amount = self.arbitrage.as_ref().unwrap().optimized_in;

        let result = evm.simulateSwapIn(path).unwrap();

        let profit = result.1;

        let is_profitable = profit >= self.arbitrage.as_mut().unwrap().max_revenue;

        if !is_profitable {
            // just reset max_revenue to 0 to make it invalid. it's `weight` should be 0
            debug!("[verify_arbitrage_profit] arbitrage is not profitable, reset max_revenue to 0");
            self.arbitrage.as_mut().unwrap().max_revenue = I256::ZERO;
        }

        // if profit is less, there should be new swap happened related pool, so will remove this piece
        Ok(is_profitable)
    }

    // TODO test
    pub async fn find_optimal_amount_in_concurrently(
        &mut self,
        evm_factory: Arc<SimulatorFactory>,
        new_block: NewBlock,
    ) -> Result<(U256, I256)> {
        let path = self.arbitrage.as_ref().unwrap().path.clone();

        let mut min_amount_in = U256::ZERO;
        let mut max_amount_in = parse_ether("20").unwrap();
        let tolerance = parse_ether("0.001").unwrap();
        let intervals = U256::from(10);

        let mut optimized_in = U256::ZERO;
        let mut max_revenue = I256::ZERO;
        let mut counter = 0;

        loop {
            counter += 1;
            trace!("----------loop begin----min/max: {:?}/{:?}----------", min_amount_in, max_amount_in);

            let diff = max_amount_in - min_amount_in;
            let step = diff.checked_div(intervals).unwrap();

            if step <= tolerance {
                break;
            }
            if max_amount_in < min_amount_in {
                break;
            }

            let mut inputs = Vec::new();
            for i in 1..u64::try_from(intervals).unwrap() + 1 {
                //TODO check
                let _i = U256::from(i);
                let input = min_amount_in + (_i * step);
                inputs.push(input);
            }

            // ============================================
            // Do concurrency

            let mut simulations = Vec::new();

            for (idx, &input) in inputs.iter().enumerate() {
                let mut path = path.clone();
                let evm_factory = evm_factory.clone();
                let new_block = new_block.clone();
                let victim_tx = self.victim_tx.clone();

                let handle = tokio::spawn(async move {
                    // let mut _path = path.clone();
                    path[0].amount = input;

                    let mut evm = evm_factory.new_fork_simulator(false);
                    evm.set_base_fee(new_block.next_base_fee);

                    // call victim_tx
                    match evm.call(Tx::from(victim_tx.clone())) {
                        Ok(result) => {
                            info!("🟢📌 victim_tx success : {:?}", victim_tx.tx_hash);
                        }
                        Err(e) => {
                            error!("❗❌ victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e);
                            return (idx, U256::ZERO, I256::ZERO);
                        }
                    }

                    let result = evm.simulateSwapIn(path).unwrap();
                    (idx, result.0, result.1)
                });

                simulations.push(handle);
            }
            let results = futures::future::join_all(simulations).await;

            let revenue: Vec<(usize, U256, I256)> = results.into_iter().map(|res| res.unwrap()).collect();

            // ============================================
            let mut max_idx = 0;

            // get the best
            let max_revenue_info = revenue.iter().max_by_key(|(_, _, profit)| *profit).unwrap();
            if max_revenue_info.2 > max_revenue {
                optimized_in = max_revenue_info.1;
                max_revenue = max_revenue_info.2;
                max_idx = max_revenue_info.0;
            }

            if max_revenue <= I256::ZERO {
                if counter > 10 {
                    break;
                }
            }

            trace!("[best in this loop]optimized_in/ max_revenue:{:?},/ {:?}", optimized_in, max_revenue);

            // prepare for next loop
            min_amount_in = if max_idx == 0 { U256::ZERO } else { revenue[max_idx - 1].1 };
            max_amount_in = if max_idx == revenue.len() - 1 { revenue[max_idx].1 } else { revenue[max_idx + 1].1 };
        }
        trace!("----------loop end counter: {:?}----------", counter);
        Ok((optimized_in, max_revenue))
    }

    // TODO test
    pub fn find_optimal_amount_by_linearly(&mut self, evm_factory: Arc<SimulatorFactory>, new_block: NewBlock) -> Result<(U256, I256)> {
        let mut evm = evm_factory.new_fork_simulator(false);
        evm.set_base_fee(new_block.next_base_fee);

        // call victim_tx once
        let victim_tx = self.victim_tx.clone();
        match evm.call(Tx::from(victim_tx.clone())) {
            Ok(result) => {
                info!("🟢📌 victim_tx success : {:?}", victim_tx.tx_hash);
            }
            Err(e) => {
                warn!("❗❌ victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e);
                return Err(anyhow!("❗❌ victim_tx error: {:?}, {:?}", victim_tx.tx_hash, e));
            }
        }

        let path = self.arbitrage.as_ref().unwrap().path.clone();

        let mut min_amount_in = U256::ZERO;
        let mut max_amount_in = parse_ether("20").unwrap();
        let tolerance = parse_ether("0.001").unwrap();
        let intervals = U256::from(10);

        let mut optimized_in = U256::ZERO;
        let mut max_revenue = I256::ZERO;
        let mut counter = 0;

        loop {
            counter += 1;
            trace!("----------loop begin----min/max: {:?}/{:?}----------", min_amount_in, max_amount_in);

            let diff = max_amount_in - min_amount_in;
            let step = diff.checked_div(intervals).unwrap();

            if step <= tolerance {
                break;
            }
            if max_amount_in < min_amount_in {
                break;
            }

            let mut inputs = Vec::new();
            for i in 1..u64::try_from(intervals).unwrap() + 1 {
                //TODO check
                let _i = U256::from(i);
                let input = min_amount_in + (_i * step);
                inputs.push(input);
            }

            // let mut simulations = Vec::new();
            // TODO check if can be parallel called, if can, no need another concurrent version
            let mut revenue = vec![];
            for (idx, &input) in inputs.iter().enumerate() {
                let mut _path = path.clone();
                _path[0].amount = input;

                let result = evm.simulateSwapIn(_path)?;
                revenue.push((idx, result.0, result.1));
            }

            let mut max_idx = 0;

            // get the best
            let max_revenue_info = revenue.iter().max_by_key(|(_, _, profit)| *profit).unwrap();
            if max_revenue_info.2 > max_revenue {
                optimized_in = max_revenue_info.1;
                max_revenue = max_revenue_info.2;
                max_idx = max_revenue_info.0;
            }

            if max_revenue <= I256::ZERO {
                if counter > 10 {
                    break;
                }
            }

            trace!("[best in this loop]optimized_in/ max_revenue:{:?},/ {:?}", optimized_in, max_revenue);

            // prepare for next loop
            min_amount_in = if max_idx == 0 { U256::ZERO } else { revenue[max_idx - 1].1 };
            max_amount_in = if max_idx == revenue.len() - 1 { revenue[max_idx].1 } else { revenue[max_idx + 1].1 };
        }

        trace!("----------loop end counter: {:?}----------", counter);

        Ok((optimized_in, max_revenue))
    }
}
