use alloy_provider::WalletProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;

use alloy::{
    consensus::{SignableTransaction, TxEip1559, TxEnvelope, TypedTransaction},
    eips::{BlockId, BlockNumberOrTag},
    network::{eip2718::Encodable2718, Ethereum, EthereumWallet, Network, NetworkWallet, TransactionBuilder},
    primitives::{utils::parse_ether, Address, BlockNumber, Bytes, TxHash, TxKind, U128, U256, U64},
    providers::{
        fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller},
        Identity, Provider, ProviderBuilder, ReqwestProvider, RootProvider,
    },
    pubsub::PubSubFrontend,
    rpc::types::{
        eth::{AccessList, Block, Log, Transaction, TransactionRequest},
        mev::{EthCallBundle, EthSendBundle},
    },
    signers::{local::PrivateKeySigner, Signer, SignerSync},
    transports::http::{self, Client, Http},
    transports::ws::WsConnect,
};

use crate::abi;
use crate::common::config::get_global_config;
use one_common::create_default_http_provider;
use one_flashbots::{BundleSigner, Endpoints, EthMevProviderExt, MevHttp};

// use reqwest;

#[derive(Debug, Clone)]
pub struct SandoBundle {
    pub frontrun_tx: TypedTransaction,
    pub victim_txs: Vec<Transaction>,
    pub backrun_tx: TypedTransaction,
}

pub struct Executor {
    pub provider: ReqwestProvider,
    // pub abi: Abi,
    // pub client: ,
    pub owner: EthereumWallet,
    pub bundle_signer: EthereumWallet,
    pub bot_address: Address,
    // pub builder_urls: HashMap<String, Url>,
    pub endpoints: Endpoints,

    // 用于发送bundle，会自动签名
    pub client: FillProvider<
        JoinFill<JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>, WalletFiller<EthereumWallet>>,
        RootProvider<Http<Client>>,
        Http<Client>,
        Ethereum,
    >,
}

impl Executor {
    pub fn new() -> Self {
        let config = get_global_config();
        let bot_address = config.bot_address;
        // TODO check chain
        let owner = EthereumWallet::new(config.searcher_signer.clone().with_chain_id(Some(1u64)));
        let bundle_signer = EthereumWallet::new(config.bundle_signer.clone().with_chain_id(Some(1u64)));

        // TODO check optimize? there are two ProviderBuilder. client is enough?  remove？需理解filler的机制
        let provider = ProviderBuilder::new().on_http(config.http_rpc.clone());

        //provider.send_transaction(tx)//

        let client = ProviderBuilder::new().with_recommended_fillers().wallet(owner.clone()).on_http(config.http_rpc.clone());

        let endpoints: Endpoints = client.endpoints_builder().flashbots(BundleSigner::flashbots(config.bundle_signer.clone())).build();

        Self { provider, owner, bundle_signer, bot_address, endpoints, client }
    }
    pub async fn create_sando_bundle(
        &self,
        victim_txs: Vec<Transaction>,
        front_calldata: Bytes,
        back_calldata: Bytes,
        front_access_list: AccessList,
        back_access_list: AccessList,
        front_gas_limit: u64,
        back_gas_limit: u64,
        base_fee: U256, // TODO check
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
    ) -> Result<SandoBundle> {
        let common = self._common_fields().await?;
        let to = TxKind::Call(self.bot_address);
        let front_nonce = common.1;
        let back_nonce = front_nonce + 1u64; // should increase nonce by 1

        let frontrun_tx = TypedTransaction::Eip1559(TxEip1559 {
            chain_id: common.2, // TODO check chain
            nonce: front_nonce,
            to: to.clone(),
            value: U256::from(0_u64),
            input: front_calldata,
            max_fee_per_gas: u128::try_from(base_fee).unwrap(), // frontrun use base_fee, bribe is included in backrun
            max_priority_fee_per_gas: 0u128, // TODO check 为何设置为0 // bribe is included in backrun 
            gas_limit: u128::from(front_gas_limit),
            access_list: front_access_list,
        });

        let backrun_tx = TypedTransaction::Eip1559(TxEip1559 {
            chain_id: common.2, // TODO check chain
            nonce: back_nonce,
            to: to.clone(),
            value: U256::from(0_u64),
            input: back_calldata,
            max_fee_per_gas: u128::try_from(max_fee_per_gas).unwrap(),
            max_priority_fee_per_gas: u128::try_from(max_priority_fee_per_gas).unwrap(), // TODO check
            gas_limit: u128::from(back_gas_limit),
            access_list: back_access_list,
        });

        Ok(SandoBundle { frontrun_tx, victim_txs, backrun_tx })
    }

    // TODO test block_id params
    pub async fn _common_fields(&self) -> Result<(Address, u64, u64)> {
        let nonce = self.provider.get_transaction_count(self.owner.default_signer().address()).block_id(BlockId::latest()).await?;
        Ok((self.owner.default_signer().address(), nonce, 1u64))
    }

    // create EthCallBundle / EthSendBundle
    // create tx array
    pub async fn to_sando_bundle_request(&self, sando_bundle: SandoBundle, block_number: u64, retries: usize) -> Result<EthCallBundle> {
        let sender = NetworkWallet::<Ethereum>::default_signer_address(&self.owner);

        let frontrun_tx = NetworkWallet::<Ethereum>::sign_transaction_from(&self.owner, sender, sando_bundle.frontrun_tx).await?;

        let backrun_tx = NetworkWallet::<Ethereum>::sign_transaction_from(&self.owner, sender, sando_bundle.backrun_tx).await?;

        let mut bundle: EthCallBundle = EthCallBundle::default(); //TODO set params

        bundle.txs.push(frontrun_tx.encoded_2718().into());
        for victim_tx in &sando_bundle.victim_txs {
            let tx = TxEnvelope::try_from(victim_tx.clone()).unwrap().encoded_2718().into();
            bundle.txs.push(tx);
        }
        bundle.txs.push(backrun_tx.encoded_2718().into());

        Ok(bundle)
    }

    // TODO check params
    pub async fn send_sando_bundle_request(&self, bundle: EthCallBundle, block_number: u64) -> Result<()> {
        let x = self.client.call_eth_bundle(bundle, &self.endpoints).await;
        info!("sando_bundle {x:#?}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_sign_transaction() -> Result<()> {
        let user001 = PrivateKeySigner::random();
        let user002 = PrivateKeySigner::random();

        let signer = PrivateKeySigner::random();
        let wallet = EthereumWallet::from(signer);

        let tx = TransactionRequest::default()
            .with_to(user001.address())
            .with_nonce(0)
            .with_chain_id(1u64)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        let tx_envelope = tx.build(&wallet).await?;

        let tx_encoded = tx_envelope.encoded_2718();

        // send to flashbot
        let flashbots_url = "https://rpc.flashbots.net".parse()?;
        let provider = ProviderBuilder::new().on_http(flashbots_url);

        let pending = provider.send_raw_transaction(&tx_encoded).await?.register().await?;

        println!("Sent transaction: {}", pending.tx_hash());

        Ok(())
    }
}
