
pub use crate::types::{Tx, TxResult};
use crate::{fork_db::ForkDB, fork_factory::ForkFactory, abis, config::cache_dir};


use anyhow::{anyhow, ensure, Result};
use std::{collections::HashMap, str::FromStr, sync::Arc};

// alloy
use alloy::pubsub::PubSubFrontend;
use alloy::rpc::types::eth::{Block, BlockId, BlockNumberOrTag};
use alloy_provider::{Provider, ProviderBuilder, ReqwestProvider, RootProvider};
use alloy_sol_types::{SolCall, SolValue};

//revm
use revm::{
    db::{CacheDB, EmptyDB},
    interpreter::Host,
    primitives::{
        address, keccak256, AccountInfo, Address, Bytecode, Bytes, ExecutionResult, Output,
        TransactTo, U256,
    },
    Database, DatabaseRef, Evm,
};

use alloy_primitives::I256;
use lazy_static::lazy_static;

use std::path::{Path, PathBuf};

lazy_static! {
    pub static ref WETH: Address =
        Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
    pub static ref USDC: Address =
        Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
}

#[derive(Clone, Debug)]
pub struct EvmSimulator {
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    pub fork_factory: ForkFactory,
    pub block_number: u64,
}

impl EvmSimulator {
    pub fn new(
        provider: Arc<RootProvider<PubSubFrontend>>,
        initial_db: CacheDB<EmptyDB>,
        block_number: u64,
    ) -> Self {
        let block_id = BlockId::Number(BlockNumberOrTag::Number(block_number));

        let cache_db = CacheDB::new(EmptyDB::default());

        // todo check if we need to use 'mut'
        let mut fork_factory =
            ForkFactory::new_sandbox_factory(provider.clone(), cache_db, Some(block_id));

        Self {
            provider,
            fork_factory,
            block_number,
        }
    }

    // fork db and share the background
    // can be used in concurrency
    pub fn new_evm(&self, move_block: bool) -> Evm<(), ForkDB> {
        let fork_db = self.fork_factory.new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();
        // TODO number should be always setted

        let block_number = if move_block {self.block_number + 1} else {self.block_number};
        evm.block_mut().number = U256::from(block_number);
        
        evm
    }

    pub async fn init_account(&mut self, address: Address) -> Result<()> {
        let cache_key = format!("bytecode-{:?}", address);

        let bytecode = match cacache::read(&cache_dir(), cache_key.clone()).await {
            Ok(bytecode) => {
                let bytecode = Bytes::from(bytecode);
                Bytecode::new_raw(bytecode)
            }
            Err(_e) => {
                let bytecode = self
                    .provider
                    .get_code_at(address).block_id( Default::default())// TODO check block_id
                    .await?;
                let bytecode_result = Bytecode::new_raw(bytecode.clone());
                let bytecode = bytecode.to_vec();
                cacache::write(&cache_dir(), cache_key, bytecode.clone()).await?;
                bytecode_result
            }
        };

        let code_hash = bytecode.hash_slow();
        let acc_info = AccountInfo {
            balance: U256::ZERO,
            nonce: 0_u64,
            code: Some(bytecode),
            code_hash,
        };
        self.fork_factory.insert_account_info(address, acc_info);
        Ok(())
    }

    // Can be EOA or contract, if contract, bytecode is required
    pub fn insert_account(&mut self, address: Address, bytecode: Bytecode) -> Result<()> {
        let code_hash = bytecode.hash_slow();
        let code = if bytecode.len() == 0 {
            None
        } else {
            Some(bytecode)
        };

        let acc_info = AccountInfo {
            balance: U256::ZERO,
            nonce: 0_u64,
            code,
            code_hash,
        };

        // TODO with weth balannce?

        self.fork_factory.insert_account_info(address, acc_info);
        Ok(())
    }

