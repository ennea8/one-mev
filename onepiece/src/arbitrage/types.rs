use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::abi::IOne;
use crate::simulation::VictimTx;
use alloy::primitives::{Address, Bytes, TxHash, I256, U256};
use alloy::rpc::types::eth::AccessList;
use alloy::rpc::types::eth::Transaction;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum DexVariant {
    #[serde(alias = "v2")]
    UniswapV2,
    #[serde(alias = "v3")]
    UniswapV3,
}

impl DexVariant {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "UniswapV2" => Ok(DexVariant::UniswapV2),
            "UniswapV3" => Ok(DexVariant::UniswapV3),
            _ => Err(anyhow!("Unknown dex variant: {}", s)),
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Pool {
    pub id: u32,
    pub address: Address,
    pub version: DexVariant,
    pub token0: Address,
    pub token1: Address,
    pub decimals0: u8,
    pub decimals1: u8,
    pub fee: Option<u32>,
    pub other_pools: Vec<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SwapDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub tx_hash: TxHash,
    pub target_pair: Address,
    pub main_currency: Address,
    pub target_token: Address,
    pub version: DexVariant,
    pub token0_is_main: bool,
    pub direction: SwapDirection,
}

#[derive(Debug, Default, Clone)]
pub struct One {
    pub pieces: Vec<Piece>,
}
#[derive(Debug, Clone)]
pub struct Piece {
    // basic info
    pub victim_tx: VictimTx,
    pub swap_info: SwapInfo,
    pub updated_at: u64,

    // mev items
    pub arbitrage: Option<Arbitrage>,
    pub sandwich: Option<Sandwich>,
    // liquidation
}

#[derive(Debug, Clone)]
pub struct Arbitrage {
    pub path: Vec<IOne::SwapParams>,
    //pub amount_in: U256,
    pub optimized_in: U256,
    pub max_revenue: I256,
    
    // optimization info
    // TODO add gas_used and access_list
    // pub access_list: AccessList,
    // pub calldata: Bytes,
    // pub gas_used: u64,
}

#[derive(Debug, Default, Clone)]
pub struct Sandwich {
    pub amount_in: U256,
    pub max_revenue: U256,
    pub front_gas_used: u64,
    pub back_gas_used: u64,
    pub front_access_list: AccessList,
    pub back_access_list: AccessList,
    pub front_calldata: Bytes,
    pub back_calldata: Bytes,
}

#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct NewBlock {
    pub block_number: u64,
    pub base_fee: U256,
    pub next_base_fee: U256,
}

#[derive(Debug, Clone)]
pub struct NewPendingTx {
    pub added_block: Option<u64>,
    pub tx: Transaction,
}

impl Default for NewPendingTx {
    fn default() -> Self {
        Self { added_block: None, tx: Transaction::default() }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PendingTxInfo {
    pub pending_tx: NewPendingTx,
    pub touched_pairs: Vec<SwapInfo>,
}

#[derive(Debug, Clone)]
pub struct BackrunAction {
    pub new_block: NewBlock,
    pub pending_txs: Vec<Transaction>,
    pub back_calldata: Bytes,
    pub realistic_back_gas_limit: u64,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
}

#[derive(Debug, Clone)]
pub enum Event {
    Block(NewBlock),
    PendingTx(NewPendingTx),
}

#[derive(Debug, Clone)]
pub enum ActionEvent {
    Backrun(BackrunAction),
}
