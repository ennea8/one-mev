#[macro_use]
extern crate tracing;

use anyhow::{anyhow, Result};
use std::sync::Arc;

// alloy
use alloy::{
    network::{Ethereum, EthereumWallet, NetworkWallet, TransactionBuilder},
    primitives::{utils::parse_ether, U256},
    providers::Provider,
    rpc::types::eth::{AccessList, Transaction},
    signers::Signer,
    sol_types::SolCall,
};
//revm
use revm::{
    db::{CacheDB, EmptyDB},
    primitives::{
        address,
        Bytecode, // AccessList, AccessListItem,
    }, Inspector,
};

// use one_evm::types::{Tx, TxResult};



use one_common::create_default_wss_provider;
use one_common::init_logs;

use onepiece::abi::IOne;
use onepiece::simulation::simulator::{SimulatorFactory, Tx, VictimTx};

use onepiece::arbitrage::config::constants::ethereum::weth_addr;
use onepiece::arbitrage::config::constants::{OWNER_ADDRESS, REVM_ONE_ADDRESS, REVM_ONE_SIMULATOR_ADDRESS};
use onepiece::arbitrage::execution::Executor;
use onepiece::arbitrage::types::NewBlock;
use onepiece::common::bytecode::{ONE_BYTECODE, ONE_SIMULATOR_BYTECODE};
use onepiece::common::config::{get_global_config, init_global_config};
use onepiece::utils::{calculate_next_block_base_fee, calculate_next_block_base_fee2};

