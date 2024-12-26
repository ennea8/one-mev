use alloy_provider::WalletProvider;
use anyhow::Result;
use std::sync::Arc;

use alloy::{
    consensus::{TxEip1559, TxEnvelope, TypedTransaction},
    eips::{BlockId, BlockNumberOrTag},
    network::{
        eip2718::Encodable2718, Ethereum, EthereumWallet, NetworkWallet,
        TransactionBuilder,
    },
    primitives::{
        Address, Bytes, TxHash, TxKind, U256,
    },
    providers::{
        fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller},
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    pubsub::PubSubFrontend,
    rpc::types::{
        eth::{AccessList, Transaction},
        mev::{EthCallBundle, EthSendBundle},
    },
    signers::Signer,
    transports::http::{Client, Http},
};

use crate::common::config::get_global_config;
use one_flashbots::{BundleSigner, Endpoints, EthMevProviderExt};

use crate::utils::log_info_to_file;
use alloy::rpc::json_rpc::RpcError;
use alloy::rpc::types::mev::EthCallBundleResponse;
use alloy::transports::TransportErrorKind;
use serde_json;

use crate::arbitrage::types::NewBlock;

// use reqwest;
#[derive(Debug, Clone, serde::Serialize)]
pub struct SandoBundle {
    pub frontrun_tx: Option<TypedTransaction>, // optional, if None, means no frontrun
    pub victim_txs: Vec<Transaction>,
    pub backrun_tx: TypedTransaction,
}

pub struct Executor {
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    pub searcher_signer: EthereumWallet,

    pub bundle_signer: EthereumWallet,
    pub bot_address: Address,
    pub endpoints: Endpoints,
    pub client: FillProvider<
        JoinFill<
            JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>,
            WalletFiller<EthereumWallet>,
        >,
        RootProvider<Http<Client>>,
        Http<Client>,
        Ethereum,
    >,
    chain_id: u64, // cache for easy to access in all methods
}

#[derive(Debug, Clone, serde::Serialize)]
struct SubmitionLogData {
    pub topic: String,
    pub block_number: u64,
    pub sender: Address,
    pub sando_bundle: SandoBundle,
    pub bundle_hash: Vec<TxHash>,
    pub bundle_errors: Vec<String>,
    pub new_block: NewBlock,
    pub backrun_tx: Option<TxEnvelope>,
}

impl Executor {
    pub fn new(provider: Arc<RootProvider<PubSubFrontend>>) -> Self {
        let config = get_global_config();
        let bot_address = config.bot_address;

        let chain_id = config.chain_id;

        let searcher_signer =
            EthereumWallet::new(config.searcher_signer.clone().with_chain_id(Some(chain_id)));
        let bundle_signer =
            EthereumWallet::new(config.bundle_signer.clone().with_chain_id(Some(chain_id)));

        let client = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(searcher_signer.clone())
            .on_http(config.http_rpc.clone());

        // TODO add more endpoints
        let endpoints: Endpoints = client
            .endpoints_builder()
            .gmbit()
            .flashbots(BundleSigner::flashbots(config.bundle_signer.clone()))
            .beaverbuild()
            .titan(BundleSigner::flashbots(config.bundle_signer.clone()))
            .rsync()
            .lokibuilder()
            .build();

        Self {
            provider,
            searcher_signer,
            bundle_signer,
            bot_address,
            endpoints,
            client,
            chain_id,
        }
    }

    pub async fn _common_fields(&self, block_number: Option<u64>) -> Result<(Address, u64, u64)> {
        let block_id = match block_number {
            Some(num) => BlockId::Number(num.into()),
            None => BlockId::latest(),
        };

        let nonce = self
            .provider
            .get_transaction_count(self.searcher_signer.default_signer().address())
            .block_id(block_id)
            .await?;
        Ok((self.searcher_signer.default_signer().address(), nonce, 1u64)) // TODO check hardcode chain_id
    }

