use anyhow::{anyhow, ensure, Result};
use std::{cell::RefCell, collections::HashMap, str::FromStr, sync::Arc};

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

use crate::types::{Tx, TxResult};
use crate::{abis, config::cache_dir, fork_db::ForkDB, fork_factory::ForkFactory};

use alloy_primitives::I256;
use lazy_static::lazy_static;

use std::path::{Path, PathBuf};

use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct SimulatorFactory {
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    pub fork_factory: Arc<RwLock<ForkFactory>>,
    pub block_number: u64,
}

impl SimulatorFactory {
    pub fn new(
        provider: Arc<RootProvider<PubSubFrontend>>,
        initial_db: CacheDB<EmptyDB>,
        block_number: u64,
    ) -> Self {
        let block_id = BlockId::Number(BlockNumberOrTag::Number(block_number));

        let cache_db = CacheDB::new(EmptyDB::default());

        // todo check if we need to use 'mut'
        let fork_factory =
            ForkFactory::new_sandbox_factory(provider.clone(), cache_db, Some(block_id));

        Self {
            provider,
            fork_factory: Arc::new(RwLock::new(fork_factory)),
            block_number,
        }
    }
    pub fn init_eoa_account(&self, address: Address, balance: U256) -> Result<()> {
        let acc_info = AccountInfo {
            balance,
            nonce: 0_u64,
            code: None,
            code_hash: keccak256(Bytes::new()).into(),
        };
        self.fork_factory
            .write()
            .unwrap()
            .insert_account_info(address, acc_info);
        Ok(())
    }

    pub fn get_balance(&self, account: Address) -> Result<U256> {
        let fork_db = self.fork_factory.read().unwrap().new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();

        let (balance, _) = evm.context.balance(account).unwrap();
        Ok(balance)
    }

    pub fn new_fork_simulator(&self, move_block: bool) -> Simulator {
        let fork_db = self.fork_factory.read().unwrap().new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();
        if move_block {
            evm.block_mut().number = U256::from(self.block_number + 1);
        }

        Simulator {
            block_number: self.block_number,
            evm,
        }
    }
}

#[derive(Debug)]
pub struct Simulator<'a> {
    // pub provider: Arc<RootProvider<PubSubFrontend>>,
    // pub fork_factory: ForkFactory,
    pub block_number: u64,
    pub evm: Evm<'a, (), ForkDB>,
}

impl<'a> Simulator<'a> {
    pub fn get_fork_db_mut(&mut self) -> &mut ForkDB {
        self.evm.db_mut()
    }

    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), std::convert::Infallible> {
        self.evm
            .db_mut()
            .db
            .insert_account_storage(address, slot, value)
    }

    pub fn get_balance(&mut self, account: Address) -> Result<U256> {
        let (balance, _) = self.evm.context.balance(account).unwrap();
        Ok(balance)
    }

    pub fn call(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, true)
    }

    pub fn call_static(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, false)
    }

    fn _call(&mut self, tx: Tx, commit: bool) -> Result<TxResult> {
        // block
        // evm.block_mut().number = U256::from(self.block_number + 1);

        // tx
        self.evm.tx_mut().caller = tx.caller.into();
        self.evm.tx_mut().transact_to = TransactTo::Call(tx.transact_to.into());
        self.evm.tx_mut().data = tx.data;
        self.evm.tx_mut().value = tx.value.into();
        self.evm.tx_mut().gas_limit = 50000000; // TODO check tx.gaslimit

        // Disable some checks for easier testing
        //  evm.cfg_mut().disable_balance_check = true;
        //  evm.cfg_mut().disable_block_gas_limit = true;
        //  evm.cfg_mut().disable_base_fee = true;

        let result;

        if commit {
            result = match self.evm.transact_commit() {
                Ok(result) => result,
                Err(e) => return Err(anyhow!("EVM call failed: {:?}", e)),
            };
        } else {
            let ref_tx = self
                .evm
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
                ));
            }
            ExecutionResult::Halt { reason, .. } => return Err(anyhow!("EVM HALT: {:?}", reason)),
        };

        debug!("_call EVM call output: {:?}", output);

        Ok(output)
    }
}

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_simulator_concurrency() -> Result<()> {
        init_tracing();

        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));

        let eth_100 = parse_ether("100")?;

        // Create 10 users and store them in the users vector
        let mut users = Vec::new();
        for i in 0..10 {
            let user = LocalSigner::random().address();
            evm_factory.init_eoa_account(user, parse_ether((i+1).to_string().as_str())?)?;
            users.push(user);
        }

        // Create 10 EVMs concurrently and read their balances
        let handles: Vec<_> = users
            .iter()
            .enumerate()
            .map(|(index, &user)| {
                let evm_factory = evm_factory.clone();
                tokio::spawn(async move {
                    let mut evm = evm_factory.new_fork_simulator(false);
                    let balance = evm.get_balance(user).unwrap();
                    (index, user, balance)
                })
            })
            .collect();

        // Wait for all tasks to complete and collect results
        let results = futures::future::join_all(handles).await;

        // Log the results
        for result in results {
            let (index, user, balance) = result.unwrap();
            info!("EVM {}: User {:?} balance: {}", index + 1, user, balance);
        }

        Ok(())
    }

}
