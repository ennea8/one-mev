use alloy_sol_types::SolEvent;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::{cell::RefCell, collections::HashSet, str::FromStr, sync::Arc};

use alloy::pubsub::PubSubFrontend;
use alloy::rpc::types::trace::geth::{
    BlockTraceResult, CallConfig, CallFrame, CallLogFrame, GethDebugBuiltInTracerType, GethDebugTracerConfig, GethDebugTracerType,
    GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace, TraceResult,
};
use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{Address, I256, U128, U256, U64},
    providers::{Provider, RootProvider},
    rpc::types::eth::{Block, Transaction},
    rpc::types::trace::parity::TraceType,
    sol_types::{sol, SolCall, SolValue},
};
use alloy_provider::ext::DebugApi;
use alloy_provider::ext::TraceApi;

use crate::abi::IOne;
use crate::arbitrage::config::constants::ethereum::weth_addr;
use crate::arbitrage::types::{NewBlock, NewPendingTx};

use crate::arbitrage::types::{DexVariant, Pool, SwapDirection, SwapInfo};

pub static V2_SWAP_EVENT_ID: &str = "0xd78ad95f";

// "Swap(address,address,int256,int256,uint160,uint128,int24)"
pub static V3_SWAP_EVENT_ID: &str = "0xc42079f9";

impl Pool {
    pub fn get_main_token(&self) -> Address {
        weth_addr()
    }
    pub fn get_other_token(&self) -> Address {
        if self.token0 == weth_addr() {
            self.token1
        } else {
            self.token0
        }
    }
    pub fn is_token0_main(&self) -> bool {
        self.token0 == weth_addr()
    }
    pub fn is_uniswap_v2(&self) -> bool {
        self.version == DexVariant::UniswapV2
    }
    pub fn is_uniswap_v3(&self) -> bool {
        self.version == DexVariant::UniswapV3
    }

    pub fn get_other_pools(&self) -> Vec<Address> {
        self.other_pools.clone()
    }
}

pub struct PoolManager {
    pub pools: DashMap<Address, Pool>,
}

impl PoolManager {
    pub fn get_pool(&self, address: &Address) -> Option<Pool> {
        self.pools.get(address).map(|pool| pool.value().clone())
    }

    pub async fn get_touched_pools_by_trace_call(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        victim_tx: &Transaction,
        block_number: u64,
    ) -> Result<()> {
        // get victim tx state diffs
        // victim_tx, vec![TraceType::StateDiff], Some(block_number)

        let req = victim_tx.clone().into_request();
        let trace_result = provider.trace_call(&req, &vec![TraceType::StateDiff]).await;

        let state_diffs = match trace_result {
            Ok(trace_result) => {
                //info!("trace_result: {:?}", trace_result);
                trace_result.state_diff.unwrap()
            }
            Err(e) => {
                warn!("trace_call error: {:?}", e);
                return Err(anyhow!("trace_call error: {:?}", e));
            }
        };

        info!("state_diffs: {:?}", state_diffs);

        // let touched_pools = state_diffs.keys().collect::<Vec<_>>();
        let touched_pools: Vec<Pool> = state_diffs.keys().filter_map(|e| self.pools.get(e).map(|p| (p.value()).clone())).collect();

        info!("touched_pools_len: {:?}", touched_pools.len());

        if !touched_pools.is_empty() {
            debug!("⭕🌱 touched_pools: {:?}", touched_pools);
        }

        // if touched_pools.is_empty() {
        //     return Ok(vec![]);
        // }

        Ok(())
    }