/**
"block_number":21228683,
v3-v3

weth-usdc

*/

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_arbi001_with_submit_bundle() -> Result<()> {
    std::env::set_var("KEYSTORE_PATH", "../.keystore");

    dotenv::from_filename(".env.eth.arbitrage").ok();
    init_logs();
    init_global_config();

    let config = get_global_config();
    let searcher_signer = EthereumWallet::new(config.searcher_signer.clone().with_chain_id(Some(1u64)));
    let bundle_signer = EthereumWallet::new(config.bundle_signer.clone().with_chain_id(Some(1u64)));
    let sender = NetworkWallet::<Ethereum>::default_signer_address(&searcher_signer);

    info!("sender: {:?}", sender);

    // block: 21228683: 9.827293126 Gwei
    // block: 21228684: 8.827883949 Gwei
    // block: 21228685: 9.376060524 Gwei
    let base_block_number = 21228683;
    let base_fee = U256::from(9827293126u64); // from onchain
    let next_base_fee = calculate_next_block_base_fee(U256::from(2796324u64), U256::from(30000000u64), base_fee);
    let next_base_fee2 = calculate_next_block_base_fee2(U256::from(2796324u64), U256::from(30000000u64), base_fee);

    info!("next_base_fee: {:?}, next_base_fee2: {:?}", next_base_fee, next_base_fee2);

    // evm_factory
    let provider = create_default_wss_provider().await.unwrap();
    let cache_db = CacheDB::new(EmptyDB::default());
    let evm_factory = Arc::new(SimulatorFactory::new(provider.clone(), cache_db, base_block_number));

    // let evm_factory = create_simulator_factory(base_block_number).await?; // before onebot contract deploy

    evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));
    // one contract
    evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone())); // use online code
    evm_factory.set_token_balance(weth_addr(), *REVM_ONE_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

    // one_simulator contract
    evm_factory.deploy(*REVM_ONE_SIMULATOR_ADDRESS, Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()));
    evm_factory.set_token_balance(weth_addr(), *REVM_ONE_SIMULATOR_ADDRESS, U256::from(3), U256::from(parse_ether("100").unwrap()));

    let mut sim = evm_factory.new_fork_simulator(false);

    let pending_tx_json = r#"
    {"hash":"0x529616367e165f25421c30d0483291982a2985e37bed1aa082a905452d163cb5","nonce":"0x283","blockHash":null,"blockNumber":null,"transactionIndex":null,"from":"0x63ae8eb0264f1d05dc0c969bb0adaa311dd9a59e","to":"0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad","value":"0x0","gasPrice":"0x2e4c3487b","gas":"0x60d3f","maxFeePerGas":"0x2e4c3487b","maxPriorityFeePerGas":"0x77359400","input":"0x3593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000673dd059000000000000000000000000000000000000000000000000000000000000000400080604000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000003600000000000000000000000000000000000000000000000000000000000000120000000000000000000000000bcbb5a8f286a3b9fef6b5a34d706ae64a854a225000000000000000000000000000000000000000000000000000000746a528800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000428ccd897ca6160ed76755383b201c1948394328c7002710a0b86991c6218b36c1d19d4a2e9eb0ce3606eb480001f4c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000028000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc200000000000000000000000045bf4c984c96c0629ba584ced7464d84dd1ee270000000000000000000000000000000000000000000000000000000000000006000000000000000000000000045bf4c984c96c0629ba584ced7464d84dd1ee270000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c0000000000000000000000000000000000000000000000000000000000000019000000000000000000000000000000000000000000000000000000000000006000000000000000000000000045bf4c984c96c0629ba584ced7464d84dd1ee27000000000000000000000000063ae8eb0264f1d05dc0c969bb0adaa311dd9a59e0000000000000000000000000000000000000000000001373028e6fbfb5a42c60c","r":"0x2e039b2c67f116351b4a97fb99791b165b18ea7b7e0c97fa8d64fe0753f6e0e0","s":"0x1f14da0452be8acba16e2869d910252b0d24f25159dbeee7fc71de1dafd0871e","v":"0x0","yParity":"0x0","chainId":"0x1","accessList":[],"type":"0x2"}
    "#;
    let pending_tx: Transaction = serde_json::from_str(&pending_tx_json).unwrap();

    let tx_req = Tx::from_transaction(pending_tx.clone());
    sim.set_base_fee(base_fee);
    match sim.call(tx_req.clone()) {
        Ok(result) => {
            info!("pending_tx result {:?}", result);
        }
        Err(err) => {
            info!("❌ [simulateArbitrage]run pending_tx error {:?}", err);
        }
    }
    info!("----------------------------begin_back_run----------------------------end");
    // sim.set_base_fee(U256::ZERO);

    let WNATIVE = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
    let OTHER_TOKEN = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    let V3_500 = address!("88e6a0c2ddd26feeb64f039a2c41296fcb3f5640");
    let V3_100 = address!("e0554a476a092703abdb3ef35c80e0d76d32939f");

    // let swap_paths = vec![
    //     (V3_500, WNATIVE, OTHER_TOKEN, 500),
    //     (V3_100, OTHER_TOKEN, WNATIVE, 100),
    // ];

    let mut pathArray = vec![];
    pathArray.push(IOne::SwapParams {
        protocol: 3,
        handler: V3_500,
        tokenIn: WNATIVE,
        tokenOut: OTHER_TOKEN,
        fee: 500,
        amount: U256::ZERO,
    });
    pathArray.push(IOne::SwapParams {
        protocol: 3,
        handler: V3_100,
        tokenIn: OTHER_TOKEN,
        tokenOut: WNATIVE,
        fee: 100,
        amount: U256::ZERO,
    });

    let new_block = NewBlock { block_number: base_block_number, base_fee, next_base_fee };

    let swap_paths = vec![pathArray];
    let victim_tx = VictimTx::from_transaction(pending_tx.clone());

    let arbitrage = match sim.find_profitable_path_and_opt_amount(swap_paths) {
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

    // let (the_path, optimized_in, max_revenue) = arbi_paths.unwrap().get(0).clone().unwrap();
    let the_path = arbitrage.path.clone();
    let optimized_in = arbitrage.optimized_in;
    let max_revenue = arbitrage.max_revenue;

    info!("find_arbi_paths: the_path: {:?}, optimized_in: {:?}, max_revenue: {:?}", the_path, optimized_in, max_revenue);

    let (revenue, realistic_back_gas_limit, gas_cost, gas_used, calldata) =
        sim.simulateArbitrageMulti(vec![arbitrage], new_block.next_base_fee).unwrap();

    info!("simulateArbitrageMulti: revenue: {:?}, realistic_back_gas_limit: {:?}, gas_cost: {:?}, gas_used: {:?}, calldata: {:?}", revenue, realistic_back_gas_limit, gas_cost, gas_used, calldata);

    let base_fee = new_block.next_base_fee;
    let revenue_u256: U256 = revenue.into_raw().try_into().unwrap();
    let bribe_pct = U256::from(10000);
    let bribe_amount = (revenue_u256 * bribe_pct) / U256::from(10000);
    let realistic_back_gas_limit = (gas_used * 105) / 100;
    let max_priority_fee_per_gas = bribe_amount / U256::from(realistic_back_gas_limit);
    let max_fee_per_gas = base_fee + max_priority_fee_per_gas;

    let executor = Executor::new(provider.clone());

    let sando_bundle = match executor
        .create_sando_bundle_backrun(
            vec![pending_tx.clone()],
            calldata,
            AccessList::default(),
            realistic_back_gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            None,
        )
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!("❗❌ [create_sando_bundle_backrun] error: {:?}", e);
            return Err(anyhow!("❗❌ [create_sando_bundle_backrun] error: {:?}", e));
        }
    };

    info!("sando_bundle: {:?}", sando_bundle);

    // simulate bundle
    // match executor.simulate_bundle(sando_bundle.clone(), new_block.clone()).await {
    //     Ok(result) => {
    //         info!("🟢🟢🟢 [simulate_bundle] success: {:?}", result);
    //     }
    //     Err(e) => {
    //         warn!("❗❌ [simulate_bundle] error: {:?}", e);
    //     }
    // }

    Ok(())
}
