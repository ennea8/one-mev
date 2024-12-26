use anyhow::{anyhow, Result};
use std::{str::FromStr, sync::Arc};

// alloy
use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::{Address, TxHash, B256, U256},
    providers::{Provider, RootProvider},
    pubsub::PubSubFrontend,
    rpc::types::eth::{Log, Transaction},
    signers::local::PrivateKeySigner,
    sol_types::{sol, SolCall, SolValue},
};

//revm
use revm::{
    db::{CacheDB, EmptyDB},
    interpreter::Host,
    primitives::{
        address, keccak256, AccountInfo, Bytecode, Bytes, ExecutionResult, Output, SpecId, TransactTo,
    },
    Database, Evm,
};

// use one_evm::types::{Tx, TxResult};
use one_evm::{database_error::DatabaseError, fork_db::ForkDB, fork_factory::ForkFactory};


use crate::abi;
use crate::inspector::access_list::AccessListInspector;

use std::sync::RwLock;

// hardcoded for testing // TODO use config
pub fn caller_address() -> Address {
    address!("29b2F9e909451a6A98Fee9215Ac3648b69598800")
}

#[derive(Debug, Clone, Default)]
pub struct VictimTx {
    pub tx_hash: TxHash,
    pub from: Address,
    pub to: Address,
    pub data: Bytes,
    pub value: U256,
    pub gas_price: U256,
    pub gas_limit: Option<U256>,
    pub gas_priority_fee: Option<U256>,
}

impl VictimTx {
    pub fn from_transaction(tx: Transaction) -> Self {
        let mut tx_req = Self {
            tx_hash: tx.hash,
            from: tx.from,
            to: tx.to.unwrap_or_default(),
            data: tx.input.0.clone().into(), 
            value: tx.value,
            gas_limit: Some(U256::from(tx.gas)),
            gas_price: U256::ZERO,
            gas_priority_fee: None,
        };

        match tx.transaction_type {
            Some(tx_type) => {
                if tx_type == 2 { // eip1559 tx
                    tx_req.gas_price = U256::from(tx.max_fee_per_gas.unwrap_or_default());
                    tx_req.gas_priority_fee = Some(U256::from(tx.max_priority_fee_per_gas.unwrap_or_default()));
                }else{
                     // legacy tx
                    tx_req.gas_price = U256::from(tx.gas_price.unwrap_or_default());
                }
            }
            _ => {
                // legacy tx
                tx_req.gas_price = U256::from(tx.gas_price.unwrap_or_default());
            }
        }

        tx_req
    }
}

// tx info for revm TxEnv type
#[derive(Debug, Clone)]
pub struct Tx {
    pub caller: Address,
    pub transact_to: Address,
    pub data: Bytes,
    pub value: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub gas_priority_fee: Option<U256>,
}

impl Tx {
    pub fn from(tx: VictimTx) -> Self {
        let gas_limit = match tx.gas_limit {
            Some(gas_limit) => gas_limit,
            None => U256::from(DEFAULT_GAS_LIMIT), // Default gas limit for basic transactions
        };
        Self { caller: tx.from, transact_to: tx.to, data: tx.data, value: tx.value, gas_price: tx.gas_price, gas_limit, gas_priority_fee:tx.gas_priority_fee }
    }

    pub fn from_transaction(tx: Transaction) -> Self {
        let mut tx_req = Self {
            caller: tx.from,
            transact_to: tx.to.unwrap(),
            data: tx.input,
            value: tx.value,
            gas_price: U256::from(1000000000),
            gas_limit: U256::from(tx.gas),
            gas_priority_fee: None,
        };

        match tx.transaction_type {
            Some(tx_type) => {
                if tx_type == 2 { // eip1559 tx
                    tx_req.gas_price = U256::from(tx.max_fee_per_gas.unwrap_or_default());
                    tx_req.gas_priority_fee = Some(U256::from(tx.max_priority_fee_per_gas.unwrap_or_default()));
                }else{
                     // legacy tx
                    tx_req.gas_price = U256::from(tx.gas_price.unwrap_or_default());
                }
            }
            _ => {
                // legacy tx
                tx_req.gas_price = U256::from(tx.gas_price.unwrap_or_default());
            }
        }

        tx_req
    }
}

