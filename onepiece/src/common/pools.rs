use anyhow::{anyhow, Result};
use std::fs;
use std::{collections::HashMap, str::FromStr, sync::Arc};

use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{Address, U128, U256, U64},
    providers::Provider,
    rpc::types::eth::{Block, Filter, Log, Transaction},
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum DexVariant {
    UniswapV2, // 2
}

impl DexVariant {
    pub fn num(&self) -> u8 {
        match self {
            DexVariant::UniswapV2 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pool {
    pub id: i64,
    pub address: Address,
    pub version: DexVariant,
    pub token0: Address,
    pub token1: Address,
    pub fee: u32, // uniswap v3 specific
    pub block_number: u64,
    pub timestamp: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PoolTouchedInfo {
    pub pool_v3: Option<Address>,
    pub pool_v2: Address,
    pub token0: Address,
    pub token1: Address,
    pub fee_tier: Option<u32>,
    pub decimals0: u32,
    pub decimals1: u32,
    //pub symbol0: Option<String>,
    //pub symbol1: Option<String>,
}

pub(crate) struct PoolManager {
    pub poolTouched: DashMap<Address, PoolTouchedInfo>,
}

pub fn load_all_pools() -> Result<PoolManager> {
    let pool_data_path: String = std::env::var("POOL_DATA_PATH").unwrap();

    info!("load_all_pools {}", pool_data_path);

    let json_from_file = fs::read_to_string(pool_data_path)?;

    let pools: Vec<PoolTouchedInfo> = serde_json::from_str(&json_from_file)?;

    let poolManager = PoolManager { poolTouched: DashMap::new() };
    for pool in &pools {
        poolManager.poolTouched.insert(pool.pool_v2, pool.clone()); // TODO 优化为引用& ？
                                                                    // poolManager.poolTouched.insert(pool.pool_v3, pool.clone());
    }
    info!("pools len: {}", pools.len());

    Ok(poolManager)
}

pub fn load_all_pools_hashmap() -> Result<HashMap<Address, PoolTouchedInfo>> {
    let pool_data_path: String = std::env::var("POOL_DATA_PATH").unwrap();
    let json_from_file = fs::read_to_string(pool_data_path)?;

    let pools: Vec<PoolTouchedInfo> = serde_json::from_str(&json_from_file)?;

    let mut poolHashMap = HashMap::new();
    for pool in &pools {
        poolHashMap.insert(pool.pool_v2, pool.clone()); // TODO 优化为引用& ？
    }
    Ok(poolHashMap)
}

mod tests {
    use super::*;
    use one_common::{init_logs, print_banner};

    #[test]
    fn test_load_all_pools() {
        let path = std::env::var("POOL_DATA_PATH").unwrap();
        std::env::set_var("POOL_DATA_PATH", format!("../{}", path));

        init_logs();

        let result = load_all_pools();

        assert!(result.is_ok());
    }
}
