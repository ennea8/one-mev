use std::{env, str::FromStr};

use alloy::primitives::Address;
use alloy_signer_local::PrivateKeySigner;

use anyhow::{anyhow, ensure, Error, Result};
use eth_keystore::decrypt_key;
use std::path::{Path, PathBuf};

use std::fs::read;
use std::fs::File;
use std::io::{self, Write};

use one_key::get_password;

pub fn get_keystore_path(suffix: &str) -> String {
    let prefix = env::var("KEYSTORE_PATH").expect("KEYSTORE_PATH not set");
    let mut path = PathBuf::from(prefix);
    path.push(suffix);

    // Convert PathBuf to &str, ensuring the result is valid UTF-8
    let full_path = path.to_str().expect("Failed to convert path to string");

    let full_path = full_path.replace("\\", "/").to_string();

    full_path
}

// get the private key from keystore file which is gererated by geth
pub fn get_key_from_keystore(path_str: &str, password: &str) -> Result<String, Error> {
    let keystore_path = Path::new(path_str);
    let secret_key = decrypt_key(keystore_path, password)?;
    let secret_key_hex = hex::encode(secret_key);
    Ok(secret_key_hex)
}

/// Encrypts or decrypts data using the XOR cipher.
pub fn xor_encrypt_decrypt(data: &str, key: &str) -> Vec<u8> {
    data.bytes()
        .zip(key.bytes().cycle())
        .map(|(d, k)| d ^ k)
        .collect()
}

pub fn write_encrypted_to_file(data: &str, key: &str, file_path: &str) -> io::Result<()> {
    let encrypted_data = xor_encrypt_decrypt(data, key);
    let mut file = File::create(file_path)?;
    file.write_all(&encrypted_data)?;
    Ok(())
}

pub fn read_and_decrypt_from_file(key: &str, file_path: &str) -> io::Result<String> {
    let encrypted_data = read(file_path)?;
    let decrypted_data = xor_encrypt_decrypt(&String::from_utf8_lossy(&encrypted_data), key);
    String::from_utf8(decrypted_data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// encript the private key to file with password + xor algorithm
// @param signer_name: str
// encrypt_privte_key_to_file('searcher_signer') will genarate searcher_signer_data file
pub fn encrypt_privte_key_to_file(signer_name: &str) {
    let password = get_password();
    let searcher_signer =
        get_key_from_keystore(get_keystore_path(signer_name).as_str(), password).unwrap();
    let searcher_signer_data = get_keystore_path((signer_name.to_owned() + "_data").as_str());
    write_encrypted_to_file(&searcher_signer, password, searcher_signer_data.as_str()).unwrap();
}

// decript the private key from file with password + xor algorithm
pub fn de_encrypt_privte_key_from_file(signer_name: &str) -> String {
    let password = get_password();
    let searcher_signer_data = get_keystore_path((signer_name.to_owned() + "_data").as_str());
    let decrypted_searcher_signer =
        read_and_decrypt_from_file(password, searcher_signer_data.as_str()).unwrap();
    decrypted_searcher_signer
}

mod tests {
    use super::*;

    #[test]
    fn test_xor_encrypt_decrypt_cipher() {
        let data = "hello world";
        let key = "key";
        let encrypted = xor_encrypt_decrypt(data, key);
        println!("encrypted: {:?}", std::str::from_utf8(&encrypted));

        let decrypted = xor_encrypt_decrypt(&String::from_utf8(encrypted).unwrap(), key);

        println!("decrypted: {:?}", std::str::from_utf8(&decrypted));

        assert_eq!(data, &String::from_utf8(decrypted).unwrap());
    }

    #[test]
    fn test_encrypt_privte_key_to_file() {
        let name = "searcher_signer";  // change name for different geth keystore file
        std::env::set_var("KEYSTORE_PATH", "../../.keystore");
        encrypt_privte_key_to_file(name); 
    }

    #[test]
    fn test_de_encrypt_privte_key_from_file() {
        std::env::set_var("KEYSTORE_PATH", "../../.keystore");

        let decrypted_searcher_signer = de_encrypt_privte_key_from_file("searcher_signer");
        println!("decrypted_searcher_signer: {}", decrypted_searcher_signer);

        let decrypted_searcher_signer = de_encrypt_privte_key_from_file("bundle_signer");
        println!("decrypted_bundle_signer: {}", decrypted_searcher_signer);
    }
    #[test]
    fn test_decrypted_bundle_signer_from_file() {
        std::env::set_var("KEYSTORE_PATH", "../../.keystore");
        let decrypted_searcher_signer = de_encrypt_privte_key_from_file("bundle_signer");
        println!("decrypted_bundle_signer: {}", decrypted_searcher_signer);
    }
}