    pub fn init_eoa_account(
        &mut self,
        address: Address,
        balance: U256,
        weth_balance: U256,
    ) -> Result<()> {
        let acc_info = AccountInfo {
            balance,
            nonce: 0_u64,
            code: None,
            code_hash: keccak256(Bytes::new()).into(),
        };
        self.fork_factory.insert_account_info(address, acc_info);

        if (weth_balance > U256::from(0)) {
            self.set_weth_balance(address, weth_balance)?;
        }

        Ok(())
    }

    pub fn insert_account_storage(
        &mut self,
        contract: Address,
        slot: U256,
        slot_address: Address,
        value: U256,
    ) -> Result<()> {
        let hashed_balance_slot = keccak256((slot_address, slot).abi_encode());

        self.fork_factory
            .insert_account_storage(contract, hashed_balance_slot.into(), value)?;
        Ok(())
    }

    // need to make sure the account is already inserted/inited locally
    pub fn set_weth_balance(
        &mut self,
        address: Address,
        amount: U256,
    ) -> Result<(), anyhow::Error> {
        // To fund any ERC20 token to an account we need the balance storage slot of the token
        // For WETH its 3
        // An amazing online tool to see the storage mapping of any contract https://evm.storage/
        let slot_num = U256::from(3);
        let addr_padded = pad_left(address.to_vec(), 32);
        let slot = slot_num.to_be_bytes_vec();

        let data = [&addr_padded, &slot]
            .iter()
            .flat_map(|x| x.iter().copied())
            .collect::<Vec<u8>>();
        let slot_hash = keccak256(&data);
        let slot: U256 = U256::from_be_bytes(slot_hash.try_into().expect("Hash must be 32 bytes"));

        // insert the erc20 token balance to the dummy account
        if let Err(e) = self
            .fork_factory
            .insert_account_storage((*WETH), slot, amount)
        {
            return Err(anyhow::anyhow!("Failed to insert account storage: {}", e));
        }

        Ok(())
    }

    pub fn set_balance() {}
    pub fn get_balance(&self, account: Address) -> Result<U256> {
        let fork_db = self.fork_factory.new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();

        let (balance, _) = evm.context.balance(account).unwrap();
        Ok(balance)
    }

    pub fn get_balance2(&mut self, account: Address) -> Result<U256> {
        let mut fork_db = self.fork_factory.new_sandbox_fork();
        let acc = fork_db.basic(account).unwrap().unwrap();
        Ok(acc.balance)
    }

    // TODO test
    pub fn get_erc20_balance(&self, token_address: Address, account: Address)->Result<U256> {

        let fork_db = self.fork_factory.new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();

        let balance_of_calldata = abis::ERC20::balanceOfCall {
            owner: account,
        }.abi_encode();
        let balance_of_calldata = Bytes::from(balance_of_calldata);

        debug!("get_erc20_balance.balance_of_calldata: {:?}", balance_of_calldata);

        let tx = Tx {
            caller: Address::ZERO,
            transact_to: token_address,
            data: balance_of_calldata,
            value: U256::ZERO,
            gas_limit: 50000000,
        };

        let balance = call_static(&mut evm, tx).map(|res| {
            let balance = abis::ERC20::balanceOfCall::abi_decode_returns(&res.output, false).unwrap();
            balance.balance
        })?;

        Ok(balance)
    }

    // ////////////////////////////////////
    // static method
}