impl From<VictimTx> for Tx {
    fn from(tx: VictimTx) -> Self {
        Self::from(tx)
    }
}

impl From<Transaction> for Tx {
    fn from(tx: Transaction) -> Self {
        Self::from_transaction(tx)
    }
}

#[derive(Debug, Clone)]
pub struct TxResult {
    pub output: Bytes,
    pub logs: Option<Vec<Log>>,
    pub gas_used: u64,
    pub gas_refunded: u64,
}

#[derive(Clone, Debug)]
pub struct SimulatorFactory {
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    pub fork_factory: Arc<RwLock<ForkFactory>>,
    pub block_number: u64,
}

impl SimulatorFactory {
    pub fn new(provider: Arc<RootProvider<PubSubFrontend>>, initial_db: CacheDB<EmptyDB>, block_number: u64) -> Self {
        let block_id = BlockId::Number(BlockNumberOrTag::Number(block_number));

        let cache_db = CacheDB::new(EmptyDB::default());

        // todo check if we need to use 'mut'
        let fork_factory = ForkFactory::new_sandbox_factory(provider.clone(), cache_db, Some(block_id));

        Self { provider, fork_factory: Arc::new(RwLock::new(fork_factory)), block_number }
    }

    // TODO check: if evm share code when create new evm
    pub fn deploy(&self, target: Address, bytecode: Bytecode) {
        // let code_hash = bytecode.hash_slow();
        let contract_info = AccountInfo::new(U256::ZERO, 0, B256::ZERO, bytecode);

        self.fork_factory.write().unwrap().insert_account_info(target, contract_info);
    }

    pub fn init_eoa_account(&self, address: Address, balance: U256) -> Result<()> {
        let acc_info = AccountInfo { balance, nonce: 0_u64, code: None, code_hash: keccak256(Bytes::new()).into() };
        self.fork_factory.write().unwrap().insert_account_info(address, acc_info);
        Ok(())
    }

    pub fn set_eth_balance(&self, target: Address, amount: U256) {
        let user_balance = amount.into();
        let user_info = AccountInfo::new(user_balance, 0, B256::ZERO, Bytecode::default());
        self.fork_factory.write().unwrap().insert_account_info(target, user_info);
    }
    pub fn set_token_balance(&self, token_address: Address, to: Address, slot: U256, amount: U256) -> Result<()> {
        let hashed_balance_slot = keccak256((to, slot).abi_encode());
        self.fork_factory.write().unwrap().insert_account_storage(token_address, hashed_balance_slot.into(), amount);

        Ok(())
    }

    pub fn get_eth_balance(&self, account: Address) -> Result<U256> {
        let fork_db = self.fork_factory.read().unwrap().new_sandbox_fork();
        let mut evm = Evm::builder().with_db(fork_db).build();

        let (balance, _) = evm.context.balance(account).unwrap();
        Ok(balance)
    }

    pub fn new_fork_simulator(&self, move_block: bool) -> Simulator<'_, ()> {
        let fork_db = self.fork_factory.read().unwrap().new_sandbox_fork();
        // let fork_db = {
        //     let fork_factory = self.fork_factory.read().unwrap();
        //     fork_factory.new_sandbox_fork()
        // };

        let mut evm = Evm::builder().with_db(fork_db).build();

        let block_number = if move_block { self.block_number + 1 } else { self.block_number };

        evm.block_mut().number = U256::from(block_number);

        let owner = PrivateKeySigner::random(); // TODO use config
        Simulator { block_number: self.block_number, evm, owner: owner.address() }
    }

    // TODO Fix type error.
    // Refer
    // -foundry/crates/evm/core/src/utils.rs
    // - foundry/crates/anvil/src/eth/backend/mem/mod.rs build_access_list_with_state|build_call_env
    // TODO Add test case
    pub fn new_fork_simulator_with_inspector(&self) -> Simulator<'_, AccessListInspector> {
        let fork_db = self.fork_factory.read().unwrap().new_sandbox_fork();

        // prepare
        let env = Box::<revm::primitives::Env>::default();
        let spec = SpecId::LATEST; // todo check
        let handler_cfg = revm::primitives::HandlerCfg::new(spec);
        // let cfg = revm::primitives::EnvWithHandlerCfg::new(env.clone(), handler_cfg);

        // inspector
        let inspector = AccessListInspector::default();

        // new_evm_with_inspector
        let context = revm::Context::new(revm::EvmContext::new_with_env(fork_db, env), inspector);
        let mut handler = revm::Handler::new(handler_cfg);
        handler.append_handler_register_plain(revm::inspector_handle_register);

        let evm = revm::Evm::new(context, handler);

        let owner = PrivateKeySigner::random(); // TODO use config

        Simulator { block_number: self.block_number, evm, owner: owner.address() }
    }
}