    pub async fn create_sando_bundle_backrun(
        &self,
        victim_txs: Vec<Transaction>,
        // front_calldata: Bytes,
        back_calldata: Bytes,
        // front_access_list: AccessList,
        back_access_list: AccessList,
        // front_gas_limit: u64,
        back_gas_limit: u64,
        //base_fee: U256,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
        block_number: Option<u64>,
    ) -> Result<SandoBundle> {
        let common = self._common_fields(block_number).await?;
        let to = TxKind::Call(self.bot_address);
        let nonce = common.1;
        let backrun_tx = TypedTransaction::Eip1559(TxEip1559 {
            chain_id: self.chain_id,
            nonce,
            to: to.clone(),
            value: U256::from(0_u64),
            input: back_calldata,
            max_fee_per_gas: u128::try_from(max_fee_per_gas).unwrap(),
            max_priority_fee_per_gas: u128::try_from(max_priority_fee_per_gas).unwrap(), // TODO check
            gas_limit: u128::from(back_gas_limit),
            access_list: back_access_list,
        });

        Ok(SandoBundle {
            frontrun_tx: None,
            victim_txs,
            backrun_tx,
        })
    }

    pub async fn simulate_bundle(
        &self,
        sando_bundle: SandoBundle,
        // block_number: u64,
        new_block: NewBlock,
    ) -> Result<Vec<Result<EthCallBundleResponse, RpcError<TransportErrorKind>>>> {
        let mut bundle: EthCallBundle = EthCallBundle::default();
        bundle.block_number = new_block.block_number + 1;
        bundle.state_block_number = BlockNumberOrTag::Number(new_block.block_number); // TODO  test

        let sender = NetworkWallet::<Ethereum>::default_signer_address(&self.searcher_signer);

        // set bundle txs
        for victim_tx in &sando_bundle.victim_txs {
            let tx = TxEnvelope::try_from(victim_tx.clone())
                .unwrap()
                .encoded_2718()
                .into();
            bundle.txs.push(tx);
        }

        let backrun_tx = NetworkWallet::<Ethereum>::sign_transaction_from(
            &self.searcher_signer,
            sender,
            sando_bundle.backrun_tx,
        )
        .await?;
        info!("backrun_tx: {:?}", backrun_tx);
        bundle.txs.push(backrun_tx.encoded_2718().into());

        // send bundle  ////////////////////
        let responses = self.client.call_eth_bundle(bundle, &self.endpoints).await;

        for response in &responses {
            match response {
                Ok(x) => info!("call_eth_bundle response: {x:#?}"),
                Err(e) => error!("call_eth_bundle response error: {e:?}"),
            }
        }

        Ok(responses)
    }

    pub async fn broadcast_bundle(
        &self,
        sando_bundle: SandoBundle,
        new_block: NewBlock,
    ) -> Result<()> {
        let target_block_number = new_block.block_number + 1;
        let mut bundle = EthSendBundle::default();
        bundle.block_number = target_block_number;

        let sender = NetworkWallet::<Ethereum>::default_signer_address(&self.searcher_signer);

        // set bundle txs
        info!(
            "🟢🟢🟢🟢🟢 broadcast_bundle: sando_bundle: {:?}, block_number: {}, sender: {:?}",
            sando_bundle, target_block_number, sender
        );

        let mut submition_log_data = SubmitionLogData {
            topic: "🟢🟢🟢broadcast_bundle".to_string(),
            block_number: target_block_number,
            sender,
            sando_bundle: sando_bundle.clone(),
            bundle_hash: vec![],
            bundle_errors: vec![],
            new_block: new_block,
            backrun_tx: None,
        };

        for victim_tx in &sando_bundle.victim_txs {
            let tx = TxEnvelope::try_from(victim_tx.clone())
                .unwrap()
                .encoded_2718()
                .into();
            bundle.txs.push(tx);
        }

        // TODO check params
        let backrun_tx = NetworkWallet::<Ethereum>::sign_transaction_from(
            &self.searcher_signer,
            sender,
            sando_bundle.backrun_tx,
        )
        .await?;

        // info!("backrun_tx info: {:?}", backrun_tx);

        submition_log_data.backrun_tx = Some(backrun_tx.clone()); // add backrun_tx to log
        bundle.txs.push(backrun_tx.encoded_2718().into());

        // send bundle  ////////////////////
        let responses = self.client.send_eth_bundle(bundle, &self.endpoints).await;
        for response in responses {
            match response {
                Ok(x) => {
                    info!("send_eth_bundle response: {x:#?}");
                    submition_log_data.bundle_hash.push(x.bundle_hash);
                }
                Err(e) => {
                    warn!("send_eth_bundle response error: {e:?}");
                    submition_log_data.bundle_errors.push(e.to_string());
                }
            }
        }

        let log_data_str = format!(
            "\n{}\n",
            serde_json::to_string(&submition_log_data)
                .expect("Failed to serialize SubmitionLogData")
        );
        log_info_to_file(&log_data_str).await;

        Ok(())
    }
}

