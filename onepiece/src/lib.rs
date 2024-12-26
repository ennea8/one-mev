#[macro_use]
extern crate tracing;

#[macro_export]
macro_rules! method_alias {
    ($alias_name:ident, $original_name:ident) => {
        pub fn $alias_name(&mut self, params: IOne::SwapParams) -> Result<U256> {
            self.$original_name(params)
        }
    };
}

pub mod common;

pub mod simulation;

pub mod abi;

pub mod inspector;

pub mod utils;

pub mod arbitrage;

// pub mod sandwich;