#[derive(Debug)]
pub struct Simulator<'a, EXT> {
    // pub provider: Arc<RootProvider<PubSubFrontend>>,
    // pub fork_factory: ForkFactory,
    pub block_number: u64,
    pub evm: Evm<'a, EXT, ForkDB>,
    pub owner: Address,
}

pub const DEFAULT_GAS_LIMIT: u64 = 5_000_000;

// insert： self.evm.db_mut().db
// get: self.evm.db_mut() // inner memory db should not be used when get！！！
impl<'a, EXT> Simulator<'a, EXT> {

    pub fn get_block_number(&mut self) -> U256 {
        self.evm.block().number
    }

    pub fn get_coinbase(&mut self) -> Address {
        self.evm.block().coinbase
    }

    pub fn get_base_fee(&mut self) -> U256 {
        self.evm.block().basefee
    }

    pub fn set_base_fee(&mut self, base_fee: U256) {
        self.evm.block_mut().basefee = base_fee.into();
    }

    pub fn insert_account_info(&mut self, target: Address, account_info: AccountInfo) {
        self.evm.db_mut().db.insert_account_info(target.into(), account_info);
    }
    // a shortcut method to insert contract account
    pub fn deploy(&mut self, target: Address, bytecode: Bytecode) {
        // code_hash will be calculated when code is not empty
        let contract_info = AccountInfo::new(U256::ZERO, 0, B256::ZERO, bytecode);
        self.insert_account_info(target, contract_info);
    }

