
use crate::common::constants::*;
use alloy::primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;

// TODO check logic for bsc chain
pub fn calculate_next_block_base_fee(gas_used: U256, gas_limit: U256, base_fee_per_gas: U256) -> U256 {
    let gas_used = gas_used;

    let mut target_gas_used = gas_limit >> 1;
    target_gas_used = if target_gas_used == U256::ZERO { U256::from(1u64) } else { target_gas_used };

    let new_base_fee = {
        if gas_used > target_gas_used {
            base_fee_per_gas + ((base_fee_per_gas * (gas_used - target_gas_used)) / target_gas_used) / U256::from(8u64)
        } else {
            base_fee_per_gas - ((base_fee_per_gas * (target_gas_used - gas_used)) / target_gas_used) / U256::from(8u64)
        }
    };

    //let seed = rand::thread_rng().gen_range(0..9);
    new_base_fee //+ U256::from(seed)
}

pub fn calculate_next_block_base_fee2(gas_used: U256, gas_limit: U256, base_fee_per_gas: U256) -> U256 {
    // Get the block base fee per gas
    let current_base_fee_per_gas = base_fee_per_gas;

    let current_gas_used = gas_used;

    let current_gas_target = gas_limit / U256::from(2);

    if current_gas_used == current_gas_target {
        current_base_fee_per_gas
    } else if current_gas_used > current_gas_target {
        let gas_used_delta = current_gas_used - current_gas_target;
        let base_fee_per_gas_delta = current_base_fee_per_gas * gas_used_delta / current_gas_target / U256::from(8);

        return current_base_fee_per_gas + base_fee_per_gas_delta;
    } else {
        let gas_used_delta = current_gas_target - current_gas_used;
        let base_fee_per_gas_delta = current_base_fee_per_gas * gas_used_delta / current_gas_target / U256::from(8);

        return current_base_fee_per_gas - base_fee_per_gas_delta;
    }
}

pub fn create_new_wallet() -> (PrivateKeySigner, Address) {
    let wallet = PrivateKeySigner::random();
    let address = wallet.address();
    (wallet, address)
}

pub fn to_address(str_address: &'static str) -> Address {
    str_address.parse::<Address>().unwrap()
}

// pub fn is_weth(token_address: Address) -> bool {
//     token_address == to_address(WETH)
// }

pub fn is_nweth(token_address: Address) -> bool {
    token_address == to_address(NWETH)
}

pub fn is_main_currency(token_address: Address) -> bool {
    let main_currencies = vec![to_address(NWETH), to_address(USDT), to_address(USDC)];
    main_currencies.contains(&token_address)
}

#[derive(Debug, Clone)]
pub enum MainCurrency {
    NWETH,
    // WETH,
    USDT,
    USDC,

    Default, // Pairs that aren't WETH/Stable pairs. Default to WETH for now
}

impl MainCurrency {
    pub fn new(address: Address) -> Self {
        if address == to_address(NWETH) {
            MainCurrency::NWETH
        }
        // else if address == to_address(WETH) {
        //     MainCurrency::WETH
        // }
        else if address == to_address(USDT) {
            MainCurrency::USDT
        } else if address == to_address(USDC) {
            MainCurrency::USDC
        } else {
            MainCurrency::Default
        }
    }

    pub fn decimals(&self) -> u8 {
        match self {
            MainCurrency::NWETH => NWETH_DECIMALS,
            // MainCurrency::WETH => WETH_DECIMALS,
            MainCurrency::USDT => USDC_DECIMALS,
            MainCurrency::USDC => USDC_DECIMALS,
            MainCurrency::Default => NWETH_DECIMALS,
        }
    }

    pub fn balance_slot(&self) -> i32 {
        match self {
            MainCurrency::NWETH => NWETH_BALANCE_SLOT,
            // MainCurrency::WETH => WETH_BALANCE_SLOT,
            MainCurrency::USDT => USDT_BALANCE_SLOT,
            MainCurrency::USDC => USDC_BALANCE_SLOT,
            MainCurrency::Default => NWETH_BALANCE_SLOT,
        }
    }

    /*
    We score the currencies by importance
    WETH has the highest importance, and USDT, USDC in the following order
    */
    pub fn weight(&self) -> u8 {
        match self {
            MainCurrency::NWETH => 4,
            // MainCurrency::WETH => 3,
            MainCurrency::USDT => 2,
            MainCurrency::USDC => 1,
            MainCurrency::Default => 4, // default is NWETH
        }
    }
}

pub fn return_main_and_target_currency(token0: Address, token1: Address) -> Option<(Address, Address)> {
    let token0_supported = is_main_currency(token0);
    let token1_supported = is_main_currency(token1);

    if !token0_supported && !token1_supported {
        return None;
    }

    if token0_supported && token1_supported {
        let mc0 = MainCurrency::new(token0);
        let mc1 = MainCurrency::new(token1);

        let token0_weight = mc0.weight();
        let token1_weight = mc1.weight();

        if token0_weight > token1_weight {
            return Some((token0, token1));
        } else {
            return Some((token1, token0));
        }
    }

    if token0_supported {
        return Some((token0, token1));
    } else {
        return Some((token1, token0));
    }
}

pub fn u256_to_f64(value: U256) -> f64 {
    let bytes: [u8; 32] = value.to_be_bytes();
    let upper = u128::from_be_bytes(bytes[..16].try_into().unwrap());
    let lower = u128::from_be_bytes(bytes[16..].try_into().unwrap());

    (upper as f64) * 2f64.powi(128) + (lower as f64)
}
