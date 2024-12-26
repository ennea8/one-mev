pub mod tools;

use reqwest::Url;
use std::{env, str::FromStr};

use alloy::primitives::Address;
use alloy_signer_local::PrivateKeySigner;

use anyhow::{anyhow, ensure, Error, Result};
use eth_keystore::decrypt_key;
use std::path::{Path, PathBuf};

use one_key::get_password;

use tools::{get_keystore_path, get_key_from_keystore, de_encrypt_privte_key_from_file};



#[derive(Debug, Clone)]
pub struct Config {
    pub searcher_signer: PrivateKeySigner,
    pub bundle_signer: PrivateKeySigner,
    pub bot_address: Address,

    pub wss_rpc: Url,
    pub http_rpc: Url,
}



impl Config {
    pub fn get_config() -> Result<Self> {
        let password = get_password();

        let searcher_signer =de_encrypt_privte_key_from_file("searcher_signer");
        let bundle_signer =de_encrypt_privte_key_from_file("bundle_signer");

        let bot_address = env::var("BOT_ADDRESS")?;
        let wss_rpc = env::var("WSS_RPC")?;
        let http_rpc = env::var("HTTP_RPC")?;

        let searcher_signer = PrivateKeySigner::from_str(&searcher_signer)?;
        let bundle_signer = PrivateKeySigner::from_str(&bundle_signer)?;
        let bot_address = Address::from_str(&bot_address)?;
        let wss_rpc = Url::parse(&wss_rpc)?;
        let http_rpc = Url::parse(&http_rpc)?;

        Ok(Config {
            searcher_signer,
            bundle_signer,
            bot_address,

            wss_rpc,
            http_rpc,
        })
    }
}

mod tests {
    use super::*;

    #[test]
    fn test_get_config() {
        std::env::set_var("KEYSTORE_PATH", "../../.keystore");

        let config = Config::get_config().unwrap();
        println!("config: {:?}", config);
    }

    #[test]
    fn  test_get_keystore_path() {
        std::env::set_var("KEYSTORE_PATH", ".keystore");

        let path = get_keystore_path("searcher_signer");
        println!("path: {}", path);
        assert_eq!(path, ".keystore/searcher_signer");
    }
}