    pub fn insert_account_storage(&mut self, address: Address, slot: U256, value: U256) -> Result<(), anyhow::Error> {
        self.get_account(address)?; // load account first
        self.evm.db_mut().db.insert_account_storage(address, slot, value).map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_account_storage(&mut self, address: Address, slot: U256) -> Result<U256, DatabaseError> {
        self.evm.db_mut().storage(address, slot)
    }

    pub fn set_eth_balance(&mut self, target: Address, amount: U256) {
        let user_balance = amount.into();
        let user_info = AccountInfo::new(user_balance, 0, B256::ZERO, Bytecode::default());
        self.insert_account_info(target.into(), user_info);
    }

    pub fn get_eth_balance(&mut self, account: Address) -> U256 {
        let result = self.evm.context.balance(account);
        match result {
            Some((balance, _)) => balance,
            None => {
                error!("get_eth_balance None for {:?}", account);
                U256::ZERO
            }
        }
    }

    pub fn get_code(&mut self, address: Address) -> Bytes {
        let (code, _) = self.evm.context.code(address).unwrap();
        code
    }

    pub fn get_account(&mut self, address: Address) -> Result<Option<AccountInfo>> {
        info!("🟢  [simulator] get_account for {:?}", address);
        self.evm.db_mut().basic(address).map_err(|e| anyhow!("Basic error: {e:?}"))
    }

    pub fn set_token_balance(&mut self, token_address: Address, to: Address, slot: U256, amount: U256) -> Result<()> {
        info!(" set_token_balance token_address: {:?}, to: {:?}, slot: {:?}, amount: {:?}", token_address, to, slot, amount);

        self.insert_mapping_storage_slot(token_address, to, slot, amount);

        Ok(())
    }

    pub fn insert_mapping_storage_slot(&mut self, contract: Address, slot_address: Address, slot: U256, value: U256) -> Result<()> {
        let hashed_balance_slot = keccak256((slot_address, slot).abi_encode());
        info!("🟢 insert_mapping_storage_slot hashed_balance_slot: {:?}", hashed_balance_slot);

        self.insert_account_storage(contract, hashed_balance_slot.into(), value);

        Ok(())
    }

    pub fn get_token_balance(&mut self, token_address: Address, owner: Address) -> Result<U256> {
        debug!("🟢 get_token_balance token_address: {:?}, owner: {:?}", token_address, owner);
        let call_data = abi::erc20::IERC20::balanceOfCall { account: owner }.abi_encode();
        let call_data = Bytes::from(call_data);

        let tx = Tx {
            caller: caller_address(), // owner is not used for: if owner is contract will cause error Transaction(RejectCallerWithCode) // TODO optimize
            transact_to: token_address,
            data: call_data,
            value: U256::ZERO,
            gas_limit: U256::from(DEFAULT_GAS_LIMIT),
            gas_price: U256::ZERO,
            gas_priority_fee: None,
        };
        let value = self.call_static(tx)?;

        info!("🟢 get_token_balance value.output: {:?}", value.output);

        // Check if value.output is "0x" and skip if true
        if value.output.as_ref() == b"" {
            info!("🟢 get_token_balance ❗Skipping❗ decoding as value.output is 0x");
            return Ok(U256::ZERO); // or handle it as needed
        }

        let out = abi::erc20::IERC20::balanceOfCall::abi_decode_returns(&value.output, false)?;
        Ok(out._0)
    }

    pub fn get_v2_pair_reserves(&mut self, pair_address: Address) -> Result<(U256, U256)> {
        let call_data = abi::uniswap2::IUniswapV2Pair::getReservesCall {}.abi_encode();
        let call_data = Bytes::from(call_data);

        let tx = Tx {
            caller: caller_address(),
            transact_to: pair_address,
            data: call_data,
            value: U256::ZERO,
            gas_limit: U256::from(DEFAULT_GAS_LIMIT),
            gas_price: U256::ZERO,
            gas_priority_fee: None,
        };

        let out = self.call_static(tx).map(|res| {
            let reserve_info = abi::uniswap2::IUniswapV2Pair::getReservesCall::abi_decode_returns(&res.output, false).unwrap();
            reserve_info
        })?;

        Ok((U256::from(out.reserve0), U256::from(out.reserve1)))
    }

    // native token is bsc for bsc chain
    // token0: BSD-USD / token1: WBNB
    // TODO 仅使用v2不太准确，待优化
    pub fn convert_usdt_to_native(&mut self, amount: U256) -> Result<U256> {
        let conversion_pair = Address::from_str("0x16b9a82891338f9ba80e2d6970fdda79d1eb0dae").unwrap();

        let reserves = self.get_v2_pair_reserves(conversion_pair)?;
        let (reserve_in, reserve_out) = (reserves.0, reserves.1);
        let weth_out = get_v2_amount_out(amount, reserve_in, reserve_out);
        Ok(weth_out)
    }

    pub fn convert_usdc_to_native(&mut self, amount: U256) -> Result<U256> {
        // TODO implementation
        todo!();

        Ok(U256::ZERO)
    }

    // =======================================================================

    pub fn call(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, true)
    }