mod tests {
    

    

    
    
    

    

    #[tokio::test]
    async fn test_execute_call_eth_bundle() -> Result<()> {
        init_logs();

        dotenv::from_filename(".env.eth.arbitrage").ok();
        let eth_rpc = std::env::var("HTTP_RPC").unwrap();
        let eth_wss = std::env::var("WSS_RPC").unwrap();

        info!("eth_rpc: {}", eth_rpc);
        info!("eth_wss: {}", eth_wss);

        let signer = PrivateKeySigner::random();
        let wallet = EthereumWallet::new(signer.clone());

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet.clone())
            .on_http(eth_rpc.parse().unwrap());

        // let endpoints = provider.endpoints_builder().flashbots(BundleSigner::flashbots(signer.clone())).build();

        let endpoints: Endpoints = provider
            .endpoints_builder()
            .beaverbuild()
            .titan(BundleSigner::flashbots(signer.clone()))
            .flashbots(BundleSigner::flashbots(signer.clone()))
            .rsync()
            .build();

        let block_number = 20247245;

        let x = provider
        .call_eth_bundle(
            EthCallBundle {
                // tx 0x0722b12f3f46877a5251ecce105263ccf9f5390f9fab5ecc51e4858705fd8667
                txs: vec![hex!("02f876018204ed843b9aca0085012a05f20082a22794825001ac81d9348f71f2dadd717335ac0ab4a9fe89056a6418b50586000080c001a0e491ff34326cd113b9a1a34f2f82f57727d70dc78577a97ae54dd3a2b43b8583a06c956d5b1dae0514360d56186870c5d50771fc4b204931a5ace7e19baa7f0a86").into()],
                block_number,
                state_block_number: BlockNumberOrTag::Number(block_number - 1),
                timestamp: None,
                gas_limit: None,
                difficulty: None,
                base_fee: None,
            },
            &endpoints,
        )
        .await;

