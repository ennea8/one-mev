use anyhow::{anyhow, ensure, Result};
use std::{cell::RefCell, collections::HashMap, str::FromStr, sync::Arc};

// alloy
use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{utils::parse_ether, Address, BlockNumber, TxHash, B256, I256, U128, U256, U64},
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    pubsub::PubSubFrontend,
    rpc::types::eth::{Block, Log, Transaction},
    signers::local::PrivateKeySigner,
    sol_types::{sol, SolCall, SolValue},
    transports::ws::WsConnect,
};

//revm
use revm::{
    db::{CacheDB, EmptyDB},
    interpreter::Host,
    primitives::{
        address, keccak256, AccessList, AccessListItem, AccountInfo, Bytecode, Bytes, ExecutionResult, Output, SpecId, TransactTo,
    },
    Database, DatabaseRef, Evm, Inspector,
};

// use one_evm::types::{Tx, TxResult};
use one_evm::{abis, config::cache_dir, database_error::DatabaseError, fork_db::ForkDB, fork_factory::ForkFactory};

use lazy_static::lazy_static;

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::abi::IOne;
use crate::inspector::access_list::AccessListInspector;

use crate::simulation::simulator::{Simulator, SimulatorFactory, Tx, TxResult};
use one_common::create_default_wss_provider;

use crate::arbitrage::config::constants::{OWNER_ADDRESS, REVM_ONE_ADDRESS, REVM_ONE_SIMULATOR_ADDRESS};
use crate::common::bytecode::{ONE_BYTECODE, ONE_SIMULATOR_BYTECODE};
use crate::arbitrage::config::constants::ethereum::weth_addr;

use crate::arbitrage::types::Arbitrage;

impl<'a, EXT> Simulator<'a, EXT> {
    // for one swap and got the best path with optimized_in
    pub fn find_profitable_path_and_opt_amount(&mut self, swap_paths: Vec<Vec<IOne::SwapParams>>) -> Result<Option<Arbitrage>> {
        // test if profitable with a small amount in
        let mut sample_results = vec![];
        for (i, path) in swap_paths.iter().enumerate() {
            let mut path = path.clone();
            path[0].amount = U256::from(parse_ether("0.01").unwrap());

            match self.simulateSwapIn(path.clone()) {
                Ok((amountIn, profit)) => {
                    sample_results.push((i, amountIn, profit));
                }
                Err(err) => {
                    // Optionally log the error
                    debug!("Skipping path {} due to error: {:?}", i, err);
                    continue;
                }
            }
        }
        debug!("sample_results: {:?}", sample_results);

        // Filter profitable paths
        let profitable_items = sample_results.into_iter().filter(|(_, _, profit)| profit > &I256::ZERO).collect::<Vec<_>>();

        if profitable_items.is_empty() {
            return Ok(None);
        }

        // find the best optimized_in for each profitable path
        let mut results = vec![];
        for item in profitable_items.iter() {
            let the_path = swap_paths[item.0].clone();
            let (optimal_amount_in, max_revenue) = self.find_optimal_amount_by_linearly(the_path.clone())?;
            results.push(Arbitrage { path: the_path, optimized_in: optimal_amount_in, max_revenue });
        }

        // sort by max_revenue
        // results.sort_by_key(|p: &Arbitrage| p.max_revenue);
        let best_arbi = results.iter().max_by_key(|p| p.max_revenue).unwrap();

        Ok(Some(best_arbi.clone()))
    }

