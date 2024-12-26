use std::fs::read;
use std::fs::File;
use std::io::{self, Write};

use one_config::tools::xor_encrypt_decrypt;

// read private key from an xor file
// xor_file_path, xor_key
// cargo run -p one-config --bin read_xor_file {xor_file_path} {xor_key}
fn main() {
    let xor_file_path = std::env::args().nth(1).expect("no xor_file_path given");
    let xor_key = std::env::args().nth(2).expect("no xor_key given");

    let encrypted_data = read(xor_file_path.as_str()).unwrap();

    let decrypted_data = xor_encrypt_decrypt(&String::from_utf8_lossy(&encrypted_data), xor_key.as_str());
    let decrypted_data = String::from_utf8(decrypted_data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)).unwrap();

    println!("decrypted_data: {:?}", decrypted_data);
}
