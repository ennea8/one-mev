use revm::{
    precompile::{PrecompileSpecId, Precompiles},
    primitives::SpecId,
};

// use std::collections::BTreeMap;
use alloy::{
    primitives::{Address, Bytes, B256, U256},
    rpc::types::{AccessList, AccessListItem, Header, Transaction, TransactionRequest},
};
use eyre::{eyre, OptionExt, Result};
use lazy_static::lazy_static;
use revm::interpreter::Host;
use revm::primitives::{Account, BlockEnv, Env, ExecutionResult, Output, ResultAndState, TransactTo, TxEnv, SHANGHAI};
use revm::{Database, DatabaseCommit, DatabaseRef, Evm};
use std::convert::Infallible;

pub fn get_precompiles_for(spec_id: SpecId) -> Vec<Address> {
    Precompiles::new(PrecompileSpecId::from_spec_id(spec_id)).addresses().copied().collect()
}

// TODO check
lazy_static! {
    static ref COINBASE: Address = "0x1f9090aaE28b8a3dCeaDf281B0F12828e676c326".parse().unwrap();
}

// TODO add test case
pub fn evm_access_list<DB>(state_db: DB, env: &Env, tx: &TransactionRequest) -> Result<(u64, AccessList)>
where
    DB: DatabaseRef<Error = Infallible>,
{
    let mut env = env.clone();

    let txto = tx.to.unwrap_or_default().to().map_or(Address::ZERO, |x| *x);

    env.tx.chain_id = tx.chain_id;
    env.tx.transact_to = TransactTo::Call(txto);
    env.tx.nonce = tx.nonce;
    env.tx.data = tx.input.clone().input.unwrap();
    env.tx.value = tx.value.unwrap_or_default();
    env.tx.caller = tx.from.unwrap_or_default();
    env.tx.gas_price = U256::from(tx.max_fee_per_gas.unwrap_or(tx.gas_price.unwrap_or_default()));
    env.tx.gas_limit = tx.gas.unwrap_or_default() as u64;
    env.tx.gas_priority_fee = Some(U256::from(tx.max_priority_fee_per_gas.unwrap_or_default()));

    env.block.coinbase = *COINBASE;

    let mut evm = Evm::builder().with_ref_db(state_db).with_spec_id(SHANGHAI).with_env(Box::new(env)).build();

    match evm.transact() {
        Ok(execution_result) => match execution_result.result {
            ExecutionResult::Success { output, gas_used, reason, .. } => {
                debug!("AccessList Gas used : {gas_used} reason : {reason:?}");
                debug!("AccessList Output : {output:?}");
                let mut acl = AccessList::default();

                for (addr, acc) in execution_result.state {
                    let storage_keys: Vec<B256> = acc.storage.keys().map(|x| (*x).into()).collect();
                    acl.0.push(AccessListItem { address: addr, storage_keys });
                }

                Ok((gas_used, acl))
            }
            ExecutionResult::Revert { output, gas_used } => {
                error!("Revert {output} gas used {gas_used}");
                Err(eyre!("EXECUTION_REVERTED"))
            }
            ExecutionResult::Halt { reason, .. } => {
                error!("Halt {reason:?}");
                Err(eyre!("EXECUTION_HALT"))
            }
        },
        Err(e) => {
            error!("Execution error : {e}");
            Err(eyre!("EXECUTION_ERROR"))
        }
    }
}