        println!("{x:#?}");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_simulate_bundle() -> Result<()> {
        use crate::arbitrage::config::constants::ethereum::weth_addr;
        use crate::arbitrage::config::constants::{
            OWNER_ADDRESS, REVM_ONE_ADDRESS, REVM_ONE_SIMULATOR_ADDRESS,
        };
        use crate::arbitrage::simulation::create_simulator_factory;
        use crate::common::bytecode::ONE_BYTECODE;
        use crate::common::bytecode::ONE_SIMULATOR_BYTECODE;
        use one_common::create_default_wss_provider;
        use revm::primitives::{
                Bytecode, Bytes,
            };

        std::env::set_var("KEYSTORE_PATH", "../.keystore");
        init_logs();
        init_global_config();

        let provider = create_default_wss_provider().await?;
        let executor = Executor::new(provider.clone());

        let WETH = Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap();
        let EBULL = Address::from_str("0x71297312753EA7A2570a5a3278eD70D9a75F4f44").unwrap();
        let EBULL_V2 = Address::from_str("0x1f4eF1F8441Caac34F58fb0CBa813dD2B09FEC63").unwrap();
        let EBULL_V3 = Address::from_str("0xa9405016F8158d87f5659b63df170c03B8396450").unwrap();

        let block_number = 20_975_913 - 1;
        let evm_factory = create_simulator_factory(block_number).await?; // before onebot contract deploy

        evm_factory.set_eth_balance(*OWNER_ADDRESS, U256::from(parse_ether("100").unwrap()));
        // one contract
        evm_factory.deploy(*REVM_ONE_ADDRESS, Bytecode::new_raw(ONE_BYTECODE.clone())); // use online code
        evm_factory.set_token_balance(
            weth_addr(),
            *REVM_ONE_ADDRESS,
            U256::from(3),
            U256::from(parse_ether("100").unwrap()),
        );
        // one_simulator contract
        evm_factory.deploy(
            *REVM_ONE_SIMULATOR_ADDRESS,
            Bytecode::new_raw(ONE_SIMULATOR_BYTECODE.clone()),
        );
        evm_factory.set_token_balance(
            weth_addr(),
            *REVM_ONE_SIMULATOR_ADDRESS,
            U256::from(3),
            U256::from(parse_ether("100").unwrap()),
        );

        let mut sim = evm_factory.new_fork_simulator(false);

        let tx_json = r#"{"hash":"0xbd75d61cf462f0c83bb57ebfda26d57fa0485dce7eb9428dafd2516549a756b8","nonce":"0x10a0","blockHash":"0x077709bc7eb7f9b73d6b30858568809ce100106d0703a05f14685c367534b530","blockNumber":"0x1401129","transactionIndex":"0x1","from":"0xefa9268490bb76d6b17793905473fefc03b5c824","to":"0x3328f7f4a1d1c57c35df56bbf0c9dcafca309c49","value":"0x0","gasPrice":"0x8392ac507","gas":"0x729ab","maxFeePerGas":"0x96330b707","maxPriorityFeePerGas":"0x60db88400","input":"0x75713a0800000000000000000000000071297312753ea7a2570a5a3278ed70d9a75f4f44000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488d0000000000000000000000001f4ef1f8441caac34f58fb0cba813dd2b09fec6300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c465cc50b7d5a29b9308968f870a4b242a8e1873000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000670f493f0000000000000000000000000000000000000000000000000000000000000000","r":"0x5dcf975a8c510c0b247e810f0e9515323b0a6fa8fcaf0e55164b67d08403ef5a","s":"0x4584e15f8a0fba7df7ccd963c212fb760f54f9acb9c2e4b3bcde27e3d9a0c345","v":"0x1","yParity":"0x1","chainId":"0x1","accessList":[],"type":"0x2"}"#;
        let pending_tx: Transaction = serde_json::from_str(&tx_json).unwrap();

        let mut pathArray = vec![];
        pathArray.push(IOne::SwapParams {
            protocol: 2,
            handler: EBULL_V2,
            tokenIn: WETH,
            tokenOut: EBULL,
            fee: 0,
            amount: parse_ether("0.5").unwrap(),
        });
        pathArray.push(IOne::SwapParams {
            protocol: 3,
            handler: EBULL_V3,
            tokenIn: EBULL,
            tokenOut: WETH,
            fee: 10_000,
            amount: U256::ZERO,
        });

        // get amountOut for v2
        let amount_out = sim.getAmountOut(pathArray[0].clone())?;
        info!("amount_out: {amount_out:#?}");

        pathArray[0].amount = amount_out;
        let pathArrayData = Bytes::from(<Vec<IOne::SwapParams>>::abi_encode(&pathArray));
        let calldata_arbitrage = Bytes::from(
            IOne::arbitrageCall {
                pathArrayData,
                baseToken: pathArray[0].tokenIn,
                requireProfit: false,
            }
            .abi_encode(),
        );

        info!("calldata_arbitrage: {calldata_arbitrage:#?}");

        let sando_data = executor
            .create_sando_bundle_backrun(
                vec![pending_tx],
                calldata_arbitrage,
                alloy::rpc::types::eth::AccessList::default(),
                5_000_000,
                U256::from(1_000_000_000u64),
                U256::from(9_596_116_417u64),
                Some(block_number),
            )
            .await?;

        info!("sando_data: {sando_data:#?}");

        let new_block = NewBlock {
            block_number,
            base_fee: U256::from(1_000_000_000u64),
            next_base_fee: U256::from(1_000_000_000u64),
        };

        let result = executor.simulate_bundle(sando_data, new_block).await?;

        info!("result: {result:#?}");

        Ok(())
    }

    #[tokio::test]
    async fn test_broadcast_bundle() -> Result<()> {
        init_logs();

        Ok(())
    }
}
