use alloy::primitives::{address, Address};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref ZERO_ADDRESS: Address = address!("0000000000000000000000000000000000000000");
}

// simulator bot address
lazy_static! {
    pub static ref OWNER_ADDRESS: Address = address!("29b2F9e909451a6A98Fee9215Ac3648b69598800");
}

lazy_static! {
    pub static ref REVM_ONE_SIMULATOR_ADDRESS: Address = address!("29b2F9e909451a6A98Fee9215Ac3648b69590000");
}

lazy_static! {
    // same as address on chain
    pub static ref REVM_ONE_ADDRESS: Address = address!("1255B93eC243828A77e20614576E216b5151Ce1B");
}


pub mod ethereum {
    use lazy_static::lazy_static;
    use revm::primitives::{address, Address};

    pub fn weth_addr() -> Address {
        address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2")
    }

    pub fn usdc_addr() -> Address {
        address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
    }

    pub fn usdt_addr() -> Address {
        address!("dAC17F958D2ee523a2206206994597C13D831ec7")
    }

    pub fn dai_addr() -> Address {
        address!("6B175474E89094C44Da98b954EedeAC495271d0F")
    }

    pub fn wbtc_addr() -> Address {
        address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599")
    }

    pub fn uniswap_v2_factory() -> Address {
        address!("5c69bee701ef814a2b6a3edd4b1652cb9cc5aa6f")
    }

    pub fn uniswap_v3_quoter_v2() -> Address {
        address!("61fFE014bA17989E743c5F6cB21bF9697530B21e")
    }

    pub fn uniswap_v3_core_factory() -> Address {
        address!("1F98431c8aD98523631AE4a59f267346ea31F984")
    }

    // define a ignore token list / blacklist
    lazy_static! {
        pub static ref IGNORE_TOKEN_LIST: Vec<Address> = vec![
            //ignore list
            usdc_addr(), usdt_addr(), dai_addr(), //wbtc_addr(),
            // blacklist
        ];
    }
}