    pub fn call_static(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, false)
    }
    // TODO 明确发送 1559 tx
    fn _call(&mut self, tx: Tx, commit: bool) -> Result<TxResult> {
        // block
        // evm.block_mut().number = U256::from(self.block_number + 1);

        if commit {
            debug!("⭕🚀 [simulator] start _call{:?} commit: {:?}", tx, commit);
        } else {
            debug!("⭕🔍 [simulator] start _call{:?} commit: {:?}", tx, commit);
        }

        // tx
        self.evm.tx_mut().caller = tx.caller.into();
        self.evm.tx_mut().transact_to = TransactTo::Call(tx.transact_to.into());
        self.evm.tx_mut().data = tx.data;
        self.evm.tx_mut().value = tx.value.into();
        self.evm.tx_mut().gas_price = tx.gas_price;
        self.evm.tx_mut().gas_limit = tx.gas_limit.try_into().unwrap_or(DEFAULT_GAS_LIMIT); // DEFAULT_GAS_LIMIT值过大可能导致提示余额不足以支付gas
        self.evm.tx_mut().gas_priority_fee = tx.gas_priority_fee;

        // Disable some checks for easier testing

        //暂关闭。实际测试 balance不够时，eth余额会出现误差。user1总共1eth转给用户b1eth，结果余额还有0.049979eth
        // self.evm.cfg_mut().disable_balance_check = true; //ignore: balance should be enough for gas etc

        // 不禁用，当gasfee< base fee时，会触发：EVM call failed: Transaction(GasPriceLessThanBasefee)
        // 禁用后，当price< base fee时，会按照传递的gas price执行
        self.evm.cfg_mut().disable_base_fee = true; // ignore:gas price should not less than base fee

        self.evm.cfg_mut().disable_block_gas_limit = true;

        let result;

        if commit {
            result = match self.evm.transact_commit() {
                Ok(result) => result,
                Err(e) => return Err(anyhow!("EVM call failed: {:?}", e)),
            };
            debug!("⭕🚀✅ [simulator] result {:?}", result);
        } else {
            let ref_tx = self.evm.transact().map_err(|e| anyhow!("EVM staticcall failed: {:?}", e))?;
            result = ref_tx.result;
            debug!("⭕🔍👌 [simulator] result {:?}", result);
        }
        // TODO check result
        let output = match result {
            ExecutionResult::Success { gas_used, gas_refunded, output, .. } => match output {
                Output::Call(o) => TxResult { output: o, logs: None, gas_used, gas_refunded },
                Output::Create(o, _) => TxResult { output: o, logs: None, gas_used, gas_refunded },
            },
            ExecutionResult::Revert { gas_used, output } => {
                error!("⭕❌ _call EVM REVERT: {:?} / Gas used: {:?}", output, gas_used);

                return Err(anyhow!("EVM REVERT: {:?} / Gas used: {:?}", output, gas_used));
            }
            ExecutionResult::Halt { reason, .. } => return Err(anyhow!("EVM HALT: {:?}", reason)),
        };
        // info!("⭕ [simulator] output {:?}", output);

        Ok(output)
    }

    pub fn get_contract_owner(&mut self, bot: Address) -> Result<Address> {
        sol! {
            function owner(
            ) external view returns (address);
        }
        let call_data = ownerCall {}.abi_encode();
        let call_data = Bytes::from(call_data);

        let tx = Tx {
            caller: caller_address(),
            transact_to: bot,
            data: call_data,
            value: U256::ZERO,
            gas_limit: U256::from(50000000),
            gas_price: U256::ZERO,
            gas_priority_fee: None,
        };
        let out = self.call_static(tx).map(|res| {
            let owner_info = ownerCall::abi_decode_returns(&res.output, false).unwrap();
            owner_info
        })?;
        Ok(out._0)
    }

    pub fn get_balance_slot(&mut self, token_address: Address) -> Result<i32> {
        sol! {
            function balanceOf(
                address token_address
            ) external view returns (uint256);
        }

        let call_data = balanceOfCall { token_address }.abi_encode();
        let call_data = Bytes::from(call_data);

        let tx = Tx {
            caller: caller_address(),
            transact_to: token_address,
            data: call_data,
            value: U256::ZERO,
            gas_limit: U256::from(50000000),
            gas_price: U256::ZERO,
            gas_priority_fee: None,
        };

        self.evm.tx_mut().caller = tx.caller.into();
        self.evm.tx_mut().transact_to = TransactTo::Call(tx.transact_to.into());
        self.evm.tx_mut().data = tx.data;

        let result = self.evm.transact()?;

        let token_acc = result.state.get(&token_address).unwrap();
        let token_touched_storage = token_acc.storage.clone();

        for i in 0..30 {
            let slot_bytes = keccak256((token_address, i).abi_encode());
            let slot: U256 = slot_bytes
                .try_into()
                .map_err(|_| {
                    error!("Failed to convert slot_bytes to U256");
                    U256::ZERO // Return a default value
                }) // Remove the '?' and handle the error explicitly
                .unwrap_or_else(|e| {
                    error!("Error: {:?}", e);
                    U256::ZERO // Return a default value in case of error
                });
            match token_touched_storage.get(&slot) {
                Some(_) => {
                    info!("🟢 get_balance_slot got: {:?}", i);

                    return Ok(i);
                }
                None => {}
            }
        }
        Ok(-1)
    }
    // won't change state
    // pub fn get_access_list(& self, tx: Tx) -> Result<AccessList> {
    //     self.evm
    // }
    // pub fn set_access_list(&mut self, access_list: AccessList) {
    //     self.evm.env.tx.access_list = access_list_to_revm(access_list);
    // }
}

