use revm::primitives::{U256, Address, Bytes};

// trait to convert U256 to u64
pub trait AsU64 {
    fn as_u64(self) -> u64;
}

impl AsU64 for U256 {
    fn as_u64(self) -> u64 {
        self.as_limbs()[0]
    }
}


#[derive(Debug, Clone)]
pub struct Tx {
    pub caller: Address,
    pub transact_to: Address,
    pub data: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

#[derive(Debug, Clone)]
pub struct TxResult {
    pub output: Bytes,
    pub gas_used: u64,
    pub gas_refunded: u64,
}