use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{Address, U128, U256, U64},
    providers::Provider,
    rpc::types::eth::{Block, Filter, Log, Transaction},
};

#[derive(Default, Debug, Clone)]
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
      Self {
          added_block: None,
          tx: Transaction::default(),
      }
  }
}


#[derive(Debug, Clone)]
pub enum Event {
    Block(NewBlock),
    PendingTx(NewPendingTx),
    // PoolTouched(PoolTouchedLog),
}
