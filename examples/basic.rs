extern crate faster_rs;

use faster_rs::{status, FasterKv};
use std::convert::TryInto;
fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

fn main() {
    // Create a Key-Value Store
    let store = FasterKv::default();
    let key0: u64 = 1;
    let value0: u64 = 1000;

    // Upsert
    for i in 0..1000 {
        let key = enc_u64(key0 + i);
        let value = enc_u64(value0 + i);
        let upsert = store.upsert(&key, &value, i);
        assert!(upsert == status::OK || upsert == status::PENDING);
    }

    assert!(store.size() > 0);

    // Read
    for i in 0..1000 {
        let key = enc_u64(key0 + i);
        let (read, recv) = store.read(&key, i);
        assert!(read == status::OK || read == status::PENDING);
        let val = recv.recv().unwrap();
        let value = dec_u64(&val);
        assert_eq!(value, value0 + i);
        println!("Key: {}, Value: {}", key0 + i, value);
    }

    // Clear used storage
    match store.clean_storage() {
        Ok(()) => {}
        Err(_err) => panic!("Unable to clear FASTER directory"),
    }
}