    pub async fn get_touched_pools_by_debug_trace_call(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        pending_tx: &Transaction,
        new_block: &NewBlock,
    ) -> Result<Vec<SwapInfo>> {
        let mut swap_info_vec: Vec<SwapInfo> = vec![];

        let mut opts = GethDebugTracingCallOptions::default();
        let mut call_config = CallConfig::default();
        call_config.with_log = Some(true);
        opts.tracing_options.tracer = Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer));
        opts.tracing_options.tracer_config = serde_json::to_value(call_config).unwrap().into();

        let tx_hash = pending_tx.hash;
        let block_number = new_block.block_number;
        let mut tx = pending_tx.clone();
        let nonce = provider.get_transaction_count(tx.from).block_id(block_number.into()).await.unwrap_or_default();
        tx.nonce = nonce;

        let tx_request = tx.into_request();

        let trace = provider.debug_trace_call(tx_request, block_number.into(), opts).await;

        let frame = match trace {
            Ok(geth_trace) => match geth_trace {
                GethTrace::CallTracer(call_frame) => Ok::<Option<CallFrame>, anyhow::Error>(Some(call_frame)),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
        .unwrap();

        if frame.is_none() {
            debug!("debug_trace_call frame is none");
            return Ok(swap_info_vec);
        }

        let frame = frame.unwrap();

        let mut logs = Vec::new();
        extract_logs(&frame, &mut logs);
        debug!("debug_trace_call logs len: {:?}", logs.len());

        for log in &logs {
            match &log.topics {
                Some(topics) => {
                    if topics.len() > 1 {
                        let selector = &format!("{:?}", topics[0])[0..10]; // Gets first 4 bytes (8 chars + "0x") "0xd78ad95f"
                        let is_v2_swap = selector == V2_SWAP_EVENT_ID;
                        let is_v3_swap = selector == V3_SWAP_EVENT_ID;
                        if is_v2_swap {
                            let pair_address = log.address.unwrap();
                            let pool = self.pools.get(&pair_address);
                            if pool.is_none() {
                                debug!("touched_pool_not_in_memory v2: {:?}", &pair_address);
                                continue;
                            }

                            let pool = pool.unwrap().value().clone();

                            // info!("⭕🌱  touched v2 pool: {:?}", &pool);

                            // direction
                            // debug!("🌱  touched v2 swap log: {:?}", &log.data);
                            let (in0, _, _, out1) = match IOne::UniswapV2Swap::abi_decode_data(&log.data.as_ref().unwrap(), false) {
                                Ok(input) => input,
                                _ => {
                                    let zero = U256::ZERO;
                                    (zero, zero, zero, zero)
                                }
                            };
                            let zero_for_one = (in0 > U256::ZERO) && (out1 > U256::ZERO);
                            let direction = if pool.is_token0_main() {
                                if zero_for_one {
                                    SwapDirection::Buy
                                } else {
                                    SwapDirection::Sell
                                }
                            } else {
                                if zero_for_one {
                                    SwapDirection::Sell
                                } else {
                                    SwapDirection::Buy
                                }
                            };
                            info!("🌱  touched_pool v2: {:?}, {:?}", &pool.address, direction);

                            // swap_info_vec.push((pair_address, direction));
                            // swap_info.insert(pair_address, direction);

                            let swap_info = SwapInfo {
                                tx_hash,
                                target_pair: pair_address,
                                main_currency: pool.get_main_token(),
                                target_token: pool.get_other_token(),
                                version: pool.version,
                                token0_is_main: pool.is_token0_main(),
                                direction,
                            };

                            swap_info_vec.push(swap_info);
                        } else if is_v3_swap {
                            let pair_address = log.address.unwrap();
                            let pool = self.pools.get(&pair_address);
                            if pool.is_none() {
                                debug!("touched_pool_not_in_memory v3: {:?}", &pair_address);
                                continue;
                            }

                            let pool = pool.unwrap().value().clone();
                            // info!("⭕🌱  touched v3 pool: {:?}", &pool);

                            //  direction
                            // info!("🌱  touched v3 swap log: {:?}", &log.data);
                            let (in0, in1, _, _, _) = match IOne::UniswapV3Swap::abi_decode_data(&log.data.as_ref().unwrap(), false) {
                                Ok(input) => input,
                                _ => {
                                    let zero = U256::ZERO;
                                    let izero = I256::ZERO;
                                    (izero, izero, zero, 0, 0)
                                }
                            };
                            let zero_for_one = (in0 > I256::ZERO) && (in1 < I256::ZERO);
                            let direction = if pool.is_token0_main() {
                                if zero_for_one {
                                    SwapDirection::Buy
                                } else {
                                    SwapDirection::Sell
                                }
                            } else {
                                if zero_for_one {
                                    SwapDirection::Sell
                                } else {
                                    SwapDirection::Buy
                                }
                            };
                            info!("🌱  touched_pool v3: {:?}, {:?}", &pool.address, direction);
                            let swap_info = SwapInfo {
                                tx_hash,
                                target_pair: pair_address,
                                main_currency: pool.get_main_token(),
                                target_token: pool.get_other_token(),
                                version: pool.version,
                                token0_is_main: pool.is_token0_main(),
                                direction,
                            };

                            swap_info_vec.push(swap_info);
                        }
                    }
                }
                _ => {}
            }
        }
        // make elements unique
        let mut seen = HashSet::new();
        let unique_swap_info_vec: Vec<SwapInfo> = swap_info_vec
            .into_iter()
            .filter(|swap_info| seen.insert((swap_info.target_pair.clone(), swap_info.direction.clone())))
            .collect();

        Ok(unique_swap_info_vec)
    }

    pub fn generate_swap_path_from_touched_pool(
        &self,
        touched_pool_address: &Address,
        direction: SwapDirection,
    ) -> Vec<Vec<IOne::SwapParams>> {
        let touched_pool = self.pools.get(touched_pool_address).unwrap().value().clone();

        let mut paths: Vec<Vec<IOne::SwapParams>> = vec![];
        let other_pools = touched_pool.get_other_pools();

        for other_pool_address in other_pools {
            if other_pool_address == touched_pool.address {
                continue;
            }
            let mut path: Vec<IOne::SwapParams> = vec![];
            let other_pool = self.pools.get(&other_pool_address).unwrap().value().clone();

            if direction == SwapDirection::Buy {
                path.push(IOne::SwapParams {
                    protocol: if other_pool.version == DexVariant::UniswapV2 { 2 } else { 3 },
                    handler: other_pool.address,
                    tokenIn: other_pool.get_main_token(),
                    tokenOut: other_pool.get_other_token(),
                    fee: other_pool.fee.unwrap_or_default(),
                    amount: U256::ZERO,
                });
                path.push(IOne::SwapParams {
                    protocol: if touched_pool.version == DexVariant::UniswapV2 { 2 } else { 3 },
                    handler: touched_pool.address,
                    tokenIn: touched_pool.get_other_token(),
                    tokenOut: touched_pool.get_main_token(),
                    fee: touched_pool.fee.unwrap_or_default(),
                    amount: U256::ZERO,
                });
            } else if direction == SwapDirection::Sell {
                path.push(IOne::SwapParams {
                    protocol: if other_pool.version == DexVariant::UniswapV2 { 2 } else { 3 },
                    handler: touched_pool.address,
                    tokenIn: touched_pool.get_main_token(),
                    tokenOut: touched_pool.get_other_token(),
                    fee: touched_pool.fee.unwrap_or_default(),
                    amount: U256::ZERO,
                });
                path.push(IOne::SwapParams {
                    protocol: if other_pool.version == DexVariant::UniswapV2 { 2 } else { 3 },
                    handler: other_pool.address,
                    tokenIn: other_pool.get_other_token(),
                    tokenOut: other_pool.get_main_token(),
                    fee: other_pool.fee.unwrap_or_default(),
                    amount: U256::ZERO,
                });
            }
            paths.push(path);
        }

        paths
    }
}

