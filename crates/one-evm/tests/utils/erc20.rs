use alloy::core::sol_types::SolCall;
use alloy::pubsub::PubSubFrontend;
use alloy::{
    primitives::{Address, Bytes, U256},
    providers::RootProvider,
    sol,
};
use std::sync::Arc;

use bigdecimal::BigDecimal;
use std::str::FromStr;

sol! {
  #[sol(rpc)]
  contract ERC20 {
      function balanceOf(address owner) external view returns (uint256 balance);
      function approve(address spender, uint256 amount) external returns (bool);
      function transfer(address recipient, uint256 amount) external returns (bool);
      function transferFrom(address from, address recipient, uint256 amount) external returns (bool);
      function allowance(address owner, address spender) external view returns (uint256);
      function name() external view returns (string memory);
      function symbol() external view returns (string memory);
      function decimals() external view returns (uint8);
      function totalSupply() external view returns (uint256);
      function deposit() external payable;
      function withdraw(uint256 amount) external;
}
}

#[derive(Debug, Clone)]
pub struct ERC20Token {
    pub address: Address,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub total_supply: U256,
}

impl ERC20Token {
    pub async fn new(
        address: Address,
        client: Arc<RootProvider<PubSubFrontend>>,
    ) -> Result<Self, anyhow::Error> {
        let contract = ERC20::new(address, client);
        let symbol = contract.symbol().call().await?._0;
        let name = contract.name().call().await?._0;
        let decimals = contract.decimals().call().await?._0;
        let total_supply = contract.totalSupply().call().await?._0;
        Ok(Self {
            address,
            symbol,
            name,
            decimals,
            total_supply,
        })
    }

    pub async fn balance_of(
        &self,
        owner: Address,
        client: Arc<RootProvider<PubSubFrontend>>,
    ) -> Result<U256, anyhow::Error> {
        let contract = ERC20::new(self.address, client);
        let bal = contract.balanceOf(owner).call().await?;
        Ok(bal.balance)
    }

    pub async fn allowance(
        &self,
        owner: Address,
        spender: Address,
        client: Arc<RootProvider<PubSubFrontend>>,
    ) -> Result<U256, anyhow::Error> {
        let contract = ERC20::new(self.address, client);
        let allowance = contract.allowance(owner, spender).call().await?._0;
        Ok(allowance)
    }

    pub fn encode_balance_of(&self, owner: Address) -> Vec<u8> {
        let contract = ERC20::balanceOfCall { owner };
        contract.abi_encode()
    }

    pub fn encode_approve(&self, spender: Address, amount: U256) -> Vec<u8> {
        let contract = ERC20::approveCall { spender, amount };
        contract.abi_encode()
    }

    pub fn encode_transfer(&self, recipient: Address, amount: U256) -> Vec<u8> {
        let contract = ERC20::transferCall { recipient, amount };
        contract.abi_encode()
    }

    pub fn encode_deposit(&self) -> Vec<u8> {
        let contract = ERC20::depositCall {};
        contract.abi_encode()
    }

    pub fn encode_withdraw(&self, amount: U256) -> Vec<u8> {
        let contract = ERC20::withdrawCall { amount };
        contract.abi_encode()
    }

    pub fn decode_balance_of(&self, bytes: &Bytes) -> Result<U256, anyhow::Error> {
        let balance = ERC20::balanceOfCall::abi_decode_returns(&bytes, true)?;
        Ok(balance.balance)
    }
}

pub fn to_readable(amount: U256, token: ERC20Token) -> String {
    let divisor_str = format!("1{:0>width$}", "", width = token.decimals as usize);
    let divisor = BigDecimal::from_str(&divisor_str).unwrap();
    let amount_as_decimal = BigDecimal::from_str(&amount.to_string()).unwrap();
    let amount = amount_as_decimal / divisor;
    format!("{:.4} {}", amount, token.symbol)
}