    // TODO base_gas
    // TODO move code with concurrency version together
    pub fn find_optimal_amount_by_linearly(&mut self, path: Vec<IOne::SwapParams>) -> Result<(U256, I256)> {
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
            // TODO check if can be parallel called
            let mut revenue = vec![];
            for (idx, &input) in inputs.iter().enumerate() {
                let mut _path = path.clone();
                _path[0].amount = input;

                let result = self.simulateSwapIn(_path)?;
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

    // TODO add front_tx logic

    pub fn simulateSwapIn(&mut self, paramsArray: Vec<IOne::SwapParams>) -> Result<(U256, I256)> {
        // TODO set input amount

        let calldata_swap_in = Bytes::from(IOne::simulateSwapInCall { paramsArray }.abi_encode());

        let mut tx = Tx {
            caller: *OWNER_ADDRESS,
            transact_to: *REVM_ONE_SIMULATOR_ADDRESS,
            data: calldata_swap_in,
            value: U256::ZERO,
            gas_price: U256::ZERO,
            gas_limit: U256::from(5000000),
            gas_priority_fee: None,
        };

        let result = match self.call_static(tx) {
            Ok(result) => result,
            Err(err) => {
                // TODO handler it better ?
                return Err(anyhow!("❌ [simulateSwapIn] error {:?}", err));
            }
        };

        let (amountIn, _, profit) = <(U256, U256, I256)>::abi_decode(&result.output, false)?;

        Ok((amountIn, profit))
    }

    // can be used to get uniswap v2/v3 swap in amountOut
    // query amountOut for v2/v3
    pub fn simulateSingleSwap(&mut self, params: IOne::SwapParams) -> Result<U256> {
        let mut tx = Tx {
            caller: *OWNER_ADDRESS,
            transact_to: *REVM_ONE_SIMULATOR_ADDRESS,
            data: Bytes::new(),
            value: U256::ZERO,
            gas_price: U256::ZERO,
            gas_limit: U256::from(5000000),
            gas_priority_fee: None,
        };

        if params.protocol == 2 {
            tx.data = Bytes::from(IOne::simulateUniswapV2SwapInCall { params }.abi_encode());
        } else if params.protocol == 3 {
            tx.data = Bytes::from(IOne::simulateUniswapV3SwapInCall { params }.abi_encode());
        } else {
            return Err(anyhow!("🟣 [simulate arbi] unsupported protocol {:?}", params.protocol));
        }

        let back_result = match self.call_static(tx) {
            Ok(result) => result,
            Err(err) => {
                return Err(anyhow!("🟣 [simulate arbi] backrun tx error {:?}", err));
                // TODO handler it better ?
            }
        };

        // let (amount_out) = <(U256)>::abi_decode(&back_result.output, false)?;
        let amount_out = match <(U256)>::abi_decode(&back_result.output, false) {
            Ok(amount) => amount,
            Err(err) => return Err(anyhow!("Failed to decode output: {:?}", err)),
        };

        Ok(amount_out)
    }

    method_alias!(getAmountOut, simulateSingleSwap);

    // Base on contract One not OneSimulator
    // params.amount is defferent if v2 is the first swap
    // return (profit, gas_used, calldata_arbitrage)

    pub fn simulateArbitrage(&mut self, mut paramsArray: Vec<IOne::SwapParams>, gas_price: U256) -> Result<(I256, u64, u64, Bytes)> {
        debug!("[in simulateArbitrage] paramsArray: {:?}", paramsArray);
        let first_swap = paramsArray[0].clone();
        if first_swap.protocol == 2 {
            // need to get amountOut for v2
            let amount_out = self.simulateSingleSwap(first_swap.clone())?;
            paramsArray[0].amount = amount_out;
        }

        debug!("[in simulateArbitrage] paramsArray after: {:?}", paramsArray);

        let pathArrayData = Bytes::from(<Vec<IOne::SwapParams>>::abi_encode(&paramsArray));

        // suppose first_swap.tokenIn is always baseToken
        let calldata_arbitrage = Bytes::from(
            IOne::arbitrageCall { pathArrayData: pathArrayData.clone(), baseToken: first_swap.tokenIn, requireProfit: false }.abi_encode(),
        );

        let mut tx = Tx {
            caller: *OWNER_ADDRESS,
            transact_to: *REVM_ONE_ADDRESS,
            data: calldata_arbitrage.clone(),
            value: U256::ZERO,
            gas_price,
            gas_limit: U256::from(5000000),
            gas_priority_fee: None,
        };

        let eth_balance_before = self.get_eth_balance(*OWNER_ADDRESS);
        debug!("[in simulateArbitrage] tx: {:?}", tx);

        // commit to get gas_cost
        let result = match self.call(tx) {
            Ok(result) => result,
            Err(err) => {
                return Err(anyhow!("❌ [in simulateArbitrage] error {:?}", err));
            }
        };

        let eth_balance_after = self.get_eth_balance(*OWNER_ADDRESS);

        let gas_cost: u64 = eth_balance_before.checked_sub(eth_balance_after).unwrap_or_default().try_into().unwrap_or_default();

        let gas_used = result.gas_used;
        let profit: I256 = I256::abi_decode(&result.output, false)?;

        // update calldata for submitting version
        let calldata_arbitrage =
            Bytes::from(IOne::arbitrageCall { pathArrayData, baseToken: first_swap.tokenIn, requireProfit: true }.abi_encode());

        Ok((profit, gas_used, gas_cost, calldata_arbitrage))
    }

    pub fn simulateArbitrageMulti(&mut self, mut arbitrages: Vec<Arbitrage>, gas_price: U256) -> Result<(I256, I256, u64, u64, Bytes)> {
        let mut groupSizeArr = vec![U256::ZERO; arbitrages.len()];
        let mut uni_paths = vec![];

        // set amountOut for v2
        for arbi in arbitrages.iter_mut() {
            arbi.path[0].amount = arbi.optimized_in; // set optimized_in
            if arbi.path[0].protocol == 2 {
                let amount_out = self.simulateSingleSwap(arbi.path[0].clone())?;
                arbi.path[0].amount = amount_out;
            }
        }

        info!("[simulateArbitrageMulti] arbitrages: {:?}", arbitrages);

        // flatten paths
        for (idx, arbi) in arbitrages.iter().enumerate() {
            groupSizeArr[idx] = U256::from(arbi.path.len());
            uni_paths.extend(arbi.path.clone());
        }

        let pathArrayData = Bytes::from(<Vec<IOne::SwapParams>>::abi_encode(&uni_paths));

        let calldata_arbitrage = Bytes::from(
            IOne::arbitrageMultiCall { pathArrayData: pathArrayData.clone(), groupSizeArr: groupSizeArr.clone(), requireProfit: false }
                .abi_encode(),
        );

        let mut tx = Tx {
            caller: *OWNER_ADDRESS,
            transact_to: *REVM_ONE_ADDRESS,
            data: calldata_arbitrage.clone(),
            value: U256::ZERO,
            gas_price,
            gas_limit: U256::from(5000000), // TODO check
            gas_priority_fee: None,
        };
        let eth_balance_before = self.get_eth_balance(*OWNER_ADDRESS);

        // commit to get gas_cost
        let result = match self.call(tx) {
            Ok(result) => result,
            Err(err) => {
                return Err(anyhow!("❌ [simulateArbitrageMulti] error {:?}", err));
            }
        };

        let eth_balance_after = self.get_eth_balance(*OWNER_ADDRESS);

        let gas_cost: u64 = eth_balance_before.checked_sub(eth_balance_after).unwrap_or_default().try_into().unwrap_or_default();

        let gas_used = result.gas_used;
        let profit: I256 = I256::abi_decode(&result.output, false)?;

        let gas_cost_i256 = I256::from_dec_str(&gas_cost.to_string())?;
        let revenue = profit - gas_cost_i256;

        let calldata_arbitrage = Bytes::from(IOne::arbitrageMultiCall { pathArrayData, groupSizeArr, requireProfit: true }.abi_encode());

        Ok((revenue, profit, gas_used, gas_cost, calldata_arbitrage))
    }
}

pub async fn create_simulator_factory(block_number: u64) -> Result<Arc<SimulatorFactory>> {
    let provider = create_default_wss_provider().await.unwrap();
    let block_number = if block_number == 0 { provider.get_block_number().await.unwrap() } else { block_number };

    let cache_db = CacheDB::new(EmptyDB::default());

    let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));
    // let evm_factory = SimulatorFactory::new(provider.clone(), cache_db, block_number);