pub fn get_v2_amount_out(amount_in: U256, reserve_in: U256, reserve_out: U256) -> U256 {
    let amount_in_with_fee = amount_in * U256::from(997);
    let numerator = amount_in_with_fee * reserve_out;
    let denominator = (reserve_in * U256::from(1000)) + amount_in_with_fee;
    let amount_out = numerator.checked_div(denominator);
    amount_out.unwrap_or_default()
}
pub fn convert_usdt_to_weth(simulator: &mut Simulator<'_, ()>, amount: U256) -> Result<U256> {
    let conversion_pair = Address::from_str("0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852").unwrap();
    // token0: WETH / token1: USDT
    let reserves = simulator.get_v2_pair_reserves(conversion_pair)?;
    let (reserve_in, reserve_out) = (reserves.1, reserves.0);
    let weth_out = get_v2_amount_out(amount, reserve_in, reserve_out);
    Ok(weth_out)
}

pub fn convert_usdc_to_weth(simulator: &mut Simulator<'_, ()>, amount: U256) -> Result<U256> {
    let conversion_pair = Address::from_str("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc").unwrap();
    // token0: USDC / token1: WETH
    let reserves = simulator.get_v2_pair_reserves(conversion_pair)?;
    let (reserve_in, reserve_out) = (reserves.0, reserves.1);
    let weth_out = get_v2_amount_out(amount, reserve_in, reserve_out);
    Ok(weth_out)
}

mod tests {
    
    


    use super::*;
    use anyhow::Ok;
    use one_common::create_default_wss_provider;

