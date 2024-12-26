use anyhow::Result;
use serde::de;
use std::{collections::HashMap, fs::OpenOptions, path::Path, str::FromStr, sync::Arc};

use alloy::{
    primitives::{address, keccak256, Address, Bytes, FixedBytes, U128, U256, U64, U8},
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    pubsub::PubSubFrontend,
    rpc::types::eth::{
        state::{AccountOverride, StateOverride},
        transaction::{TransactionInput, TransactionRequest},
        Block, Log, Transaction,
    },
    // errors::Error,
};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, sol_data::String as SolString, SolCall, SolStruct, SolType, SolValue};

use crate::abi;
use crate::common::bytecode;

// Interface of the ITokenInfo
sol! {
  #[derive(Debug, PartialEq, Eq)]
  #[sol(rpc, all_derives)]
  contract ITokenInfo {
    function getTokenInfo(address) external returns (string,string,uint8,uint256);
  }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub id: i64,
    pub address: Address,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub pool_ids: Vec<i64>, // refers to the "id" field of Pool struct
}

// for eth_call response
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub address: Address,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

pub async fn get_token_info(provider: Arc<RootProvider<PubSubFrontend>>, block_number: U64, token_address: Address) -> Result<TokenInfo> {
    info!("get_token_info");

    let mut overwrites = StateOverride::new();

    // override the balance of the owner
    let owner = PrivateKeySigner::random().address();
    let balance = provider.get_balance(owner).await?;
    info!("balance: {:?}", balance);
    let mut acc_override = AccountOverride::default();
    acc_override.balance = Some(U256::MAX);

    overwrites.insert(owner, acc_override);

    // override code of the request_address
    let request_address = PrivateKeySigner::random().address();
    let mut acc_override = AccountOverride::default();
    acc_override.code = Some(bytecode::REQUEST_BYTECODE.clone());

    overwrites.insert(request_address, acc_override);

    let ret_encodes: ITokenInfo::getTokenInfoReturn = ITokenInfo::new(request_address, provider.clone())
        .getTokenInfo(token_address)
        .with_cloned_provider()
        .from(owner)
        .state(overwrites)
        .await?;

    let token_info = TokenInfo { address: token_address, name: ret_encodes._0, symbol: ret_encodes._1, decimals: ret_encodes._2 };

    info!("got token_info  {:?}", token_info);

    Ok(token_info)
}

pub async fn get_token_balance(provider: Arc<RootProvider<PubSubFrontend>>, owner: Address, token_address: Address) -> Result<U256> {
    let token = abi::erc20::IERC20::new(token_address, provider);

    let balance = token.balanceOf(owner).call().await?;

    Ok(balance._0)
}

pub async fn get_token_balances(
    provider: Arc<RootProvider<PubSubFrontend>>,
    owner: Address,
    tokens: &Vec<Address>,
) -> HashMap<Address, U256> {
    // TODO
    let mut token_balances = HashMap::new();

    for token in tokens {
        let balance = get_token_balance(provider.clone(), owner, *token).await.unwrap_or_default();
        token_balances.insert(*token, balance);
    }
    token_balances
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::parse_ether;
    use one_common::{create_default_wss_provider, init_logs};

    use alloy_sol_types::{
        sol_data::{String as SolString, Uint},
        SolType,
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tokens_get_info() -> Result<()> {
        init_logs();
        let provider = create_default_wss_provider().await?;

        get_token_info(
            provider,
            U64::from(0),
            address!("2170Ed0880ac9A755fd29B2688956BD959F933F8"), //bsc::eth
        )
        .await?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_overwrite_weth_balance() -> Result<()> {
        init_logs();
        let provider = create_default_wss_provider().await?;

        let bot_address = address!("29b2F9e909451a6A98Fee9215Ac3648b69598800");
        let weth = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");

        let balance = get_token_balance(provider.clone(), bot_address, weth).await?;
        info!(" bot weth balance: {:?}", balance);

        // overwrite weth balance of bot
        let mut overwrites = StateOverride::new();
        let mut acc_override = AccountOverride::default();
        let hashed_balance_slot = keccak256((bot_address, 3).abi_encode());
        let balance_bytes: FixedBytes<32> = parse_ether("100").unwrap().into();
        acc_override.state_diff = Some(HashMap::from([(hashed_balance_slot.into(), balance_bytes)]));
        overwrites.insert(weth, acc_override);

        // get the balance after overwrite
        let erc20_token = abi::erc20::IERC20::new(weth, provider.clone());
        let balance = erc20_token.balanceOf(bot_address).state(overwrites.clone()).call().await?;
        info!(" bot weth when overwrite balance: {:?}", balance._0);

        Ok(())
    }
}
