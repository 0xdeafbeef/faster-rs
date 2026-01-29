extern crate faster_rs;
use faster_rs::{status, FasterKv};
fn main() {
    // Create a Key-Value Store
    let store = FasterKv::default();
    let key = b"hello-world".to_vec();
    let value = vec![1u8, 2, 3, 4];

    // Upsert
    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    assert!(store.size() > 0);

    let (read, recv) = store.read(&key, 1);
    assert!(read == status::OK || read == status::PENDING);
    let val = recv.recv().unwrap();
    println!("Key: {:?}, Value: {:?}", key, val);

    // Clear used storage
    match store.clean_storage() {
        Ok(()) => {}
        Err(_err) => panic!("Unable to clear FASTER directory"),
    }
}
