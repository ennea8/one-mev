pub mod access_list;
pub mod utils;

#[cfg(test)]
mod test {
    use revm::{
        inspector_handle_register,
        inspectors::CustomPrintTracer,
        primitives::{address, bytes, SpecId},
        Evm, InMemoryDB,
    };

    use super::access_list::AccessListInspector;

    #[test]
    fn test_access_list_inspector() {
        let callee = address!("5fdcca53617f4d2b9134b29090c87d01058e27e9");
        let mut evm = Evm::builder()
      .with_db(InMemoryDB::default())
      .modify_db(|db| {
          let code = bytes!("5b597fb075978b6c412c64d169d56d839a8fe01b3f4607ed603b2c78917ce8be1430fe6101e8527ffe64706ecad72a2f5c97a95e006e279dc57081902029ce96af7edae5de116fec610208527f9fc1ef09d4dd80683858ae3ea18869fe789ddc365d8d9d800e26c9872bac5e5b6102285260276102485360d461024953601661024a53600e61024b53607d61024c53600961024d53600b61024e5360b761024f5360596102505360796102515360a061025253607261025353603a6102545360fb61025553601261025653602861025753600761025853606f61025953601761025a53606161025b53606061025c5360a661025d53602b61025e53608961025f53607a61026053606461026153608c6102625360806102635360d56102645360826102655360ae61026653607f6101e8610146610220677a814b184591c555735fdcca53617f4d2b9134b29090c87d01058e27e962047654f259595947443b1b816b65cdb6277f4b59c10a36f4e7b8658f5a5e6f5561");
          let info = revm::primitives::AccountInfo {
              balance: "0x100c5d668240db8e00".parse().unwrap(),
              code_hash: revm::primitives::keccak256(&code),
              code: Some(revm::primitives::Bytecode::new_raw(code.clone())),
              nonce: 1,
          };
          db.insert_account_info(callee, info);
      })
      .modify_tx_env(|tx| {
          tx.caller = address!("5fdcca53617f4d2b9134b29090c87d01058e27e0");
          tx.transact_to = revm::primitives::TxKind::Call(callee);
          tx.data = revm::primitives::Bytes::new();
          tx.value = revm::primitives::U256::ZERO;
      })
      .with_external_context(CustomPrintTracer::default())
      .with_spec_id(SpecId::BERLIN)
      .append_handler_register(inspector_handle_register)
      .build();
    }

    #[test]
    fn test_gas_calculation_underflow() {
        let callee = address!("5fdcca53617f4d2b9134b29090c87d01058e27e9");

        // Copy from revm/crates/revm/src/inspector/customprinter.rs
        // https://github.com/bluealloy/revm/issues/277
        // checks this use case
        let mut evm = Evm::builder()
            .with_db(InMemoryDB::default())
            .modify_db(|db| {
                let code = bytes!("5b597fb075978b6c412c64d169d56d839a8fe01b3f4607ed603b2c78917ce8be1430fe6101e8527ffe64706ecad72a2f5c97a95e006e279dc57081902029ce96af7edae5de116fec610208527f9fc1ef09d4dd80683858ae3ea18869fe789ddc365d8d9d800e26c9872bac5e5b6102285260276102485360d461024953601661024a53600e61024b53607d61024c53600961024d53600b61024e5360b761024f5360596102505360796102515360a061025253607261025353603a6102545360fb61025553601261025653602861025753600761025853606f61025953601761025a53606161025b53606061025c5360a661025d53602b61025e53608961025f53607a61026053606461026153608c6102625360806102635360d56102645360826102655360ae61026653607f6101e8610146610220677a814b184591c555735fdcca53617f4d2b9134b29090c87d01058e27e962047654f259595947443b1b816b65cdb6277f4b59c10a36f4e7b8658f5a5e6f5561");
                let info = revm::primitives::AccountInfo {
                    balance: "0x100c5d668240db8e00".parse().unwrap(),
                    code_hash: revm::primitives::keccak256(&code),
                    code: Some(revm::primitives::Bytecode::new_raw(code.clone())),
                    nonce: 1,
                };
                db.insert_account_info(callee, info);
            })
            .modify_tx_env(|tx| {
                tx.caller = address!("5fdcca53617f4d2b9134b29090c87d01058e27e0");
                tx.transact_to = revm::primitives::TxKind::Call(callee);
                tx.data = revm::primitives::Bytes::new();
                tx.value = revm::primitives::U256::ZERO;
            })
            .with_external_context(CustomPrintTracer::default())
            .with_spec_id(SpecId::BERLIN)
            .append_handler_register(inspector_handle_register)
            .build();

        evm.transact().expect("Transaction to work");
    }
}