    async fn create_simulator_factory() -> Result<Arc<SimulatorFactory>> {
        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));

        Ok(evm_factory)
    }

    #[test]
    fn test_simulator_get_v2_amount_out() {
        init_logs();

        let amount_in = U256::from(100);
        let reserve_in = U256::from(1000);
        let reserve_out = U256::from(1000);
        let amount_out = get_v2_amount_out(amount_in, reserve_in, reserve_out);
        assert_eq!(amount_out, U256::from(90));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]

    async fn test_simulator_set_eth_balance() -> Result<()> {
        init_logs();

        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        let user = PrivateKeySigner::random().address();

        sim.set_eth_balance(user, parse_ether("1").unwrap());

        let balance = sim.get_eth_balance(user);

        info!("balance {}", balance);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_set_token_balance() -> Result<()> {
        init_logs();
        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        let main_currency = Address::from_str("0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c").unwrap();
        let bot_address = Address::from_str("0xF0C5fEF0F2Aa2AeE69d3b6a386D6a8628F28995b").unwrap();

        let balance_slot = 3;

        let hashed_balance_slot = keccak256((bot_address, balance_slot).abi_encode());
        info!("🟢 test_simulator_set_token_balance hashed_balance_slot: {:?}", hashed_balance_slot);

        sim.set_token_balance(main_currency, bot_address, U256::from(balance_slot), U256::from(9999999999999999u64))?;

        let balance1 = sim.get_account_storage(main_currency, hashed_balance_slot.into())?;
        info!("🟢 test_simulator_set_token_balance [balance1]: {:?}", balance1);

        let balance2 = sim.get_token_balance(main_currency, bot_address)?;
        info!("🟢 test_simulator_set_token_balance [balance2]: {:?}", balance2);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_account() -> Result<()> {
        init_logs();
        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        let main_currency = Address::from_str("0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c").unwrap();
        let basic = sim.get_account(main_currency);
        info!("🟢 test_get_account get_account: {:?}", basic);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_token_balance_remote() -> Result<()> {
        init_logs();

        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        let main_currency = Address::from_str("0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c").unwrap();
        let holder = Address::from_str("0xc736cA3d9b1E90Af4230BD8F9626528B3D4e0Ee0").unwrap();
        let balance = sim.get_token_balance(main_currency, holder);

        info!("🟢 balance: {:?}", balance);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_transfer_eth() -> Result<()> {
        init_logs();

        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        sim.set_base_fee(U256::from(2_000_000_000u64));

        let base_fee = sim.get_base_fee();
        info!("🟢 base_fee: {:?}", base_fee);

        let user = Address::from_str("0x1000000000000000000000000000000000000000").unwrap();
        let to = Address::from_str("0x2000000000000000000000000000000000000000").unwrap();

        sim.set_eth_balance(user, parse_ether("1").unwrap());

        let balance_start = sim.get_eth_balance(user);

        info!("🟢 balance user before transfer {:?}", balance_start);

        let tx = Tx {
            caller: user, //if owner is contract will cause error Transaction(RejectCallerWithCode) // TODO optimize
            transact_to: to,
            data: Bytes::default(),
            value: U256::from(500_000_000_000_000_000u64), // 0.5 ETH in wei
            gas_limit: U256::from(21000),                  // Standard gas limit for a transfer
            gas_price: U256::from(3_000_000_000u64),       // 3 Gwei
            gas_priority_fee: None,
        };

        let result = sim.call(tx)?;

        info!("🟢🟢 test_transfer_eth result {:?}", result);

        let balance_after: alloy_primitives::Uint<256, 4> = sim.get_eth_balance(user);
        let balance_to = sim.get_eth_balance(to);

        info!("🟢  balance user after transfer {:?}", balance_after);
        info!("🟢  balance to after transfer {:?}", balance_to);

        info!("🟢  gas_used: {:?}, gas cost: {:?}", result.gas_used, balance_start - U256::from(500_000_000_000_000_000u64) - balance_after);

        // user 1000000000000000000->500000000000000000 + 499979000000000000

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_transfer_erc20() -> Result<()> {
        init_logs();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_simulator_get_balance_slot() -> Result<()> {
        init_logs();

        let sim_factory = create_simulator_factory().await?;
        let mut sim = sim_factory.new_fork_simulator(false);

        let main_currency = Address::from_str("0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c").unwrap();
        let balance_slot = sim.get_balance_slot(main_currency)?;
        info!("🟢 get_balance_slot [balance_slot]: {:?}", balance_slot);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_v2_pair_reserves() -> Result<()> {
        init_logs();

        let wbnb_usdt_address = "0x16b9a82891338f9ba80e2d6970fdda79d1eb0dae";
        let pair_address = Address::from_str(wbnb_usdt_address).unwrap();

        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));

        let mut evm = evm_factory.new_fork_simulator(false);

        let (reserve0, reserve1) = evm.get_v2_pair_reserves(pair_address).unwrap();
        info!("reserve0, reserve1 {},{}", reserve0, reserve1);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_simulator_concurrency() -> Result<()> {
        init_logs();

        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));

        // let eth_100 = parse_ether("100")?;

        // Create 10 users and store them in the users vector
        let mut users = Vec::new();
        for i in 0..10 {
            let user = PrivateKeySigner::random().address();
            evm_factory.init_eoa_account(user, parse_ether((i + 1).to_string().as_str())?)?;
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
                    let balance = evm.get_eth_balance(user);
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_convert_usdt_to_native() -> Result<()> {
        init_logs();

        

        let provider = create_default_wss_provider().await?;
        let block_number = provider.get_block_number().await?;

        let provider = create_default_wss_provider().await.unwrap();
        let block_number = provider.get_block_number().await.unwrap();
        let cache_db = CacheDB::new(EmptyDB::default());

        let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, block_number));
        let mut evm = evm_factory.new_fork_simulator(false);

        let out = convert_usdt_to_weth(&mut evm, U256::from(100_000_000)).unwrap();
        info!("100usdt in, out weth:{}", out);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_access_list_001() -> Result<()> {
        init_logs();
        let sim_factory = create_simulator_factory().await?;

        let mut sim = sim_factory.new_fork_simulator_with_inspector();

        let result = sim.evm.transact().unwrap();

        info!("result {:?}", result.result);

        // let access_list = evm.get

        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tx_from_transaction() -> Result<()> {
        init_logs();

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();


        let tx_req = Tx::from_transaction(pending_tx);

        info!("tx_req {:?}", tx_req);

        Ok(())

    }
}
