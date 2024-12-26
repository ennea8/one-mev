use eth_keystore::decrypt_key;
use std::path::Path;
use anyhow::{anyhow, ensure, Error, Result};


pub fn get_key_from_keystore(
    path_str: &str,
    password: &str,
) -> Result<String, Error> {
    let keystore_path = Path::new(path_str);
    let secret_key = decrypt_key(keystore_path, password)?;
    let secret_key_hex = hex::encode(secret_key);
    Ok(secret_key_hex)
}

// mod tests {
//     use super::*;

//     #[test]
//     fn test_get_key_from_keystore() {
//         let path = "../../.keystore/flashbot-182a2ee3d552df095febc5f1e2bc56411bf7ba50";
//         let key = get_key_from_keystore(path, "------").unwrap();
//         println!("key: {}", key);
//     }
// }