    Ok(evm_factory)
}

pub fn create_evm_factory(provider: Arc<RootProvider<PubSubFrontend>>, block_number: u64) -> Result<Arc<SimulatorFactory>> {
    let cache_db = CacheDB::new(EmptyDB::default());
    let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));

    // add gas for owner
    evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("1").unwrap()));

    // init simulator contract
    evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
    let _ = evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

    // init one bot contract locally
    let _ = evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone()));
    let _ = evm_factory.set_token_balance(weth_addr(), *REVM_ONE_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

    Ok(evm_factory)
}


mod tests {
    use serde_json;
    use std::ops::Add;

    use super::*;
    use crate::arbitrage::config::constants::ethereum::weth_addr;
    use crate::common::bytecode::ONE_BYTECODE;
    use crate::common::bytecode::ONE_SIMULATOR_BYTECODE; // 模拟版本 // 线上版本

    use one_common::{init_logs, measure_end, measure_start};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_onebot_simulateArbitrageMulti() -> Result<()> {
        init_logs();

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();
        let EBULL_V3 = Address::from_str("0xa9405016F8158d87f5659b63df170c03B8396450").unwrap();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?; // before onebot contract deploy

        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));

        // one contract
        evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone())); // use online code
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        // one_simulator contract
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        // ---------------------------- front tx ----------------------------
        info!("----------------------------begin_front_run----------------------------");

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();

        let tx_req = Tx::from_transaction(pending_tx);

        info!("tx_req: {:?}", tx_req);

        sim.set_base_fee(U256::from(1000000000));

        match sim.call(tx_req.clone()) {
            Ok(result) => {
                info!("pending_tx result {:?}", result);
            }
            Err(err) => {
                info!("❌ [simulateArbitrage]run pending_tx error {:?}", err);
            }
        }

        info!("----------------------------begin_back_run----------------------------end");

        sim.set_base_fee(U256::ZERO);

        // TODO fix "buffer overrun while deserializing"
        let mut pathArray = vec![];
        pathArray.push(IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH,
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(),
        });
        pathArray.push(IOne::SwapParams {
            protocol: 3,
            handler: EBULL_V3,
            tokenIn: EBULL,
            tokenOut: WETH,
            fee: 10_000,
            amount: U256::ZERO,
        });

        // ---------------------------- find profitable path ----------------------------

        info!("start find_profitable_path_and_opt_amount");

        let paths = vec![pathArray.clone()];
        // find best optimized_in
        let arbi = match sim.find_profitable_path_and_opt_amount(paths) {
            Ok(Some(result)) => result,
            Ok(None) => {
                debug!("No profitable path found");
                return Err(anyhow!("No profitable path found"));
            }
            Err(e) => {
                warn!("Error finding profitable path: {:?}", e);
                return Err(anyhow!("Error finding profitable path: {:?}", e));
            }
        };

        // ---------------------------- call onebot.arbitrageMulti ----------------------------
        // base on a new simulator
        let mut sim = evm_factory.new_fork_simulator(false);

        // front-run
        match sim.call(tx_req.clone()) {
            Ok(result) => {
                info!("pending_tx result {:?}", result);
            }
            Err(err) => {
                info!("❌ [simulateArbitrage]run pending_tx error {:?}", err);
            }
        }

        //  simulateArbitrageMulti
        let gas_price = U256::from_str("1000000000").unwrap();
        let (revenue, profit, gas_used, gas_cost, calldata) = sim.simulateArbitrageMulti(vec![arbi], gas_price)?;

        info!("revenue: {:?}, profit: {:?}, gas_used: {:?}, gas_cost: {:?}, calldata: {:?}", revenue, profit, gas_used, gas_cost, calldata);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_onebot_profit_simulateArbitrage_001() -> Result<()> {
        init_logs();

        // TODO debug 应该和 case的profit一样

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();
        let EBULL_V3 = Address::from_str("0xa9405016F8158d87f5659b63df170c03B8396450").unwrap();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?; // before onebot contract deploy

        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));

        // one contract
        evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone())); // use online code
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        // one_simulator contract
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        // owner for bot_one
        // let owner_u256 = U256::from_str(&format!("{:?}", *OWNER_ADDRESS)).unwrap();
        // sim.insert_account_storage(*REVM_ONE_ADDRESS, U256::from(0), owner_u256)?;

        //TODO 解决 100 error问题

        // ---------------------------- front tx ----------------------------
        info!("----------------------------begin_front_run----------------------------");

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();

        let tx_req = Tx::from_transaction(pending_tx);

        info!("tx_req: {:?}", tx_req);

        sim.set_base_fee(U256::from(1000000000));

        match sim.call(tx_req.clone()) {
            Ok(result) => {
                info!("pending_tx result {:?}", result);
            }
            Err(err) => {
                info!("❌ [simulateArbitrage]run pending_tx error {:?}", err);
            }
        }

        info!("----------------------------begin_back_run----------------------------end");

        sim.set_base_fee(U256::ZERO);

        // TODO fix "buffer overrun while deserializing"
        let mut pathArray = vec![];
        pathArray.push(IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH,
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(),
        });
        pathArray.push(IOne::SwapParams {
            protocol: 3,
            handler: EBULL_V3,
            tokenIn: EBULL,
            tokenOut: WETH,
            fee: 10_000,
            amount: U256::ZERO,
        });

        // ---------------------------- find profitable path ----------------------------

        info!("start find_profitable_path_and_opt_amount");

        let paths = vec![pathArray.clone()];
        // find best optimized_in
        let arbitrage = match sim.find_profitable_path_and_opt_amount(paths) {
            Ok(Some(result)) => result,
            Ok(None) => {
                debug!("No profitable path found");
                return Err(anyhow!("No profitable path found"));
            }
            Err(e) => {
                warn!("Error finding profitable path: {:?}", e);
                return Err(anyhow!("Error finding profitable path: {:?}", e));
            }
        };

        let the_path = arbitrage.path.clone();
        let optimized_in = arbitrage.optimized_in;
        let max_revenue = arbitrage.max_revenue;

        info!("found profitable path: {:?}, optimized_in: {:?}, max_revenue: {:?}", the_path, optimized_in, max_revenue);

        // ---------------------------- call onebot ----------------------------
        let mut sim = evm_factory.new_fork_simulator(false);

        let mut the_path: Vec<IOne::SwapParams> = the_path.clone();
        the_path[0].amount = optimized_in;

        // TODO

        // run pending tx
        match sim.call(tx_req.clone()) {
            Ok(result) => {
                info!("pending_tx result {:?}", result);
            }
            Err(err) => {
                info!("❌ [simulateArbitrage]run pending_tx error {:?}", err);
            }
        }

        let gas_price = U256::from_str("1000000000").unwrap();
        let (profit, gas_used, gas_cost, calldata) = match sim.simulateArbitrage(the_path, gas_price) {
            Ok(result) => result,
            Err(err) => {
                info!("❌ [simulateArbitrage]  error {:?}", err);
                return Err(anyhow!("❌ [simulateArbitrage]  error {:?}", err));
            }
        };

        info!("⭕⭕⭕ profit: {:?}, gas_used: {:?}, gas_cost: {:?}, calldata: {:?}", profit, gas_used, gas_cost, calldata);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_onebot_profit_simulateArbitrage_002() -> Result<()> {
        init_logs();

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();
        let EBULL_V3 = Address::from_str("0xa9405016F8158d87f5659b63df170c03B8396450").unwrap();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;

        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));

        evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone())); // use online code
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        // prepare bot
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        // owner for bot_one
        // let owner_u256 = U256::from_str(&format!("{:?}", *OWNER_ADDRESS)).unwrap();
        // sim.insert_account_storage(*REVM_ONE_ADDRESS, U256::from(0), owner_u256)?;

        // TODO fix "buffer overrun while deserializing"
        let mut pathArray = vec![];
        pathArray.push(IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH,
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(),
        });
        pathArray.push(IOne::SwapParams {
            protocol: 3,
            handler: EBULL_V3,
            tokenIn: EBULL,
            tokenOut: WETH,
            fee: 10_000,
            amount: U256::ZERO,
        });

        let gas_price = U256::from_str("1000000000").unwrap();
        let (profit, gas_used, gas_cost, calldata) = match sim.simulateArbitrage(pathArray, gas_price) {
            Ok(result) => result,
            Err(err) => {
                info!("❌ [simulateArbitrage]  error {:?}", err);
                return Err(anyhow!("❌ [simulateArbitrage]  error {:?}", err));
            }
        };

        info!("profit: {:?}, gas_used: {:?}, calldata: {:?}", profit, gas_used, calldata);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_find_profitable_path_and_opt_amount_001() -> Result<()> {
        init_logs();
        let sample_results = vec![
            (U256::from(100), U256::from(200), I256::try_from(-110i32).unwrap()),
            (U256::from(100), U256::from(200), I256::try_from(110i32).unwrap()),
            (U256::from(100), U256::from(200), I256::try_from(-1100i32).unwrap()),
            (U256::from(100), U256::from(200), I256::try_from(910i32).unwrap()),
        ];

        let most_profitable = sample_results.into_iter().filter(|(_, _, profit)| profit > &I256::ZERO).max_by_key(|(_, _, profit)| *profit);

        info!("most_profitable: {:?}", most_profitable);

        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_get_block_number() -> Result<()> {
        init_logs();

        let provider = create_default_wss_provider().await.unwrap();

        let block_number = provider.get_block_number().await?;

        info!("block_number: {:?}", block_number);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_get_tx_data() -> Result<()> {
        init_logs();

        let tx_hash = TxHash::from_str("0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8").unwrap();
        let provider = create_default_wss_provider().await.unwrap();
        let transaction = provider.get_transaction_by_hash(tx_hash).await?;

        let tx_request = transaction.unwrap().into_request();

        let tx_json = serde_json::to_string(&tx_request)?;
        // std::fs::write("tx_0xbd75.json", tx_json.as_bytes())?;

        info!("pending_transaction: {:?}", tx_json);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_run_front_tx() -> Result<()> {
        init_logs();
        // TODO debug 当cacache开启情况第二次运行，会失败的问题
        /*
          from: 0xEFA9268490BB76D6b17793905473feFc03b5C824
          to: 0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49
        */

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;

        let mut sim = evm_factory.new_fork_simulator(false);

        // TODO get approve amount 信息？

        let block_number = sim.get_block_number();

        info!("⭕==block_number: {:?}", block_number);
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();

        let token_balance = sim.get_token_balance(EBULL, pending_tx.from)?;

        info!("⭕==token_balance: {:?}", token_balance);

        let tx_req = Tx::from_transaction(pending_tx);

        info!("tx_req: {:?}", tx_req);

        info!("sim.get_base_fee(): {:?}", sim.get_base_fee());

        sim.set_base_fee(U256::from(1000000000));

        info!("sim.get_base_fee() afters set: {:?}", sim.get_base_fee());

        let result = sim.call(tx_req)?; // TODO fix will trigger error

        info!("result: {:?}", result);

        // TODO check why gas_refunded
        // { output: 0x, logs: None, gas_used: 159584, gas_refunded: 27500 }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_profit_simulateSwapIn() -> Result<()> {
        init_logs();

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();
        let EBULL_V3 = Address::from_str("0xa9405016F8158d87f5659b63df170c03B8396450").unwrap();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;

        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));

        // prepare bot
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        // ---------------------------- front tx ----------------------------
        info!("----------------------------begin_front_run----------------------------");

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();

        let tx_req = Tx::from_transaction(pending_tx);

        info!("tx_req: {:?}", tx_req);

        sim.set_base_fee(U256::from(1000000000));

        match sim.call(tx_req) {
            Ok(result) => {
                info!("pending_tx result {:?}", result);
            }
            Err(err) => {
                info!("❌ [test_sim_arbi_profit_simulateSwapIn] run pending_tx error {:?}", err);
            }
        }

        info!("----------------------------begin_back_run----------------------------end");

        // TODO fix "buffer overrun while deserializing"
        let mut pathArray = vec![];
        pathArray.push(IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH,
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(),
        });
        pathArray.push(IOne::SwapParams {
            protocol: 3,
            handler: EBULL_V3,
            tokenIn: EBULL,
            tokenOut: WETH,
            fee: 10_000,
            amount: U256::ZERO,
        });

        let (amountIn, profit) = match sim.simulateSwapIn(pathArray) {
            Ok(result) => result,
            Err(err) => {
                info!("❌ [test_sim_arbi_profit_simulateSwapIn] simulateSwapIn error {:?}", err);
                return Err(anyhow!("❌ [test_sim_arbi_profit_simulateSwapIn] simulateSwapIn error {:?}", err));
            }
        };

        info!("amountIn: {:?} profit: {:?}", amountIn, profit);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim_arbi_skip_v3_with_empty_liquidity() -> Result<()> {
        init_logs();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;
        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        // token pool
        let pool_v2 = address!("Efdf4DfC4e817197851266Acf0082A80DaB18b24");
        let pool_v3 = address!("f422803c1cDC6FD07c585132ff0526A38d2A239B"); // v3
        let token0 = address!("68a47fe1cf42eba4a030a10cd4d6a1031ca3ca0a"); // TEA
        let token1 = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");

        // ---------------------------- ----------------------------

        let params = IOne::SwapParams {
            protocol: 3,
            handler: pool_v3,
            tokenIn: token1, // 应为WETH，计算出out，在传给swap？
            tokenOut: token0,
            fee: 0,
            amount: parse_ether("0.5").unwrap(), // 根据eth的input提前计算
        };

        let amount_out = sim.simulateSingleSwap(params)?;

        info!("amount_out: {:?}", amount_out);

        // TODO 合约需加逻辑

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn test_sim_simulateUniswapV2SwapIn_linear() -> Result<()> {
        init_logs();

        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;
        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();

        let params = IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH, // 应为WETH，计算出out，在传给swap？
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(), // 根据eth的input提前计算
        };

        let tx = Tx {
            caller: *OWNER_ADDRESS,
            transact_to: *REVM_ONE_SIMULATOR_ADDRESS,
            data: Bytes::new(),
            value: U256::ZERO,
            gas_price: U256::ZERO, // TODO check
            gas_limit: U256::from(5000000),
            gas_priority_fee: None,
        };

        // check why 4 requests?
        // let amount_out = sim.simulateSingleSwap(params.clone())?;
        // info!("amount_out: {:?}", amount_out);

        let start = measure_start("test");

        let mut amounts = Vec::new();
        for i in 0..100 {
            // let mut sim = evm_factory.new_fork_simulator(false); // not necessary
            // parse_ether((i + 1).to_string().as_str()) //i as f64 * 0.1 + 0.1
            let amount = parse_ether((i as f64 * 0.1 + 0.1).to_string().as_str())?;
            let mut params = params.clone();
            params.amount = amount;
            let amount_out = sim.simulateSingleSwap(params)?;
            amounts.push(amount_out);
        }

        measure_end(start);

        info!("amounts: {:?}", amounts);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 5)]
    async fn test_sim_simulateUniswapV2SwapIn_parallel() -> Result<()> {
        init_logs();

        println!("RUST_LOG: {}", std::env::var("RUST_LOG").unwrap());

        // prepare env: gas, code, weth balance
        let evm_factory = create_simulator_factory(20_975_913 - 1).await?;
        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));
        evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
        evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

        let mut sim = evm_factory.new_fork_simulator(false);

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();

        let params = IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH, // 应为WETH，计算出out，在传给swap？
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(), // 根据eth的input提前计算
        };

        // let amount_out = sim.simulateSingleSwap(params.clone())?;
        // info!("amount_out: {:?}", amount_out);

        // // 并发测试
        let mut amounts = Vec::new();
        for i in 0..100 {
            // parse_ether((i + 1).to_string().as_str())
            amounts.push(parse_ether((i as f64 * 0.1 + 0.1).to_string().as_str())?);
        }

        let handles: Vec<_> = amounts
            .iter()
            .enumerate()
            .map(|(index, &amount)| {
                let evm_factory = evm_factory.clone();
                let mut params = params.clone();
                params.amount = amount;
                tokio::spawn(async move {
                    let mut sim = evm_factory.new_fork_simulator(false);
                    let amount_out = sim.simulateSingleSwap(params);
                    // let amount_out = sim.get_block_number();
                    let amount_out = amount_out.unwrap();
                    //info!("amount_out: {:?}", amount_out);
                    (index, amount_out)
                })
            })
            .collect();

        let start = measure_start("test");
        // Wait for all tasks to complete and collect results
        let results = futures::future::join_all(handles).await;
        measure_end(start);

        let mut amounts = Vec::new();
        for result in results {
            let (index, amount_out) = result.unwrap();
            // info!("index {}: amount_out {:?}", index + 1, amount_out);
            amounts.push(amount_out);
        }

        info!("amounts: {:?}", amounts);

        Ok(())
    }
}