pub fn extract_logs(call_frame: &CallFrame, logs: &mut Vec<CallLogFrame>) {
    let ref logs_vec = call_frame.logs;
    if !logs_vec.is_empty() {
        logs.extend(logs_vec.iter().cloned());
    }

    if let ref calls_vec = call_frame.calls {
        for call in calls_vec {
            extract_logs(call, logs);
        }
    }
}

// create pool manager and load pools from json file
pub fn load_pools() -> Result<PoolManager> {
    let path = std::env::var("POOL_DATA_PATH").unwrap();
    let json_from_file = fs::read_to_string(path)?;
    let pools: Vec<Pool> = serde_json::from_str(&json_from_file)?;

    let pools_map: DashMap<Address, Pool> = DashMap::new();

    let poolManager = PoolManager { pools: DashMap::new() };
    for pool in pools {
        poolManager.pools.insert(pool.address, pool);
    }

    Ok(poolManager)
}

mod tests {
    use super::*;
    use one_common::{init_logs, measure_end, measure_start};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_arbi_pools_load() -> Result<()> {
        init_logs();
        /*
        [
            {
            "id": 1,
            "version": "v3",
            "address": "0x3f153545fa22cddad697156bffe34dac7bc5021a",
            "token0": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
            "token1": "0xfd957f21bd95e723645c07c48a2d8acb8ffb3794",
            "symbol0": "WETH",
            "symbol1": "ETHM",
            "decimals0": 18,
            "decimals1": 18,
            "fee": 500,
            "other_pools": [
                "0x3f153545fa22cddad697156bffe34dac7bc5021a",
                "0x70b6e82ba0e3ca4539057dc64ef8e89bed479edf",
                "0xc60604a8e104940cf28f4fd9af8abb06dc50b812",
                "0xf6a42a1963b34ad95bc82c8afe1cadf27b0abf2d",
                "0xfeeed96fdcaa5632c7def0bc18c483f1d9f6079b"
            ]
        }
        ]
        */

        let json_from_file = fs::read_to_string("../.data/eth-uniswap-v2-v3-path.json")?;
        let pools: Vec<Pool> = serde_json::from_str(&json_from_file)?;
        let pool_map = DashMap::new();
        for pool in &pools {
            pool_map.insert(pool.address, pool);
        }

        info!("pools: {:?}", pools.len());
        info!("pools: {:?}", pools[1]);
        info!("pools 2: {:?}", pool_map.get(&pools[1].other_pools[0]).unwrap().value());

        // test touched pools

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_arbi_touched_pools() -> Result<()> {
        init_logs();
        std::env::set_var("POOL_DATA_PATH", "../.data/eth-uniswap-v2-v3-path.json");

        let poolManager = load_pools()?;

        let addresses = vec![
            Address::from_str("0xb9ebf49f3c12a3f9aa18f4ff0383c0ec29750070").unwrap(),
            Address::from_str("0xf57f918bf2f645895486f69d32894398c371086d").unwrap(),
        ];

        let touched_pools: Vec<Pool> = addresses.iter().filter_map(|e| poolManager.pools.get(e).map(|p| (p.value()).clone())).collect();

        info!("touched_pools: {:?}", touched_pools);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_generate_swap_path_from_touched_pool() -> Result<()> {
        init_logs();
        std::env::set_var("POOL_DATA_PATH", "../.data/eth-uniswap-v2-v3-path.json");

        let poolManager = load_pools()?;
        let touched_pool =
            poolManager.pools.get(&Address::from_str("0x3f153545fa22cddad697156bffe34dac7bc5021a").unwrap()).unwrap().value().clone();

        let paths = poolManager.generate_swap_path_from_touched_pool(&touched_pool.address, SwapDirection::Buy);

        info!("touched_pool: {:?}", touched_pool);
        info!("paths: {:?}", paths);

        Ok(())
    }
}