fn _call(evm: &mut Evm<(), ForkDB>, tx: Tx, commit: bool) -> Result<TxResult> {
    // block
    // evm.block_mut().number = U256::from(self.block_number + 1);

    // tx
    evm.tx_mut().caller = tx.caller.into();
    evm.tx_mut().transact_to = TransactTo::Call(tx.transact_to.into());
    evm.tx_mut().data = tx.data;
    evm.tx_mut().value = tx.value.into();
    evm.tx_mut().gas_limit = 50000000; // TODO check tx.gaslimit

    //  Disable some checks for easier testing
    //  evm.cfg_mut().disable_balance_check = true;
    //  evm.cfg_mut().disable_block_gas_limit = true;
    //  evm.cfg_mut().disable_base_fee = true;

    let result;

    if commit {
        result = match evm.transact_commit() {
            Ok(result) => result,
            Err(e) => return Err(anyhow!("EVM call failed: {:?}", e)),
        };
    } else {
        let ref_tx = evm
            .transact()
            .map_err(|e| anyhow!("EVM staticcall failed: {:?}", e))?;
        result = ref_tx.result;
    }

    // TODO check result
    let output = match result {
        ExecutionResult::Success {
            gas_used,
            gas_refunded,
            output,
            ..
        } => match output {
            Output::Call(o) => TxResult {
                output: o,
                gas_used,
                gas_refunded,
            },
            Output::Create(o, _) => TxResult {
                output: o,
                gas_used,
                gas_refunded,
            },
        },
        ExecutionResult::Revert { gas_used, output } => {
            error!("_call EVM REVERT: {:?} / Gas used: {:?}", output, gas_used);
    
            return Err(anyhow!(
                "EVM REVERT: {:?} / Gas used: {:?}",
                output,
                gas_used
            ))
        }
        ExecutionResult::Halt { reason, .. } => return Err(anyhow!("EVM HALT: {:?}", reason)),
    };

    debug!("_call EVM call output: {:?}", output);

    Ok(output)
}

pub fn call(evm: &mut Evm<(), ForkDB>, tx: Tx) -> Result<TxResult> {
    _call(evm, tx, true)
}

pub fn call_static(evm: &mut Evm<(), ForkDB>, tx: Tx) -> Result<TxResult> {
    _call(evm, tx, false)
}

fn pad_left(vec: Vec<u8>, full_len: usize) -> Vec<u8> {
    let mut padded = vec![0u8; full_len - vec.len()];
    padded.extend(vec);
    padded
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloy_primitives::utils::parse_ether;
    // use alloy_signer_wallet::LocalWallet;
    use alloy_signer_local::LocalSigner;

    use alloy::transports::ws::WsConnect;
    use alloy::{
        primitives::{Address, U128, U256, U64},
        providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
        pubsub::PubSubFrontend,
        rpc::types::eth::{Block, Log, Transaction},
    };
    use alloy_transport_http::Http;

    use reqwest::Client;

    use std::sync::Once;
    static INIT: Once = Once::new();
    fn init_tracing() {
        INIT.call_once(|| {
            let _ = tracing_subscriber::fmt::try_init();
        });
    }

    pub async fn create_default_wss_provider(
    ) -> Result<Arc<RootProvider<PubSubFrontend>>, anyhow::Error> {
        let url: &str = "wss://rpc.ankr.com/eth/ws/dbfed5edb557956802a57d9f327ed66469d08ff3a70c3ee16b406ecca32cb67f";
        let client = ProviderBuilder::new().on_ws(WsConnect::new(url)).await?;
        Ok(Arc::new(client))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sim2_init_account() -> Result<()> {
        init_tracing();


        info!("test_sim2_init_account");
        debug!("test_sim2_init_account");

        let provider = create_default_wss_provider().await.unwrap();

        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let mut sim = EvmSimulator::new(provider.clone(), cache_db, block_number);

        let user001 = LocalSigner::random().address();
        let eth_100 = parse_ether("100")?;

        let _ = sim.init_eoa_account(user001, eth_100, eth_100);

        let eth_balance = sim.get_balance(user001);

        assert_eq!(eth_balance.unwrap(), eth_100);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_usdc_balance()-> Result<()> {
        init_tracing();

        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let sim = EvmSimulator::new(provider.clone(), cache_db, block_number);


        let usdc_address = Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
        let account = Address::from_str("0x98C23E9d8f34FEFb1B7BD6a91B7FF122F4e16F5c").unwrap(); // aave v3

        let balance = sim.get_erc20_balance(usdc_address, account);

        info!("usdc balance: {:?}", balance);

        Ok(())

    }
}
