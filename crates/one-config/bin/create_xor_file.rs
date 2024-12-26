use std::fs::read;
use std::fs::File;
use std::io::{self, Write};

use one_config::tools::{get_key_from_keystore, xor_encrypt_decrypt};

// create an xor file from a keystore file
// keystore_path,keystore_password, xor_file_path， xor_key
// cargo run -p one-config --bin create_xor_file {keystore_path} {keystore_password} {xor_file_path} {xor_key}
fn main() {
    let keystore_path = std::env::args().nth(1).expect("no keystore_path given");
    let keystore_password = std::env::args().nth(2).expect("no keystore_password given");

    let xor_file_path = std::env::args().nth(3).expect("no xor_file_path given");
    let xor_key = std::env::args().nth(4).expect("no xor_key given");

    let signer = get_key_from_keystore(keystore_path.as_str(), keystore_password.as_str()).unwrap();

    let encrypted_data = xor_encrypt_decrypt(signer.as_str(), xor_key.as_str());
    let mut file = File::create(xor_file_path.as_str()).unwrap();
    file.write_all(&encrypted_data).unwrap();

    print!("done");

}
